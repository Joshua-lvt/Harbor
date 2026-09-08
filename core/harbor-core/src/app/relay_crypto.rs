//! End-to-end encryption for relayed frames: the server routes ciphertext
//! and never holds keys.
//!
//! Per relay session both sides generate an ephemeral X25519 keypair. The
//! public halves travel inside the signed `relay.open` / `relay.accept`
//! envelopes, so a relay that swaps them breaks the Ed25519 signatures and
//! is detected. HKDF-SHA256 over the Diffie-Hellman shared secret (salt =
//! relay id, info binds direction) derives two ChaCha20-Poly1305 keys, one
//! per direction; sequence numbers ride as associated data (replays and
//! reorders rejected) inside 12-byte nonces (4 zero bytes + 64-bit
//! big-endian sequence). Keys differ per direction, so sequence-only nonces
//! never repeat within a key. Sessions are time-boxed by the server TTL, so
//! counters never approach wraparound; overflow is a hard error, never a
//! wrap. Secrets zeroize on drop; `Debug` never prints key material.

use chacha20poly1305::{
    ChaCha20Poly1305, Key, KeyInit, Nonce,
    aead::{Aead, Payload},
};
use hkdf::Hkdf;
use sha2::Sha256;
use thiserror::Error;
use uuid::Uuid;
use x25519_dalek::{PublicKey, StaticSecret};

/// Largest plaintext one frame seals. Ciphertext lands under the server's
/// 48 KiB string cap with base64 overhead to spare.
pub const MAX_PLAINTEXT_BYTES: usize = 32 * 1024;

const INFO_AB: &[u8] = b"harbor-relay-v1/opener-to-peer";
const INFO_BA: &[u8] = b"harbor-relay-v1/peer-to-opener";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum CryptoError {
    #[error("key derivation failed")]
    KeyDerivation,
    #[error("plaintext exceeds the frame budget")]
    PlaintextTooLarge,
    #[error("sequence counter exhausted; open a fresh session")]
    SequenceExhausted,
    #[error("frame is replayed, reordered, or forged")]
    Rejected,
}

/// One sealed frame: the sequence the recipient must expect next, plus the
/// ciphertext whose associated data binds that same sequence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SealedFrame {
    pub seq: u64,
    pub ciphertext: Vec<u8>,
}

/// An ephemeral keypair awaiting the peer's public half. The secret never
/// leaves this struct; completing consumes it.
pub struct PendingCrypto {
    secret: StaticSecret,
}

impl PendingCrypto {
    pub fn generate() -> Self {
        Self {
            secret: StaticSecret::random(),
        }
    }

    /// Deterministic construction for tests and vectors. Production uses
    /// [`PendingCrypto::generate`].
    #[cfg(test)]
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self {
            secret: StaticSecret::from(seed),
        }
    }

    pub fn public_bytes(&self) -> [u8; 32] {
        *PublicKey::from(&self.secret).as_bytes()
    }

    /// Completes the handshake: Diffie-Hellman against the peer's public
    /// half, then one ChaCha20-Poly1305 key per direction. `opener` picks
    /// which derived key sends: the opener's sends use the opener→peer key
    /// on both sides, so directions agree without extra negotiation.
    /// Borrows so callers can upgrade in place (the replaced `Pending` drops
    /// and zeroizes); complete once per ephemeral — the upgrade must replace
    /// the pending value immediately, never keep both.
    pub fn complete(
        &self,
        peer_public: [u8; 32],
        relay_id: Uuid,
        opener: bool,
    ) -> Result<RelayCrypto, CryptoError> {
        let peer = PublicKey::from(peer_public);
        let shared = self.secret.diffie_hellman(&peer);
        let hkdf = Hkdf::<Sha256>::new(Some(relay_id.as_bytes()), shared.as_bytes());
        let mut key_ab = [0_u8; 32];
        let mut key_ba = [0_u8; 32];
        hkdf.expand(INFO_AB, &mut key_ab)
            .and_then(|()| hkdf.expand(INFO_BA, &mut key_ba))
            .map_err(|_| CryptoError::KeyDerivation)?;
        let (send_key, recv_key) = if opener {
            (key_ab, key_ba)
        } else {
            (key_ba, key_ab)
        };
        Ok(RelayCrypto {
            send: ChaCha20Poly1305::new(Key::from_slice(&send_key)),
            recv: ChaCha20Poly1305::new(Key::from_slice(&recv_key)),
            send_seq: 0,
            recv_next: 0,
        })
    }
}

/// An established relay cryptor: seals in sequence order, opens in strict
/// sequence order. Directions use independent keys and counters.
pub struct RelayCrypto {
    send: ChaCha20Poly1305,
    recv: ChaCha20Poly1305,
    send_seq: u64,
    recv_next: u64,
}

impl std::fmt::Debug for RelayCrypto {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RelayCrypto")
            .field("send_seq", &self.send_seq)
            .field("recv_next", &self.recv_next)
            .finish_non_exhaustive()
    }
}

fn nonce_for(seq: u64) -> [u8; 12] {
    let mut nonce = [0_u8; 12];
    nonce[4..].copy_from_slice(&seq.to_be_bytes());
    nonce
}

impl RelayCrypto {
    /// Seals one plaintext frame in send order. The sequence binds as
    /// associated data, so any tampering with it fails authentication.
    pub fn seal(&mut self, plaintext: &[u8]) -> Result<SealedFrame, CryptoError> {
        if plaintext.len() > MAX_PLAINTEXT_BYTES {
            return Err(CryptoError::PlaintextTooLarge);
        }
        let seq = self.send_seq;
        self.send_seq = seq.checked_add(1).ok_or(CryptoError::SequenceExhausted)?;
        let nonce = nonce_for(seq);
        let ciphertext = self
            .send
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: plaintext,
                    aad: &seq.to_be_bytes(),
                },
            )
            .map_err(|_| CryptoError::Rejected)?;
        Ok(SealedFrame { seq, ciphertext })
    }

    /// Opens the next expected frame. Anything else — replay, gap, forgery,
    /// wrong-direction key, swapped peer key — is rejected and advances
    /// nothing.
    pub fn open(&mut self, seq: u64, ciphertext: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if seq != self.recv_next {
            return Err(CryptoError::Rejected);
        }
        let nonce = nonce_for(seq);
        let plaintext = self
            .recv
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: ciphertext,
                    aad: &seq.to_be_bytes(),
                },
            )
            .map_err(|_| CryptoError::Rejected)?;
        self.recv_next = seq.saturating_add(1);
        Ok(plaintext)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair(
        opener_seed: [u8; 32],
        peer_seed: [u8; 32],
        relay_id: Uuid,
    ) -> (RelayCrypto, RelayCrypto) {
        let opener_pending = PendingCrypto::from_seed(opener_seed);
        let peer_pending = PendingCrypto::from_seed(peer_seed);
        let opener_pub = opener_pending.public_bytes();
        let peer_pub = peer_pending.public_bytes();
        let opener = opener_pending.complete(peer_pub, relay_id, true).unwrap();
        let peer = peer_pending.complete(opener_pub, relay_id, false).unwrap();
        (opener, peer)
    }

    #[test]
    fn both_directions_round_trip_in_order() {
        let relay_id = Uuid::new_v4();
        let (mut opener, mut peer) = pair([1; 32], [2; 32], relay_id);
        for (index, body) in ["hello", "a longer second message", "3"]
            .into_iter()
            .enumerate()
        {
            let sealed = opener.seal(body.as_bytes()).unwrap();
            assert_eq!(sealed.seq, index as u64);
            assert_eq!(
                peer.open(sealed.seq, &sealed.ciphertext).unwrap(),
                body.as_bytes()
            );
        }
        for (index, body) in ["back", "forth"].into_iter().enumerate() {
            let sealed = peer.seal(body.as_bytes()).unwrap();
            assert_eq!(sealed.seq, index as u64);
            assert_eq!(
                opener.open(sealed.seq, &sealed.ciphertext).unwrap(),
                body.as_bytes()
            );
        }
    }

    #[test]
    fn same_seeds_derive_same_ciphertext() {
        let relay_id = Uuid::new_v4();
        let (mut first_a, _) = pair([3; 32], [4; 32], relay_id);
        let (mut second_a, _) = pair([3; 32], [4; 32], relay_id);
        assert_eq!(
            first_a.seal(b"deterministic").unwrap(),
            second_a.seal(b"deterministic").unwrap()
        );
    }

    #[test]
    fn direction_keys_do_not_interoperate() {
        let relay_id = Uuid::new_v4();
        let (mut opener, mut peer) = pair([5; 32], [6; 32], relay_id);
        // Same sequence number exists in both directions, but the keys
        // differ: neither side opens a frame sealed for the opposite
        // direction, and the failures advance nothing.
        let to_peer = opener.seal(b"wrong way").unwrap();
        let to_opener = peer.seal(b"other way").unwrap();
        assert_eq!(
            peer.open(to_opener.seq, &to_opener.ciphertext),
            Err(CryptoError::Rejected)
        );
        assert_eq!(
            opener.open(to_peer.seq, &to_peer.ciphertext),
            Err(CryptoError::Rejected)
        );
        assert_eq!(
            peer.open(to_peer.seq, &to_peer.ciphertext).unwrap(),
            b"wrong way"
        );
        assert_eq!(
            opener.open(to_opener.seq, &to_opener.ciphertext).unwrap(),
            b"other way"
        );
    }

    #[test]
    fn replay_gap_and_forgery_are_rejected_without_advancing() {
        let relay_id = Uuid::new_v4();
        let (mut opener, mut peer) = pair([7; 32], [8; 32], relay_id);
        let first = opener.seal(b"one").unwrap();
        let second = opener.seal(b"two").unwrap();
        // Gap first: strict order refuses the jump.
        assert_eq!(
            peer.open(second.seq, &second.ciphertext),
            Err(CryptoError::Rejected)
        );
        // Forged bytes under the right sequence fail authentication.
        let mut forged = first.ciphertext.clone();
        forged[0] ^= 0xff;
        assert_eq!(peer.open(first.seq, &forged), Err(CryptoError::Rejected));
        // The real first frame still opens afterwards: failures advance nothing.
        assert_eq!(peer.open(first.seq, &first.ciphertext).unwrap(), b"one");
        // Replay of it now fails.
        assert_eq!(
            peer.open(first.seq, &first.ciphertext),
            Err(CryptoError::Rejected)
        );
        assert_eq!(peer.open(second.seq, &second.ciphertext).unwrap(), b"two");
    }

    #[test]
    fn swapped_peer_key_detects_a_meddling_relay() {
        let relay_id = Uuid::new_v4();
        let opener_pending = PendingCrypto::from_seed([9; 32]);
        let peer_pending = PendingCrypto::from_seed([10; 32]);
        let attacker = PendingCrypto::from_seed([11; 32]);
        let opener_pub = opener_pending.public_bytes();
        // The relay swaps the peer's public half for the attacker's on the
        // opener's side. Handshakes "complete" — but no frame ever opens.
        let mut opener = opener_pending
            .complete(attacker.public_bytes(), relay_id, true)
            .unwrap();
        let mut peer = peer_pending.complete(opener_pub, relay_id, false).unwrap();
        let sealed = opener.seal(b"secret").unwrap();
        assert_eq!(
            peer.open(sealed.seq, &sealed.ciphertext),
            Err(CryptoError::Rejected)
        );
    }

    #[test]
    fn oversize_plaintext_is_refused_before_touching_keys() {
        let relay_id = Uuid::new_v4();
        let (mut opener, _) = pair([12; 32], [13; 32], relay_id);
        assert_eq!(
            opener.seal(&vec![0_u8; MAX_PLAINTEXT_BYTES + 1]),
            Err(CryptoError::PlaintextTooLarge)
        );
        assert!(opener.seal(&vec![0_u8; MAX_PLAINTEXT_BYTES]).is_ok());
    }

    #[test]
    fn refused_sealed_sends_still_consume_the_counter() {
        let relay_id = Uuid::new_v4();
        let (mut opener, mut peer) = pair([16; 32], [17; 32], relay_id);
        let refused = opener.seal(b"refused after sealing").unwrap();
        let next = opener.seal(b"different plaintext").unwrap();
        assert_eq!(refused.seq, 0);
        assert_eq!(next.seq, 1);
        assert_ne!(next.seq, refused.seq);
        assert_eq!(
            peer.open(refused.seq, &refused.ciphertext).unwrap(),
            b"refused after sealing"
        );
        assert_eq!(
            peer.open(next.seq, &next.ciphertext).unwrap(),
            b"different plaintext"
        );
    }

    #[test]
    fn debug_never_prints_key_material() {
        let relay_id = Uuid::new_v4();
        let (opener, _) = pair([14; 32], [15; 32], relay_id);
        let rendered = format!("{opener:?}");
        assert!(rendered.contains("send_seq"));
        assert!(!rendered.contains("key"));
    }
}
