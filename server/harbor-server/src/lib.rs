//! Authenticated dispatch for the future Harbor control-plane listener.
//!
//! This module accepts only signed control envelopes. It is intentionally
//! transport-independent so TLS, connection limits, and persistence can be
//! reviewed before a listener is exposed on the K11+.

use std::path::Path;

use harbor_control::{ControlError, ControlPlane, IdentityRecord, Presence};
use harbor_protocol::{AuthenticatedEnvelope, Envelope, ProtocolError};
use serde::Serialize;
use serde_json::{Value, json};
use thiserror::Error;
use uuid::Uuid;

mod store;
mod transport;

pub use store::{StateStore, StoreError};
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
}

impl ServerCore {
    pub fn open(directory: &Path) -> Result<Self, StoreError> {
        let store = StateStore::open(directory)?;
        let control = store
            .load()?
            .map_or_else(ControlPlane::default, ControlPlane::restore);
        Ok(Self { control, store })
    }

    /// Handles one authenticated network request. Requests whose timestamp is
    /// outside the server clock tolerance are refused before any state
    /// transition, which bounds how long a captured request stays replayable.
    pub fn handle(
        &mut self,
        authenticated: AuthenticatedEnvelope,
        now: u64,
    ) -> Result<Envelope, HandleError> {
        let durable = DURABLE_MESSAGE_TYPES.contains(&authenticated.envelope.message_type.as_str());
        let response = if timestamp_within_skew(authenticated.envelope.timestamp.as_deref(), now) {
            dispatch(&mut self.control, authenticated, now)?
        } else {
            let mut response = response_envelope(&authenticated.envelope)?;
            response.error = Some(stale_timestamp_error());
            response
        };
        if durable && response.error.is_none() {
            self.flush()?;
        }
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

    let Some(identity) = control.identity(authenticated.signer_id) else {
        response.error = Some(unauthorized_error());
        return Ok(response);
    };
    if authenticated.verify(&identity.public_key).is_err() {
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
        "pairing.incoming" => set_control_response(
            &mut response,
            control
                .incoming_pairings(authenticated.signer_id, now)
                // Envelope payloads are objects by protocol rule; a bare
                // array would fail response validation and drop the reply.
                .map(|requests| json!({ "requests": requests })),
        ),
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
    if device_id != authenticated.signer_id || authenticated.verify(public_key).is_err() {
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
        Err(_) => response.error = Some(invalid_request_error()),
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
    use harbor_control::MAX_SIGNAL_BYTES;
    use harbor_protocol::AuthenticatedEnvelope;

    use super::*;

    fn signed_identity_update(key: &SigningKey, device_id: Uuid) -> AuthenticatedEnvelope {
        signed_request(
            key,
            device_id,
            "identity.update",
            json!({
                "device_id": device_id,
                "harbor_id": "harbor-test",
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

    #[test]
    fn identity_registration_requires_proof_of_the_public_key() {
        let key = SigningKey::from_bytes(&[9; 32]);
        let mut control = ControlPlane::default();
        let device_id = Uuid::new_v4();
        let response = dispatch(&mut control, signed_identity_update(&key, device_id), 10).unwrap();
        assert!(response.error.is_none());
        assert_eq!(
            control.identity(device_id).unwrap().harbor_id,
            "harbor-test"
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
    fn requests_outside_the_clock_window_are_refused_before_any_transition() {
        let key = SigningKey::from_bytes(&[13; 32]);
        let device_id = Uuid::new_v4();
        let directory =
            std::env::temp_dir().join(format!("harbor-server-core-test-{}", Uuid::new_v4()));
        let mut server = ServerCore::open(&directory).unwrap();
        let identity_payload = || {
            json!({
                "device_id": device_id,
                "harbor_id": "harbor-test",
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
