//! Authenticated dispatch for the future Harbor control-plane listener.
//!
//! This module accepts only signed control envelopes. It is intentionally
//! transport-independent so TLS, connection limits, and persistence can be
//! reviewed before a listener is exposed on the K11+.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::Path;
use std::sync::{Arc, Mutex};

use harbor_control::{ControlError, ControlPlane, IdentityRecord, Presence};
use harbor_protocol::{AuthenticatedEnvelope, Envelope, ProtocolError};
use serde::Serialize;
use serde_json::{Value, json};
use thiserror::Error;
use uuid::Uuid;

mod relay;
mod store;
mod stun;
mod transport;
mod turn;

pub use turn::{
    MAX_CHANNELS_PER_ALLOCATION, MAX_NONCES, MAX_PERMISSIONS_PER_ALLOCATION, MAX_TURN_ALLOCATIONS,
    TURN_CHANNEL_LIFETIME_SECS, TURN_CRED_TTL_SECS, TURN_DEFAULT_LIFETIME_SECS, TURN_FIRST_CHANNEL,
    TURN_LAST_CHANNEL, TURN_MAX_LIFETIME_SECS, TURN_MIN_LIFETIME_SECS, TURN_NONCE_TTL_SECS,
    TURN_PERMISSION_LIFETIME_SECS, TURN_REALM, TURN_RELAY_BUDGET_BYTES_PER_SEC, TurnOutcome,
    TurnRelayConfig, TurnState,
};

pub use relay::{
    DeliveredFrame, MAX_PENDING_PER_SESSION, MAX_RELAY_DATA_CHARS, MAX_RELAY_SESSIONS,
    POLL_MAX_BYTES, RELAY_ACCEPT_WINDOW_SECS, RELAY_IDLE_TIMEOUT_SECS, RELAY_SESSION_TTL_SECS,
    RelayDelivery, RelayFault, RelayFrame, RelayOpenNotice, RelayPurpose, RelayTable,
};
pub use store::{StateStore, StoreError};
pub use stun::{StunError, StunServer};
pub use transport::{Listener, ListenerConfig, MAX_NETWORK_FRAME_BYTES, TransportError};

/// Message types whose success mutates durable server state. Everything else
/// (presence leases, sessions, listing) stays in memory by design.
const DURABLE_MESSAGE_TYPES: &[&str] = &[
    "identity.update",
    "pairing.create",
    "pairing.submit",
    "pairing.accept",
    "pairing.decline",
    "pairing.cancel",
];

#[derive(Debug, Error)]
pub enum DispatchError {
    #[error("request cannot be correlated")]
    Correlation,
}

#[derive(Debug, Error)]
pub enum HandleError {
    #[error(transparent)]
    Dispatch(#[from] DispatchError),
    #[error("durable state write failed: {0}")]
    Persistence(#[from] StoreError),
}

/// Bundles the authoritative control plane with its durable state so a
/// restarted listener resumes with the identities and relationships it had.
pub struct ServerCore {
    control: ControlPlane,
    store: StateStore,
    /// Transport endpoints observed per device, keyed by signer id. Transient
    /// rendezvous fact, deliberately outside the control plane (which stays
    /// transport-independent) and outside the durable snapshot: a restart
    /// clears it and devices re-report by simply talking to the server again.
    observed: HashMap<Uuid, ObservedEndpoint>,
    /// TURN relay state shared with the UDP serve loop: the control plane
    /// mints short-lived Allocate credentials here (`turn.credentials`) and
    /// the datagram loop authenticates against the same store. Wrapped so
    /// both sides lock narrowly without taking the whole core. Transient by
    /// design (restart drops allocations; clients re-Allocate on 401/438).
    turn: Arc<Mutex<TurnState>>,
}

/// Transport endpoint the server observed for a device: the source address
/// of its control-plane connection, as seen at accept time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ObservedEndpoint {
    pub address: String,
    pub port: u16,
    pub transport: String,
    pub observed_at: u64,
}

impl ObservedEndpoint {
    fn tcp(addr: SocketAddr, now: u64) -> Self {
        Self {
            address: addr.ip().to_string(),
            port: addr.port(),
            transport: "tcp".to_owned(),
            observed_at: now,
        }
    }
}

impl ServerCore {
    pub fn open(directory: &Path) -> Result<Self, StoreError> {
        let store = StateStore::open(directory)?;
        let control = match store.load()? {
            Some(snapshot) => ControlPlane::restore(snapshot)?,
            None => ControlPlane::default(),
        };
        Ok(Self {
            control,
            store,
            observed: HashMap::new(),
            turn: Arc::new(Mutex::new(TurnState::new())),
        })
    }

    /// Shared TURN state for the UDP serve loop. Clone the [`Arc`] once at
    /// startup; every datagram locks only this, never the whole core.
    pub fn turn(&self) -> Arc<Mutex<TurnState>> {
        Arc::clone(&self.turn)
    }

    /// Handles one authenticated network request. Requests whose timestamp is
    /// outside the server clock tolerance are refused before any state
    /// transition, which bounds how long a captured request stays replayable.
    ///
    /// Embedded and test callers use this when no transport endpoint is
    /// known; the listener path uses [`ServerCore::handle_observed`] with the
    /// connection's source address instead.
    pub fn handle(
        &mut self,
        authenticated: AuthenticatedEnvelope,
        now: u64,
    ) -> Result<Envelope, HandleError> {
        self.handle_observed(authenticated, now, None)
    }

    /// Primary transport path: `observed` is the connection's source endpoint
    /// as seen at accept time. Recording happens only for requests that
    /// authenticate and dispatch without error, so strangers cannot pollute
    /// the rendezvous map for arbitrary device ids.
    pub fn handle_observed(
        &mut self,
        authenticated: AuthenticatedEnvelope,
        now: u64,
        observed: Option<SocketAddr>,
    ) -> Result<Envelope, HandleError> {
        let durable = DURABLE_MESSAGE_TYPES.contains(&authenticated.envelope.message_type.as_str());
        let is_endpoint_self = authenticated.envelope.message_type == "endpoint.self";
        let is_turn_credentials = authenticated.envelope.message_type == "turn.credentials";
        let signer = authenticated.signer_id;
        let response = if timestamp_within_skew(authenticated.envelope.timestamp.as_deref(), now) {
            if is_endpoint_self {
                self.serve_endpoint_self(authenticated, now, observed)?
            } else if is_turn_credentials {
                self.serve_turn_credentials(authenticated, now)?
            } else {
                dispatch(&mut self.control, authenticated, now)?
            }
        } else {
            let mut response = response_envelope(&authenticated.envelope)?;
            response.error = Some(stale_timestamp_error());
            response
        };
        if durable && response.error.is_none() {
            self.flush()?;
        }
        if !is_endpoint_self && response.error.is_none() {
            if let Some(addr) = observed {
                self.observed
                    .insert(signer, ObservedEndpoint::tcp(addr, now));
            }
        }
        Ok(response)
    }

    /// Serves `endpoint.self`: the transport endpoint observed for this very
    /// connection. Recording happens first so the served value is always the
    /// stored value; a later peer-rendezvous phase reads the same map.
    /// Unregistered or unverified callers, and callers without a transport
    /// endpoint, learn nothing.
    fn serve_endpoint_self(
        &mut self,
        authenticated: AuthenticatedEnvelope,
        now: u64,
        observed: Option<SocketAddr>,
    ) -> Result<Envelope, DispatchError> {
        let mut response = response_envelope(&authenticated.envelope)?;
        let Some(addr) = observed else {
            response.error = Some(ProtocolError {
                code: "endpoint_unknown".into(),
                ui_key: "error.server.endpointUnknown".into(),
                retryable: false,
                detail: "No transport endpoint is known for this connection".into(),
            });
            return Ok(response);
        };
        if verified_identity(&self.control, &authenticated).is_none() {
            response.error = Some(unauthorized_error());
            return Ok(response);
        }
        self.observed
            .insert(authenticated.signer_id, ObservedEndpoint::tcp(addr, now));
        response.payload = json!(
            self.observed
                .get(&authenticated.signer_id)
                .expect("endpoint just recorded")
        );
        Ok(response)
    }

    /// Serves `turn.credentials`: mints (or rotates) the short-lived
    /// username/password the caller's media worker uses in TURN Allocate.
    /// Verified identities only — the password is returned once, over the
    /// pinned control channel, and never logged. Transient: no durable
    /// write, no pairing requirement (any registered device may need a
    /// relayed call); abuse is bounded by credential TTL, nonce TTL, and
    /// the per-server allocation/permission/channel quotas in [`TurnState`].
    fn serve_turn_credentials(
        &self,
        authenticated: AuthenticatedEnvelope,
        now: u64,
    ) -> Result<Envelope, DispatchError> {
        let mut response = response_envelope(&authenticated.envelope)?;
        if verified_identity(&self.control, &authenticated).is_none() {
            response.error = Some(unauthorized_error());
            return Ok(response);
        }
        let mut turn = self
            .turn
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let (username, password, expires_at) = turn.mint_credential(authenticated.signer_id, now);
        response.payload = json!({
            "username": username,
            "password": password,
            "realm": TURN_REALM,
            "ttl_secs": TURN_CRED_TTL_SECS,
            "expires_at": expires_at,
        });
        Ok(response)
    }

    pub fn flush(&mut self) -> Result<(), StoreError> {
        self.store.store(&self.control.snapshot())
    }

    /// Read-only lookup used by transports and health checks; it never
    /// transitions state.
    pub fn identity(&self, device_id: Uuid) -> Option<IdentityRecord> {
        self.control.identity(device_id).cloned()
    }

    /// Relay fallback entry point, mirroring [`ServerCore::handle`] for the
    /// `relay.*` family: clock-skew gate first, then verified dispatch into
    /// the caller's [`RelayTable`]. The table lives outside the control-plane
    /// mutex (transport threads drain it without touching control state), so
    /// it travels as a parameter instead of a field. `relay.poll` parks
    /// instead of answering: a verified poll returns `Park` with its request
    /// envelope for the connection loop to answer later.
    pub(crate) fn relay_request(
        &self,
        relay: &Mutex<RelayTable>,
        authenticated: AuthenticatedEnvelope,
        now: u64,
    ) -> Result<RelayAction, DispatchError> {
        if !timestamp_within_skew(authenticated.envelope.timestamp.as_deref(), now) {
            let mut response = response_envelope(&authenticated.envelope)?;
            response.error = Some(stale_timestamp_error());
            return Ok(RelayAction::Reply(response));
        }
        let mut response = response_envelope(&authenticated.envelope)?;
        let action = match authenticated.envelope.message_type.as_str() {
            "relay.open" => {
                let (peer, purpose) = match parse_relay_open(&authenticated.envelope.payload) {
                    Some(pair) => pair,
                    None => {
                        response.error = Some(invalid_request_error());
                        return Ok(RelayAction::Reply(response));
                    }
                };
                let Some(_) = verified_identity(&self.control, &authenticated) else {
                    response.error = Some(unauthorized_error());
                    return Ok(RelayAction::Reply(response));
                };
                if !RelayTable::check_paired(&self.control, authenticated.signer_id, peer) {
                    response.error = Some(unauthorized_error());
                    return Ok(RelayAction::Reply(response));
                }
                let mut table = relay.lock().unwrap_or_else(|poison| poison.into_inner());
                match table.open(authenticated.signer_id, peer, purpose, now) {
                    Ok(id) => response.payload = json!({"relay_id": id}),
                    Err(fault) => response.error = Some(relay_fault_error(fault)),
                }
                RelayAction::Reply(response)
            }
            "relay.accept" => {
                let Some(id) = uuid_field(&authenticated.envelope.payload, "relay_id") else {
                    response.error = Some(invalid_request_error());
                    return Ok(RelayAction::Reply(response));
                };
                let Some(_) = verified_identity(&self.control, &authenticated) else {
                    response.error = Some(unauthorized_error());
                    return Ok(RelayAction::Reply(response));
                };
                let mut table = relay.lock().unwrap_or_else(|poison| poison.into_inner());
                match table.accept(id, authenticated.signer_id, now) {
                    Ok(()) => response.payload = json!({"relay_id": id, "accepted": true}),
                    Err(fault) => response.error = Some(relay_fault_error(fault)),
                }
                RelayAction::Reply(response)
            }
            "relay.data" => {
                let Some((id, seq, bytes)) = parse_relay_data(&authenticated.envelope.payload)
                else {
                    response.error = Some(invalid_request_error());
                    return Ok(RelayAction::Reply(response));
                };
                let Some(_) = verified_identity(&self.control, &authenticated) else {
                    response.error = Some(unauthorized_error());
                    return Ok(RelayAction::Reply(response));
                };
                let mut table = relay.lock().unwrap_or_else(|poison| poison.into_inner());
                match table.push(id, authenticated.signer_id, seq, bytes, now) {
                    Ok(()) => {
                        response.payload = json!({"relay_id": id, "seq": seq, "queued": true})
                    }
                    Err(fault) => response.error = Some(relay_fault_error(fault)),
                }
                RelayAction::Reply(response)
            }
            "relay.poll" => {
                let Some(_) = verified_identity(&self.control, &authenticated) else {
                    response.error = Some(unauthorized_error());
                    return Ok(RelayAction::Reply(response));
                };
                RelayAction::Park(authenticated.envelope)
            }
            "relay.close" => {
                let Some(id) = uuid_field(&authenticated.envelope.payload, "relay_id") else {
                    response.error = Some(invalid_request_error());
                    return Ok(RelayAction::Reply(response));
                };
                let Some(_) = verified_identity(&self.control, &authenticated) else {
                    response.error = Some(unauthorized_error());
                    return Ok(RelayAction::Reply(response));
                };
                let mut table = relay.lock().unwrap_or_else(|poison| poison.into_inner());
                match table.close(id, authenticated.signer_id, now) {
                    Ok(()) => response.payload = json!({"relay_id": id, "closed": true}),
                    Err(fault) => response.error = Some(relay_fault_error(fault)),
                }
                RelayAction::Reply(response)
            }
            "relay.error" => {
                response.error = Some(ProtocolError::invalid_request(
                    "relay.error is server-originated; clients must not send it",
                ));
                RelayAction::Reply(response)
            }
            _ => {
                response.error = Some(ProtocolError {
                    code: "capability_unavailable".into(),
                    ui_key: "error.server.capabilityUnavailable".into(),
                    retryable: false,
                    detail: format!(
                        "{} is not enabled by the current server foundation",
                        authenticated.envelope.message_type
                    ),
                });
                RelayAction::Reply(response)
            }
        };
        Ok(action)
    }
}

/// Requests older or newer than this window are rejected as replay vectors.
const MAX_CLOCK_SKEW_SECONDS: u64 = 300;

fn timestamp_within_skew(timestamp: Option<&str>, now: u64) -> bool {
    timestamp
        .and_then(|value| {
            time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc3339).ok()
        })
        .is_some_and(|parsed| {
            let now_seconds = i64::try_from(now).unwrap_or(i64::MAX);
            parsed.unix_timestamp().abs_diff(now_seconds) <= MAX_CLOCK_SKEW_SECONDS
        })
}

fn stale_timestamp_error() -> ProtocolError {
    ProtocolError {
        code: "stale_timestamp".into(),
        ui_key: "error.protocol.staleTimestamp".into(),
        retryable: true,
        detail: "The request timestamp is outside the server clock tolerance".into(),
    }
}

fn response_envelope(request: &Envelope) -> Result<Envelope, DispatchError> {
    let timestamp = request
        .timestamp
        .clone()
        .ok_or(DispatchError::Correlation)?;
    Envelope::response_to(request, request.message_type.clone(), json!({}), timestamp)
        .map_err(|_| DispatchError::Correlation)
}

/// Registered identity whose public key verifies the envelope signature.
/// Every network request except first-time `identity.update` registration
/// funnels through here; `endpoint.self` reuses it so rendezvous facts never
/// leak to unverified callers.
fn verified_identity(
    control: &ControlPlane,
    authenticated: &AuthenticatedEnvelope,
) -> Option<IdentityRecord> {
    let identity = control.identity(authenticated.signer_id)?;
    authenticated.verify(&identity.public_key).ok()?;
    Some(identity.clone())
}

/// Outcome of one `relay.*` request: answer now, or park the poll request
/// for the connection loop to answer when traffic (or the hold deadline)
/// arrives.
pub(crate) enum RelayAction {
    Reply(Envelope),
    Park(Envelope),
}

/// `{peer: <uuid>, purpose: "chat"|"file"|"media"}` or `None` for anything
/// else. Unknown purposes are not defaulted: the client must name one.
fn parse_relay_open(payload: &Value) -> Option<(Uuid, RelayPurpose)> {
    let peer = uuid_field(payload, "peer")?;
    let purpose = payload
        .get("purpose")
        .and_then(Value::as_str)
        .and_then(RelayPurpose::parse)?;
    Some((peer, purpose))
}

/// `{relay_id: <uuid>, seq: <u64>, bytes: <string>}` or `None`. Size is
/// enforced by the table, not here; shape only.
fn parse_relay_data(payload: &Value) -> Option<(Uuid, u64, String)> {
    let id = uuid_field(payload, "relay_id")?;
    let seq = payload.get("seq")?.as_u64()?;
    let bytes = payload.get("bytes")?.as_str()?.to_owned();
    Some((id, seq, bytes))
}

fn relay_fault_error(fault: RelayFault) -> ProtocolError {
    ProtocolError {
        code: fault.code().into(),
        ui_key: fault.ui_key().into(),
        retryable: fault.retryable(),
        detail: "The relay request cannot be completed".into(),
    }
}

pub fn dispatch(
    control: &mut ControlPlane,
    authenticated: AuthenticatedEnvelope,
    now: u64,
) -> Result<Envelope, DispatchError> {
    let message_type = authenticated.envelope.message_type.clone();
    let mut response = response_envelope(&authenticated.envelope)?;

    if message_type == "identity.update" {
        return Ok(register_identity(control, authenticated, now, response));
    }

    if verified_identity(control, &authenticated).is_none() {
        response.error = Some(unauthorized_error());
        return Ok(response);
    }

    match message_type.as_str() {
        "pairing.create" => {
            let Some(code) = authenticated
                .envelope
                .payload
                .get("code")
                .and_then(Value::as_str)
            else {
                response.error = Some(invalid_request_error());
                return Ok(response);
            };
            set_control_response(
                &mut response,
                control.create_pairing(authenticated.signer_id, code.to_owned(), now),
            );
        }
        "pairing.submit" => {
            let Some(code) = authenticated
                .envelope
                .payload
                .get("code")
                .and_then(Value::as_str)
            else {
                response.error = Some(invalid_request_error());
                return Ok(response);
            };
            set_control_response(
                &mut response,
                control.submit_pairing(authenticated.signer_id, code, now),
            );
        }
        // Harbor-ID invitation: the caller names the peer by full Harbor ID
        // (`pairing.invite {peer: "harbor-xxxxxxxx"}`); format, resolution,
        // self-pairing, and duplicate checks all happen in the control plane.
        // The legacy code flow above stays untouched for older clients.
        "pairing.invite" => {
            let Some(peer) = authenticated
                .envelope
                .payload
                .get("peer")
                .and_then(Value::as_str)
            else {
                response.error = Some(invalid_request_error());
                return Ok(response);
            };
            set_control_response(
                &mut response,
                control.invite_pairing(authenticated.signer_id, peer, now),
            );
        }
        "pairing.incoming" => {
            let result = control.incoming_pairings(authenticated.signer_id, now);
            // Envelope payloads are objects by protocol rule; a bare
            // array would fail response validation and drop the reply.
            // Each snapshot carries who asks as a Harbor ID (never a bare
            // device UUID as a display name) alongside the raw snapshot the
            // legacy flow already consumes.
            let enriched = result.map(|requests| {
                let enriched: Vec<Value> = requests
                    .iter()
                    .map(|snapshot| {
                        let mut value = serde_json::to_value(snapshot)
                            .expect("pairing snapshot is serializable");
                        if let Some(requester) = snapshot.requester {
                            value["requester_harbor_id"] =
                                json!(control.harbor_id_of(requester).unwrap_or_default());
                        }
                        value
                    })
                    .collect();
                json!({ "requests": enriched })
            });
            set_control_response(&mut response, enriched);
        }
        "pairing.status" => set_id_action(
            &mut response,
            uuid_field(&authenticated.envelope.payload, "pairing_id"),
            |pairing_id| control.pairing_status(authenticated.signer_id, pairing_id, now),
        ),
        "pairing.accept" => set_id_action(
            &mut response,
            uuid_field(&authenticated.envelope.payload, "pairing_id"),
            |pairing_id| control.accept_pairing(authenticated.signer_id, pairing_id, now),
        ),
        "pairing.decline" => set_id_action(
            &mut response,
            uuid_field(&authenticated.envelope.payload, "pairing_id"),
            |pairing_id| control.decline_pairing(authenticated.signer_id, pairing_id, now),
        ),
        "pairing.cancel" => set_id_action(
            &mut response,
            uuid_field(&authenticated.envelope.payload, "pairing_id"),
            |pairing_id| control.cancel_pairing(authenticated.signer_id, pairing_id, now),
        ),
        "presence.publish" => {
            let Some(state) = authenticated
                .envelope
                .payload
                .get("state")
                .and_then(|value| serde_json::from_value::<Presence>(value.clone()).ok())
            else {
                response.error = Some(invalid_request_error());
                return Ok(response);
            };
            set_control_response(
                &mut response,
                control
                    .publish_presence(authenticated.signer_id, state, now)
                    .map(|_| json!({ "state": state })),
            );
        }
        // The paired read of a peer's presence lease. The same pairing and
        // expiry rules apply as everywhere else: an expired or missing lease
        // reads as Offline, and the answer never reveals lease mechanics.
        "presence.status" => {
            let Some(peer) = uuid_field(&authenticated.envelope.payload, "peer") else {
                response.error = Some(invalid_request_error());
                return Ok(response);
            };
            set_control_response(
                &mut response,
                control
                    .presence_of(authenticated.signer_id, peer, now)
                    .map(|state| json!({ "state": state })),
            );
        }
        "session.connect" => set_id_action(
            &mut response,
            uuid_field(&authenticated.envelope.payload, "peer"),
            |peer| control.connect_session(authenticated.signer_id, peer, now),
        ),
        "session.disconnect" => set_id_action(
            &mut response,
            uuid_field(&authenticated.envelope.payload, "session_id"),
            |session_id| control.disconnect_session(authenticated.signer_id, session_id, now),
        ),
        "session.signal" => {
            let Some(session_id) = uuid_field(&authenticated.envelope.payload, "session_id") else {
                response.error = Some(invalid_request_error());
                return Ok(response);
            };
            let Some(signal) = authenticated
                .envelope
                .payload
                .get("signal")
                .and_then(Value::as_str)
            else {
                response.error = Some(invalid_request_error());
                return Ok(response);
            };
            // Relay only: the signal is validated, queued for the session's
            // other peer, and dropped on disconnect — never interpreted.
            set_control_response(
                &mut response,
                control
                    .queue_signal(authenticated.signer_id, session_id, signal, now)
                    .map(|_| json!({ "session_id": session_id, "queued": true })),
            );
        }
        "session.signal_poll" => {
            let Some(session_id) = uuid_field(&authenticated.envelope.payload, "session_id") else {
                response.error = Some(invalid_request_error());
                return Ok(response);
            };
            set_control_response(
                &mut response,
                control
                    .drain_signals(authenticated.signer_id, session_id)
                    .map(|signals| {
                        json!({
                            "session_id": session_id,
                            "signals": signals.iter().map(|entry| json!({
                                "from": entry.from_peer,
                                "signal": entry.signal,
                                "enqueued_at": entry.enqueued_at,
                            })).collect::<Vec<_>>()
                        })
                    }),
            );
        }
        "contacts.list" => {
            set_control_response(
                &mut response,
                control
                    .paired_peers(authenticated.signer_id)
                    .map(|peers| json!({ "peers": peers })),
            );
        }
        _ => {
            response.error = Some(ProtocolError {
                code: "capability_unavailable".into(),
                ui_key: "error.server.capabilityUnavailable".into(),
                retryable: false,
                detail: format!("{message_type} is not enabled by the current server foundation"),
            })
        }
    }
    Ok(response)
}

/// First registration bootstraps trust from the payload key. Existing devices
/// may update only with that same registered key; recovery and key rotation
/// require a separate future authenticated protocol.
fn register_identity(
    control: &mut ControlPlane,
    authenticated: AuthenticatedEnvelope,
    now: u64,
    mut response: Envelope,
) -> Envelope {
    let payload = &authenticated.envelope.payload;
    let device_id = payload
        .get("device_id")
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok());
    let harbor_id = payload.get("harbor_id").and_then(Value::as_str);
    let public_key = payload.get("public_key").and_then(Value::as_str);
    let (Some(device_id), Some(harbor_id), Some(public_key)) = (device_id, harbor_id, public_key)
    else {
        response.error = Some(invalid_request_error());
        return response;
    };
    if device_id != authenticated.signer_id {
        response.error = Some(unauthorized_error());
        return response;
    }
    let authorized = match control.identity(device_id) {
        Some(identity) => {
            identity.public_key == public_key && authenticated.verify(&identity.public_key).is_ok()
        }
        None => authenticated.verify(public_key).is_ok(),
    };
    if !authorized {
        response.error = Some(unauthorized_error());
        return response;
    }

    match control.register_identity(device_id, harbor_id.to_owned(), public_key.to_owned(), now) {
        Ok(identity) => {
            response.payload = json!({
                "device_id": identity.device_id,
                "harbor_id": identity.harbor_id,
                "public_key": identity.public_key,
                "registered_at": identity.registered_at,
            });
        }
        Err(error) => response.error = Some(control_error(error)),
    }
    response
}

fn unauthorized_error() -> ProtocolError {
    ProtocolError {
        code: "unauthorized".into(),
        ui_key: "error.server.unauthorized".into(),
        retryable: false,
        detail: "The signed identity is not authorized for this operation".into(),
    }
}

fn invalid_request_error() -> ProtocolError {
    ProtocolError::invalid_request("The request payload does not meet the control-plane schema")
}

fn uuid_field(payload: &Value, field: &str) -> Option<Uuid> {
    payload
        .get(field)
        .and_then(Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
}

fn set_id_action<T: Serialize>(
    response: &mut Envelope,
    id: Option<Uuid>,
    action: impl FnOnce(Uuid) -> Result<T, ControlError>,
) {
    let Some(id) = id else {
        response.error = Some(invalid_request_error());
        return;
    };
    set_control_response(response, action(id));
}

fn set_control_response<T: Serialize>(response: &mut Envelope, result: Result<T, ControlError>) {
    match result {
        Ok(value) => {
            response.payload = serde_json::to_value(value).expect("control state is serializable")
        }
        Err(error) => response.error = Some(control_error(error)),
    }
}

fn control_error(error: ControlError) -> ProtocolError {
    let (code, ui_key) = match error {
        ControlError::Unauthorized
        | ControlError::UnknownIdentity
        | ControlError::PeersNotPaired => ("unauthorized", "error.server.unauthorized"),
        ControlError::InvalidPairingCode
        | ControlError::DuplicatePairingCode
        | ControlError::InvalidHarborId
        | ControlError::HarborIdAlreadyRegistered
        | ControlError::HarborIdImmutable
        | ControlError::AlreadyPaired
        | ControlError::SelfPairing => ("pairing_invalid", "error.pairing.invalid"),
        ControlError::UnknownPairing | ControlError::InactivePairing => {
            ("pairing_unavailable", "error.pairing.unavailable")
        }
        ControlError::UnknownSession
        | ControlError::OversizedSignal
        | ControlError::SignalQueueFull
        | ControlError::InvalidIdentity => ("invalid_request", "error.protocol.invalidRequest"),
    };
    ProtocolError {
        code: code.into(),
        ui_key: ui_key.into(),
        retryable: false,
        detail: "The control-plane request cannot be completed".into(),
    }
}

#[cfg(test)]
mod tests {
    use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};
    use ed25519_dalek::SigningKey;
    use harbor_control::{ControlSnapshot, IdentityRecord, MAX_SIGNAL_BYTES, RestoreError};
    use harbor_protocol::AuthenticatedEnvelope;

    use super::*;

    fn signed_identity_update(key: &SigningKey, device_id: Uuid) -> AuthenticatedEnvelope {
        signed_request(
            key,
            device_id,
            "identity.update",
            json!({
                "device_id": device_id,
                "harbor_id": format!("harbor-{:08x}", device_id.as_u128() as u32),
                "public_key": STANDARD_NO_PAD.encode(key.verifying_key().as_bytes()),
            }),
            &rfc3339_now(0),
        )
    }

    fn signed_request(
        key: &SigningKey,
        signer: Uuid,
        message_type: &str,
        payload: Value,
        timestamp: &str,
    ) -> AuthenticatedEnvelope {
        AuthenticatedEnvelope::sign(
            signer,
            Envelope::request(message_type, payload, timestamp),
            key,
        )
        .unwrap()
    }

    /// Wall-clock helpers: `ServerCore::handle` enforces timestamp freshness,
    /// so tests driving it must timestamp requests "now".
    fn unix_now() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn rfc3339_now(offset_seconds: i64) -> String {
        time::OffsetDateTime::from_unix_timestamp(unix_now() as i64 + offset_seconds)
            .unwrap()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap()
    }

    fn store_snapshot(directory: &std::path::Path, snapshot: &ControlSnapshot) {
        StateStore::open(directory)
            .unwrap()
            .store(snapshot)
            .unwrap();
    }

    #[test]
    fn first_identity_registration_and_same_key_update_succeed() {
        let key = SigningKey::from_bytes(&[9; 32]);
        let mut control = ControlPlane::default();
        let device_id = Uuid::new_v4();
        let response = dispatch(&mut control, signed_identity_update(&key, device_id), 10).unwrap();
        assert!(response.error.is_none());
        assert_eq!(
            control.identity(device_id).unwrap().harbor_id,
            format!("harbor-{:08x}", device_id.as_u128() as u32)
        );

        let updated = dispatch(
            &mut control,
            signed_request(
                &key,
                device_id,
                "identity.update",
                json!({
                    "device_id": device_id,
                    "harbor_id": "harbor-aabbccdd",
                    "public_key": STANDARD_NO_PAD.encode(key.verifying_key().as_bytes()),
                }),
                &rfc3339_now(1),
            ),
            11,
        )
        .unwrap();
        assert_eq!(updated.error.unwrap().code, "pairing_invalid");
        assert_eq!(
            control.identity(device_id).unwrap().harbor_id,
            format!("harbor-{:08x}", device_id.as_u128() as u32)
        );
    }

    #[test]
    fn pairing_transitions_require_authenticated_registered_devices() {
        let first_key = SigningKey::from_bytes(&[1; 32]);
        let second_key = SigningKey::from_bytes(&[2; 32]);
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let mut control = ControlPlane::default();
        dispatch(&mut control, signed_identity_update(&first_key, first), 10).unwrap();
        dispatch(
            &mut control,
            signed_identity_update(&second_key, second),
            10,
        )
        .unwrap();

        let created = dispatch(
            &mut control,
            signed_request(
                &second_key,
                second,
                "pairing.create",
                json!({"code": "123456"}),
                "2026-08-31T20:00:01Z",
            ),
            11,
        )
        .unwrap();
        let pairing_id = created.payload["pairing_id"].as_str().unwrap().to_owned();

        assert!(
            dispatch(
                &mut control,
                signed_request(
                    &first_key,
                    first,
                    "pairing.submit",
                    json!({"code": "123456"}),
                    "2026-08-31T20:00:02Z",
                ),
                12,
            )
            .unwrap()
            .error
            .is_none()
        );

        let accepted = dispatch(
            &mut control,
            signed_request(
                &second_key,
                second,
                "pairing.accept",
                json!({"pairing_id": pairing_id}),
                "2026-08-31T20:00:03Z",
            ),
            13,
        )
        .unwrap();
        assert_eq!(accepted.payload["state"], "ACCEPTED");
    }

    fn registered_with_harbor_id(
        control: &mut ControlPlane,
        key: &SigningKey,
        device: Uuid,
        harbor_id: &str,
        now: u64,
    ) {
        use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};
        dispatch(
            control,
            signed_request(
                key,
                device,
                "identity.update",
                json!({
                    "device_id": device,
                    "harbor_id": harbor_id,
                    "public_key": STANDARD_NO_PAD.encode(key.verifying_key().as_bytes()),
                }),
                &rfc3339_now(0),
            ),
            now,
        )
        .unwrap();
    }

    #[test]
    fn invite_flow_uses_full_harbor_ids_end_to_end() {
        let first_key = SigningKey::from_bytes(&[31; 32]);
        let second_key = SigningKey::from_bytes(&[32; 32]);
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let mut control = ControlPlane::default();
        registered_with_harbor_id(&mut control, &first_key, first, "harbor-aaaaaaaa", 10);
        registered_with_harbor_id(&mut control, &second_key, second, "harbor-bbbbbbbb", 10);

        // Malformed IDs and unknown peers never create state.
        for (peer, code) in [
            ("123456", "pairing_invalid"),
            ("harbor-zzzzzzzz", "pairing_invalid"),
            ("harbor-cccccccc", "pairing_unavailable"),
        ] {
            let refused = dispatch(
                &mut control,
                signed_request(
                    &first_key,
                    first,
                    "pairing.invite",
                    json!({"peer": peer}),
                    &rfc3339_now(1),
                ),
                11,
            )
            .unwrap();
            assert_eq!(refused.error.unwrap().code, code, "peer {peer}");
        }
        let missing = dispatch(
            &mut control,
            signed_request(
                &first_key,
                first,
                "pairing.invite",
                json!({}),
                &rfc3339_now(1),
            ),
            11,
        )
        .unwrap();
        assert_eq!(missing.error.unwrap().code, "invalid_request");

        // Invite by full Harbor ID; the target sees who asks, by Harbor ID.
        let invited = dispatch(
            &mut control,
            signed_request(
                &first_key,
                first,
                "pairing.invite",
                json!({"peer": "harbor-bbbbbbbb"}),
                &rfc3339_now(2),
            ),
            12,
        )
        .unwrap();
        assert!(invited.error.is_none(), "{:?}", invited.error);
        assert_eq!(invited.payload["state"], "WAITING_APPROVAL");
        let pairing_id = invited.payload["pairing_id"].as_str().unwrap().to_owned();

        let incoming = dispatch(
            &mut control,
            signed_request(
                &second_key,
                second,
                "pairing.incoming",
                json!({}),
                &rfc3339_now(3),
            ),
            13,
        )
        .unwrap();
        let requests = incoming.payload["requests"].as_array().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0]["pairing_id"], pairing_id);
        assert_eq!(requests[0]["requester_harbor_id"], "harbor-aaaaaaaa");

        let accepted = dispatch(
            &mut control,
            signed_request(
                &second_key,
                second,
                "pairing.accept",
                json!({"pairing_id": pairing_id}),
                &rfc3339_now(4),
            ),
            14,
        )
        .unwrap();
        assert_eq!(accepted.payload["state"], "ACCEPTED");

        // Re-inviting a paired peer names the relationship instead of forking.
        let again = dispatch(
            &mut control,
            signed_request(
                &first_key,
                first,
                "pairing.invite",
                json!({"peer": "harbor-bbbbbbbb"}),
                &rfc3339_now(5),
            ),
            15,
        )
        .unwrap();
        assert_eq!(again.error.unwrap().code, "pairing_invalid");

        // The legacy code flow still answers beside the new one.
        let created = dispatch(
            &mut control,
            signed_request(
                &second_key,
                second,
                "pairing.create",
                json!({"code": "777777"}),
                &rfc3339_now(6),
            ),
            16,
        )
        .unwrap();
        assert!(created.error.is_none());
    }

    #[test]
    fn presence_and_session_dispatch_stay_within_paired_relationships() {
        let keys: Vec<_> = (1..=3)
            .map(|seed| SigningKey::from_bytes(&[seed; 32]))
            .collect();
        let devices: Vec<_> = (0..3).map(|_| Uuid::new_v4()).collect();
        let mut control = ControlPlane::default();
        for (key, device) in keys.iter().zip(&devices) {
            dispatch(&mut control, signed_identity_update(key, *device), 10).unwrap();
        }
        let [first, second, outsider] = devices.as_slice() else {
            unreachable!("three devices were registered");
        };
        let (first, second, outsider) = (*first, *second, *outsider);

        let created = dispatch(
            &mut control,
            signed_request(
                &keys[1],
                second,
                "pairing.create",
                json!({"code": "654321"}),
                "2026-08-31T20:00:01Z",
            ),
            11,
        )
        .unwrap();
        dispatch(
            &mut control,
            signed_request(
                &keys[0],
                first,
                "pairing.submit",
                json!({"code": "654321"}),
                "2026-08-31T20:00:02Z",
            ),
            12,
        )
        .unwrap();
        dispatch(
            &mut control,
            signed_request(
                &keys[1],
                second,
                "pairing.accept",
                json!({"pairing_id": created.payload["pairing_id"]}),
                "2026-08-31T20:00:03Z",
            ),
            13,
        )
        .unwrap();

        let published = dispatch(
            &mut control,
            signed_request(
                &keys[1],
                second,
                "presence.publish",
                json!({"state": "ONLINE"}),
                "2026-08-31T20:00:04Z",
            ),
            14,
        )
        .unwrap();
        assert_eq!(published.payload["state"], "ONLINE");

        // The paired peer reads the live lease directly; an unpaired device
        // reads nothing at all, exactly as for sessions.
        let partner_view = dispatch(
            &mut control,
            signed_request(
                &keys[0],
                first,
                "presence.status",
                json!({"peer": second}),
                "2026-08-31T20:00:05Z",
            ),
            15,
        )
        .unwrap();
        assert_eq!(partner_view.payload["state"], "ONLINE");
        let unpaired_view = dispatch(
            &mut control,
            signed_request(
                &keys[2],
                outsider,
                "presence.status",
                json!({"peer": second}),
                "2026-08-31T20:00:06Z",
            ),
            16,
        )
        .unwrap();
        assert_eq!(unpaired_view.error.unwrap().code, "unauthorized");

        let rejected_state = dispatch(
            &mut control,
            signed_request(
                &keys[1],
                second,
                "presence.publish",
                json!({"state": "AWAY"}),
                "2026-08-31T20:00:07Z",
            ),
            17,
        )
        .unwrap();
        assert_eq!(rejected_state.error.unwrap().code, "invalid_request");

        let connected = dispatch(
            &mut control,
            signed_request(
                &keys[0],
                first,
                "session.connect",
                json!({"peer": second}),
                "2026-08-31T20:00:08Z",
            ),
            18,
        )
        .unwrap();
        let session_id = connected.payload["session_id"].as_str().unwrap();

        let intruder = dispatch(
            &mut control,
            signed_request(
                &keys[2],
                outsider,
                "session.connect",
                json!({"peer": first}),
                "2026-08-31T20:00:09Z",
            ),
            19,
        )
        .unwrap();
        assert_eq!(intruder.error.unwrap().code, "unauthorized");

        let signalled = dispatch(
            &mut control,
            signed_request(
                &keys[0],
                first,
                "session.signal",
                json!({"session_id": session_id, "signal": r#"{"sdp":"o=- 1 2"}"#}),
                "2026-08-31T20:00:10Z",
            ),
            20,
        )
        .unwrap();
        assert_eq!(signalled.payload["queued"], true);

        let oversized = dispatch(
            &mut control,
            signed_request(
                &keys[1],
                second,
                "session.signal",
                json!({"session_id": session_id, "signal": "x".repeat(MAX_SIGNAL_BYTES + 1)}),
                "2026-08-31T20:00:11Z",
            ),
            21,
        )
        .unwrap();
        assert_eq!(oversized.error.unwrap().code, "invalid_request");

        // The queued signal reaches exactly the session's other peer, once.
        let polled = dispatch(
            &mut control,
            signed_request(
                &keys[1],
                second,
                "session.signal_poll",
                json!({"session_id": session_id}),
                "2026-08-31T20:00:12Z",
            ),
            22,
        )
        .unwrap();
        let drained = polled.payload["signals"].as_array().unwrap();
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0]["from"], first.to_string());
        assert_eq!(drained[0]["signal"], r#"{"sdp":"o=- 1 2"}"#);

        // A caller never receives its own queued signaling back, and the
        // queue is consumed by the drain.
        let own = dispatch(
            &mut control,
            signed_request(
                &keys[0],
                first,
                "session.signal_poll",
                json!({"session_id": session_id}),
                "2026-08-31T20:00:13Z",
            ),
            23,
        )
        .unwrap();
        assert_eq!(own.payload["signals"].as_array().unwrap().len(), 0);
        let drained_again = dispatch(
            &mut control,
            signed_request(
                &keys[1],
                second,
                "session.signal_poll",
                json!({"session_id": session_id}),
                "2026-08-31T20:00:14Z",
            ),
            24,
        )
        .unwrap();
        assert_eq!(
            drained_again.payload["signals"].as_array().unwrap().len(),
            0
        );

        // Pairing relationships resolve as call targets for their members.
        let contacts = dispatch(
            &mut control,
            signed_request(
                &keys[0],
                first,
                "contacts.list",
                json!({}),
                "2026-08-31T20:00:15Z",
            ),
            25,
        )
        .unwrap();
        let peers = contacts.payload["peers"].as_array().unwrap();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0]["device_id"], second.to_string());
    }

    #[test]
    fn registered_identity_cannot_be_impersonated() {
        let key = SigningKey::from_bytes(&[9; 32]);
        let attacker = SigningKey::from_bytes(&[3; 32]);
        let mut control = ControlPlane::default();
        let device_id = Uuid::new_v4();
        dispatch(&mut control, signed_identity_update(&key, device_id), 10).unwrap();

        let request = AuthenticatedEnvelope::sign(
            device_id,
            Envelope::request("pairing.incoming", json!({}), "2026-08-31T20:00:01Z"),
            &attacker,
        )
        .unwrap();
        let response = dispatch(&mut control, request, 11).unwrap();
        assert_eq!(response.error.unwrap().code, "unauthorized");
    }

    #[test]
    fn identity_update_cannot_replace_registered_key_or_relationships() {
        let registered_key = SigningKey::from_bytes(&[21; 32]);
        let peer_key = SigningKey::from_bytes(&[22; 32]);
        let attacker_key = SigningKey::from_bytes(&[23; 32]);
        let registered = Uuid::new_v4();
        let peer = Uuid::new_v4();
        let mut control = ControlPlane::default();
        assert!(
            dispatch(
                &mut control,
                signed_identity_update(&registered_key, registered),
                10,
            )
            .unwrap()
            .error
            .is_none()
        );
        assert!(
            dispatch(&mut control, signed_identity_update(&peer_key, peer), 10)
                .unwrap()
                .error
                .is_none()
        );

        let created = dispatch(
            &mut control,
            signed_request(
                &peer_key,
                peer,
                "pairing.create",
                json!({"code": "987654"}),
                "2026-08-31T20:00:01Z",
            ),
            11,
        )
        .unwrap();
        dispatch(
            &mut control,
            signed_request(
                &registered_key,
                registered,
                "pairing.submit",
                json!({"code": "987654"}),
                "2026-08-31T20:00:02Z",
            ),
            12,
        )
        .unwrap();
        dispatch(
            &mut control,
            signed_request(
                &peer_key,
                peer,
                "pairing.accept",
                json!({"pairing_id": created.payload["pairing_id"]}),
                "2026-08-31T20:00:03Z",
            ),
            13,
        )
        .unwrap();
        let before_attack = control.snapshot();

        let attack = dispatch(
            &mut control,
            signed_identity_update(&attacker_key, registered),
            14,
        )
        .unwrap();

        assert_eq!(attack.error.unwrap().code, "unauthorized");
        assert_eq!(control.snapshot(), before_attack);
        assert_eq!(
            control.identity(registered).unwrap().public_key,
            STANDARD_NO_PAD.encode(registered_key.verifying_key().as_bytes())
        );
        assert_eq!(control.paired_peers(registered).unwrap()[0].device_id, peer);

        let unmodeled_rotation = dispatch(
            &mut control,
            signed_request(
                &registered_key,
                registered,
                "identity.update",
                json!({
                    "device_id": registered,
                    "harbor_id": "harbor-aabbccdd",
                    "public_key": STANDARD_NO_PAD.encode(attacker_key.verifying_key().as_bytes()),
                }),
                &rfc3339_now(1),
            ),
            15,
        )
        .unwrap();
        assert_eq!(unmodeled_rotation.error.unwrap().code, "unauthorized");
        assert_eq!(control.snapshot(), before_attack);
    }

    #[test]
    fn durable_state_survives_a_restart_without_resurrecting_pending_pairings() {
        let directory =
            std::env::temp_dir().join(format!("harbor-server-core-test-{}", Uuid::new_v4()));
        let first_key = SigningKey::from_bytes(&[11; 32]);
        let second_key = SigningKey::from_bytes(&[12; 32]);
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let now = unix_now();

        {
            let mut server = ServerCore::open(&directory).unwrap();
            server
                .handle(signed_identity_update(&first_key, first), now)
                .unwrap();
            server
                .handle(signed_identity_update(&second_key, second), now)
                .unwrap();
            let created = server
                .handle(
                    signed_request(
                        &second_key,
                        second,
                        "pairing.create",
                        json!({"code": "246810"}),
                        &rfc3339_now(0),
                    ),
                    now,
                )
                .unwrap();
            server
                .handle(
                    signed_request(
                        &first_key,
                        first,
                        "pairing.submit",
                        json!({"code": "246810"}),
                        &rfc3339_now(1),
                    ),
                    now,
                )
                .unwrap();
            server
                .handle(
                    signed_request(
                        &second_key,
                        second,
                        "pairing.accept",
                        json!({"pairing_id": created.payload["pairing_id"]}),
                        &rfc3339_now(2),
                    ),
                    now,
                )
                .unwrap();
        }

        let mut restarted = ServerCore::open(&directory).unwrap();
        let stale_pairing = restarted
            .handle(
                signed_request(
                    &first_key,
                    first,
                    "pairing.submit",
                    json!({"code": "246810"}),
                    &rfc3339_now(3),
                ),
                unix_now(),
            )
            .unwrap();
        assert_eq!(stale_pairing.error.unwrap().code, "pairing_unavailable");

        let connected = restarted
            .handle(
                signed_request(
                    &first_key,
                    first,
                    "session.connect",
                    json!({"peer": second}),
                    &rfc3339_now(4),
                ),
                unix_now(),
            )
            .unwrap();
        assert!(connected.error.is_none());

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn open_surfaces_structured_restore_errors_from_persisted_snapshots() {
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let cases = [
            (
                ControlSnapshot {
                    identities: vec![
                        IdentityRecord {
                            device_id: first,
                            harbor_id: "harbor-aaaaaaaa".into(),
                            public_key: "one".into(),
                            registered_at: 1,
                        },
                        IdentityRecord {
                            device_id: second,
                            harbor_id: "harbor-AAAAAAAA".into(),
                            public_key: "two".into(),
                            registered_at: 2,
                        },
                    ],
                    relationships: vec![],
                },
                RestoreError::DuplicateHarborId {
                    harbor_id: "harbor-AAAAAAAA".into(),
                    first_device: first,
                    second_device: second,
                },
            ),
            (
                ControlSnapshot {
                    identities: vec![
                        IdentityRecord {
                            device_id: first,
                            harbor_id: "harbor-aaaaaaaa".into(),
                            public_key: "one".into(),
                            registered_at: 1,
                        },
                        IdentityRecord {
                            device_id: first,
                            harbor_id: "harbor-bbbbbbbb".into(),
                            public_key: "two".into(),
                            registered_at: 2,
                        },
                    ],
                    relationships: vec![],
                },
                RestoreError::DuplicateDeviceId { device_id: first },
            ),
            (
                ControlSnapshot {
                    identities: vec![IdentityRecord {
                        device_id: first,
                        harbor_id: "harbor-aaaaaaaa".into(),
                        public_key: "one".into(),
                        registered_at: 1,
                    }],
                    relationships: vec![(first, second)],
                },
                RestoreError::DanglingRelationship { first, second },
            ),
            (
                ControlSnapshot {
                    identities: vec![IdentityRecord {
                        device_id: first,
                        harbor_id: "harbor-aaaaaaaa".into(),
                        public_key: "one".into(),
                        registered_at: 1,
                    }],
                    relationships: vec![(first, first)],
                },
                RestoreError::SelfRelationship { device_id: first },
            ),
        ];

        for (snapshot, expected) in cases {
            let directory =
                std::env::temp_dir().join(format!("harbor-server-restore-test-{}", Uuid::new_v4()));
            store_snapshot(&directory, &snapshot);
            let result = ServerCore::open(&directory);
            assert!(matches!(result, Err(StoreError::Restore(error)) if error == expected));
            std::fs::remove_dir_all(directory).unwrap();
        }
    }

    #[test]
    fn open_restores_legacy_harbor_test_identity_relationship_and_pairing_codes() {
        let directory =
            std::env::temp_dir().join(format!("harbor-server-legacy-test-{}", Uuid::new_v4()));
        let first_key = SigningKey::from_bytes(&[21; 32]);
        let second_key = SigningKey::from_bytes(&[22; 32]);
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        store_snapshot(
            &directory,
            &ControlSnapshot {
                identities: vec![
                    IdentityRecord {
                        device_id: first,
                        harbor_id: "harbor-test".into(),
                        public_key: STANDARD_NO_PAD.encode(first_key.verifying_key().as_bytes()),
                        registered_at: 1,
                    },
                    IdentityRecord {
                        device_id: second,
                        harbor_id: "harbor-peer".into(),
                        public_key: STANDARD_NO_PAD.encode(second_key.verifying_key().as_bytes()),
                        registered_at: 1,
                    },
                ],
                relationships: vec![(first, second)],
            },
        );

        {
            let server = ServerCore::open(&directory).unwrap();
            assert_eq!(server.identity(first).unwrap().harbor_id, "harbor-test");
            assert_eq!(
                server.control.paired_peers(first).unwrap()[0].device_id,
                second
            );
        }
        let mut server = ServerCore::open(&directory).unwrap();
        assert_eq!(server.identity(first).unwrap().harbor_id, "harbor-test");
        assert_eq!(
            server.control.paired_peers(first).unwrap()[0].device_id,
            second
        );

        let created = server
            .handle(
                signed_request(
                    &second_key,
                    second,
                    "pairing.create",
                    json!({"code": "123456"}),
                    &rfc3339_now(0),
                ),
                unix_now(),
            )
            .unwrap();
        assert!(created.error.is_none(), "{created:?}");
        let submitted = server
            .handle(
                signed_request(
                    &first_key,
                    first,
                    "pairing.submit",
                    json!({"code": "123456"}),
                    &rfc3339_now(1),
                ),
                unix_now(),
            )
            .unwrap();
        assert!(submitted.error.is_none(), "{submitted:?}");

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn requests_outside_the_clock_window_are_refused_before_any_transition() {
        let key = SigningKey::from_bytes(&[13; 32]);
        let device_id = Uuid::new_v4();
        let directory =
            std::env::temp_dir().join(format!("harbor-server-core-test-{}", Uuid::new_v4()));
        let mut server = ServerCore::open(&directory).unwrap();
        let identity_payload = || {
            json!({
                "device_id": device_id,
                "harbor_id": "harbor-aabbccdd",
                "public_key": STANDARD_NO_PAD.encode(key.verifying_key().as_bytes()),
            })
        };

        let stale = server
            .handle(
                signed_request(
                    &key,
                    device_id,
                    "identity.update",
                    identity_payload(),
                    "2020-01-01T00:00:00Z",
                ),
                unix_now(),
            )
            .unwrap();
        assert_eq!(stale.error.unwrap().code, "stale_timestamp");
        assert!(server.identity(device_id).is_none());

        let future = server
            .handle(
                signed_request(
                    &key,
                    device_id,
                    "identity.update",
                    identity_payload(),
                    &rfc3339_now(3_600),
                ),
                unix_now(),
            )
            .unwrap();
        assert_eq!(future.error.unwrap().code, "stale_timestamp");
        assert!(server.identity(device_id).is_none());

        // Inside the window the same request registers the identity.
        let current = server
            .handle(
                signed_request(
                    &key,
                    device_id,
                    "identity.update",
                    identity_payload(),
                    &rfc3339_now(0),
                ),
                unix_now(),
            )
            .unwrap();
        assert!(current.error.is_none());
        assert!(server.identity(device_id).is_some());

        std::fs::remove_dir_all(directory).unwrap();
    }

    /// Activity never leaves a device core. Its private UI↔core messages cannot
    /// be signed for, or represented on, the Harbor Server transport.
    #[test]
    fn turn_credentials_mint_short_lived_passwords_for_verified_devices() {
        let key = SigningKey::from_bytes(&[17; 32]);
        let attacker = SigningKey::from_bytes(&[18; 32]);
        let device_id = Uuid::new_v4();
        let directory =
            std::env::temp_dir().join(format!("harbor-server-turn-cred-test-{}", Uuid::new_v4()));
        let mut server = ServerCore::open(&directory).unwrap();
        let now = unix_now();

        server
            .handle_observed(signed_identity_update(&key, device_id), now, None)
            .unwrap();

        let minted = server
            .handle_observed(
                signed_request(
                    &key,
                    device_id,
                    "turn.credentials",
                    json!({}),
                    &rfc3339_now(0),
                ),
                now,
                None,
            )
            .unwrap();
        assert!(minted.error.is_none(), "{minted:?}");
        assert_eq!(minted.payload["username"], json!(device_id.to_string()));
        assert_eq!(minted.payload["realm"], json!("harbor"));
        assert_eq!(
            minted.payload["ttl_secs"].as_u64().unwrap(),
            TURN_CRED_TTL_SECS
        );
        assert!(minted.payload["expires_at"].as_u64().unwrap() > now);
        let password = minted.payload["password"].as_str().unwrap().to_owned();
        assert_eq!(password.len(), 32);

        // The minted password authenticates against the very state the UDP
        // loop reads: same store, no separate provisioning step.
        let turn = server.turn();
        let guard = turn.lock().unwrap_or_else(|poison| poison.into_inner());
        let stored = guard
            .credential_password_for_tests(&device_id.to_string())
            .unwrap();
        assert_eq!(stored, password);
        drop(guard);

        // Wrong key for a known device id: no password leaks.
        let forged = server
            .handle_observed(
                signed_request(
                    &attacker,
                    device_id,
                    "turn.credentials",
                    json!({}),
                    &rfc3339_now(0),
                ),
                now,
                None,
            )
            .unwrap();
        assert_eq!(forged.error.unwrap().code, "unauthorized");

        // Unknown device: nothing leaks either.
        let stranger = server
            .handle_observed(
                signed_request(
                    &attacker,
                    Uuid::new_v4(),
                    "turn.credentials",
                    json!({}),
                    &rfc3339_now(0),
                ),
                now,
                None,
            )
            .unwrap();
        assert_eq!(stranger.error.unwrap().code, "unauthorized");

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn endpoint_self_echoes_the_current_source_and_rejects_strangers() {
        let key = SigningKey::from_bytes(&[15; 32]);
        let attacker = SigningKey::from_bytes(&[16; 32]);
        let device_id = Uuid::new_v4();
        let directory =
            std::env::temp_dir().join(format!("harbor-server-endpoint-test-{}", Uuid::new_v4()));
        let mut server = ServerCore::open(&directory).unwrap();
        let now = unix_now();
        let first: SocketAddr = "192.0.2.10:40001".parse().unwrap();
        let second: SocketAddr = "192.0.2.10:40002".parse().unwrap();

        server
            .handle_observed(signed_identity_update(&key, device_id), now, Some(first))
            .unwrap();

        // The served value is the endpoint of this very connection: a NAT
        // rebinding between requests must never read stale.
        let probed = server
            .handle_observed(
                signed_request(&key, device_id, "endpoint.self", json!({}), &rfc3339_now(0)),
                now,
                Some(second),
            )
            .unwrap();
        assert!(probed.error.is_none(), "{probed:?}");
        assert_eq!(probed.payload["address"], json!("192.0.2.10"));
        assert_eq!(probed.payload["port"], json!(40002));
        assert_eq!(probed.payload["transport"], json!("tcp"));
        assert!(probed.payload["observed_at"].as_u64().unwrap() >= now);

        // Wrong key for a known device id: nothing leaks.
        let forged = server
            .handle_observed(
                signed_request(
                    &attacker,
                    device_id,
                    "endpoint.self",
                    json!({}),
                    &rfc3339_now(0),
                ),
                now,
                Some(second),
            )
            .unwrap();
        assert_eq!(forged.error.unwrap().code, "unauthorized");

        // Unknown device: nothing leaks either.
        let stranger = server
            .handle_observed(
                signed_request(
                    &attacker,
                    Uuid::new_v4(),
                    "endpoint.self",
                    json!({}),
                    &rfc3339_now(0),
                ),
                now,
                Some(second),
            )
            .unwrap();
        assert_eq!(stranger.error.unwrap().code, "unauthorized");

        // No transport endpoint (embedded/test path): honest error instead
        // of a guess, and the skew gate still applies first.
        let blind = server
            .handle_observed(
                signed_request(&key, device_id, "endpoint.self", json!({}), &rfc3339_now(0)),
                now,
                None,
            )
            .unwrap();
        assert_eq!(blind.error.unwrap().code, "endpoint_unknown");

        let stale = server
            .handle_observed(
                signed_request(
                    &key,
                    device_id,
                    "endpoint.self",
                    json!({}),
                    "2020-01-01T00:00:00Z",
                ),
                now,
                Some(second),
            )
            .unwrap();
        assert_eq!(stale.error.unwrap().code, "stale_timestamp");

        std::fs::remove_dir_all(directory).unwrap();
    }

    /// Activity never leaves a device core. Its private UI↔core messages cannot
    /// be signed for, or represented on, the Harbor Server transport.
    #[test]
    fn local_activity_requests_cannot_be_signed_for_the_server() {
        let key = SigningKey::from_bytes(&[14; 32]);
        let device_id = Uuid::new_v4();

        for message_type in [
            "activity.state",
            "activity.updated",
            "activity.update",
            "activity.subscribe",
        ] {
            let attempt = AuthenticatedEnvelope::sign(
                device_id,
                Envelope::request(message_type, json!({}), rfc3339_now(0)),
                &key,
            );
            match attempt {
                Err(harbor_protocol::AuthenticationError::InvalidEnvelope(
                    harbor_protocol::ValidationError::ForbiddenMessageType(refused),
                )) => assert_eq!(refused, message_type),
                other => panic!("{message_type} must be refused at envelope validation: {other:?}"),
            }
        }
    }

    #[test]
    fn the_requester_observes_accept_and_decline_through_pairing_status() {
        let host_key = SigningKey::from_bytes(&[3; 32]);
        let peer_key = SigningKey::from_bytes(&[4; 32]);
        let outsider_key = SigningKey::from_bytes(&[5; 32]);
        let host = Uuid::new_v4();
        let peer = Uuid::new_v4();
        let outsider = Uuid::new_v4();
        let mut control = ControlPlane::default();
        for (key, device) in [
            (&host_key, host),
            (&peer_key, peer),
            (&outsider_key, outsider),
        ] {
            dispatch(&mut control, signed_identity_update(key, device), 10).unwrap();
        }

        let created = dispatch(
            &mut control,
            signed_request(
                &host_key,
                host,
                "pairing.create",
                json!({"code": "654321"}),
                &rfc3339_now(0),
            ),
            unix_now(),
        )
        .unwrap();
        let pairing_id = created.payload["pairing_id"].as_str().unwrap().to_owned();

        // Before a submit the requester has nothing to observe yet.
        let pending = dispatch(
            &mut control,
            signed_request(
                &host_key,
                host,
                "pairing.status",
                json!({"pairing_id": pairing_id}),
                &rfc3339_now(0),
            ),
            unix_now(),
        )
        .unwrap();
        assert_eq!(pending.payload["state"], "PENDING_CODE");

        dispatch(
            &mut control,
            signed_request(
                &peer_key,
                peer,
                "pairing.submit",
                json!({"code": "654321"}),
                &rfc3339_now(0),
            ),
            unix_now(),
        )
        .unwrap();
        dispatch(
            &mut control,
            signed_request(
                &host_key,
                host,
                "pairing.accept",
                json!({"pairing_id": pairing_id}),
                &rfc3339_now(0),
            ),
            unix_now(),
        )
        .unwrap();

        // The peer sees the accept; an unrelated device does not.
        let observed = dispatch(
            &mut control,
            signed_request(
                &peer_key,
                peer,
                "pairing.status",
                json!({"pairing_id": pairing_id}),
                &rfc3339_now(0),
            ),
            unix_now(),
        )
        .unwrap();
        assert_eq!(observed.payload["state"], "ACCEPTED");

        let refused = dispatch(
            &mut control,
            signed_request(
                &outsider_key,
                outsider,
                "pairing.status",
                json!({"pairing_id": pairing_id}),
                &rfc3339_now(0),
            ),
            unix_now(),
        )
        .unwrap();
        assert_eq!(refused.error.unwrap().code, "unauthorized");

        // A declined pairing is observable the same way.
        let created = dispatch(
            &mut control,
            signed_request(
                &host_key,
                host,
                "pairing.create",
                json!({"code": "765432"}),
                &rfc3339_now(0),
            ),
            unix_now(),
        )
        .unwrap();
        let pairing_id = created.payload["pairing_id"].as_str().unwrap().to_owned();
        dispatch(
            &mut control,
            signed_request(
                &peer_key,
                peer,
                "pairing.submit",
                json!({"code": "765432"}),
                &rfc3339_now(0),
            ),
            unix_now(),
        )
        .unwrap();
        dispatch(
            &mut control,
            signed_request(
                &host_key,
                host,
                "pairing.decline",
                json!({"pairing_id": pairing_id}),
                &rfc3339_now(0),
            ),
            unix_now(),
        )
        .unwrap();
        let declined = dispatch(
            &mut control,
            signed_request(
                &peer_key,
                peer,
                "pairing.status",
                json!({"pairing_id": pairing_id}),
                &rfc3339_now(0),
            ),
            unix_now(),
        )
        .unwrap();
        assert_eq!(declined.payload["state"], "DECLINED");
    }
}
