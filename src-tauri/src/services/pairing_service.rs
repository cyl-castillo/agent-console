//! Pairing orchestrator (Hito 3 of the secure-pairing MVP).
//!
//! Glues the crypto core (secure_channel) to the trust store
//! (paired_devices_service) into the desktop's view of the pairing lifecycle:
//! hold the long-term identity, mint a pairing offer (the QR), and build the
//! responder side of both the first-contact (XX+PSK) and reconnect (KK)
//! handshakes. The message pumping itself rides the transport (Hito 4); here the
//! integration test drives it in-memory with a simulated phone client to prove
//! the whole flow — pair → human approval → trusted → reconnect-authenticated →
//! E2E message — plus the negative paths (unapproved, revoked, MITM).
#![allow(dead_code)]

use std::sync::Mutex;

use snow::HandshakeState;

use crate::error::{AppError, AppResult};
use crate::services::secure_channel::{
    build_responder, build_responder_known, load_or_create_identity, new_offer, PairingOffer,
    StaticKeypair,
};

/// The currently displayed pairing offer's secret state, kept only in memory and
/// only while the QR is live. One-time: consumed by the first handshake.
struct ActiveOffer {
    psk: Vec<u8>,
    created_ms: i64,
    ttl_secs: u64,
}

pub struct PairingService {
    identity: StaticKeypair,
    active: Mutex<Option<ActiveOffer>>,
}

impl PairingService {
    pub fn new(identity: StaticKeypair) -> Self {
        Self { identity, active: Mutex::new(None) }
    }

    /// Load (or first-run create) the desktop's stable long-term identity.
    pub fn load() -> AppResult<Self> {
        Ok(Self::new(load_or_create_identity()?))
    }

    /// The desktop identity public key — what a phone pins (via the QR) and uses
    /// to authenticate the desktop on reconnect.
    pub fn identity_public(&self) -> Vec<u8> {
        self.identity.public.clone()
    }

    /// Mint a fresh pairing offer (the QR payload) and arm the one-time secret.
    pub fn start_pairing(&self, now_ms: i64, ttl_secs: u64) -> PairingOffer {
        let (offer, psk) = new_offer(&self.identity.public, now_ms, ttl_secs);
        *self.active.lock().unwrap() = Some(ActiveOffer { psk, created_ms: now_ms, ttl_secs });
        offer
    }

    /// Build the responder handshake for an in-flight first-contact pairing.
    /// Consumes the active offer (one-time) and refuses an expired one.
    pub fn pairing_responder(&self, now_ms: i64) -> AppResult<HandshakeState> {
        let offer = self
            .active
            .lock()
            .unwrap()
            .take()
            .ok_or_else(|| AppError::Other("no active pairing offer".into()))?;
        if now_ms.saturating_sub(offer.created_ms) > (offer.ttl_secs as i64) * 1000 {
            return Err(AppError::Other("pairing offer expired".into()));
        }
        build_responder(&self.identity.private, &offer.psk)
    }

    /// Build the responder handshake for a reconnect of an already-paired device
    /// (KK; the caller must have looked the device up in the trust store first).
    pub fn reconnect_responder(&self, device_public: &[u8]) -> AppResult<HandshakeState> {
        build_responder_known(&self.identity.private, device_public)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::paired_devices_service::{DeviceScope, PairedDevicesService};
    use crate::services::secure_channel::{
        build_initiator, build_initiator_known, decrypt, drive_handshake, encrypt, generate_static,
        verify_pinned_remote,
    };
    use data_encoding::BASE64URL_NOPAD;
    use std::time::{SystemTime, UNIX_EPOCH};

    /// A stand-in for the phone: its own identity, and (after pairing) the pinned
    /// desktop identity it will authenticate against on reconnect.
    struct PhoneClient {
        keys: StaticKeypair,
        pinned_desktop: Option<Vec<u8>>,
    }

    impl PhoneClient {
        fn new() -> Self {
            Self { keys: generate_static().unwrap(), pinned_desktop: None }
        }

        /// Consume a scanned offer and build the initiator (XX+PSK) side. Records
        /// the desktop key to pin.
        fn pair_initiator(&mut self, offer: &PairingOffer) -> HandshakeState {
            let pinned = offer.responder_static_bytes().unwrap();
            self.pinned_desktop = Some(pinned);
            build_initiator(&self.keys.private, &offer.psk_bytes().unwrap()).unwrap()
        }

        /// Build the reconnect (KK) initiator using the pinned desktop key.
        fn reconnect_initiator(&self) -> HandshakeState {
            build_initiator_known(
                &self.keys.private,
                self.pinned_desktop.as_ref().expect("must have paired first"),
            )
            .unwrap()
        }
    }

    fn isolated_data_dir() {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let base = std::env::temp_dir()
            .join(format!("ac-pairing-test-{}-{}", std::process::id(), nanos));
        std::fs::create_dir_all(&base).unwrap();
        std::env::set_var("XDG_DATA_HOME", &base);
    }

    #[test]
    fn full_lifecycle_pair_approve_reconnect_revoke() {
        let _env = crate::test_support::lock_env();
        isolated_data_dir();
        let now = 1_700_000_000_000_i64;

        // Desktop: stable identity (created on first run) + the trust store.
        let desktop = PairingService::load().unwrap();
        let devices = PairedDevicesService::new();

        // Identity persists: a second load yields the same public key.
        assert_eq!(
            PairingService::load().unwrap().identity_public(),
            desktop.identity_public(),
            "desktop identity must be stable across loads"
        );

        // --- 1. First-contact pairing (XX + PSK), QR out-of-band ---
        let offer = desktop.start_pairing(now, 60);
        let mut phone = PhoneClient::new();
        let initiator = phone.pair_initiator(&offer);
        let responder = desktop.pairing_responder(now).unwrap();
        let (init_hs, resp_hs) =
            drive_handshake(initiator, responder).expect("pairing handshake succeeds");

        // Phone confirms it really spoke to the QR's desktop (anti-MITM pin).
        verify_pinned_remote(&init_hs, &desktop.identity_public()).unwrap();
        // Desktop learns the phone's identity → proposes (does NOT trust yet).
        let phone_pub = resp_hs.get_remote_static().unwrap().to_vec();
        let phone_pub_b64 = BASE64URL_NOPAD.encode(&phone_pub);
        let pid = devices.propose_pairing("Carlos iPhone", &phone_pub_b64, DeviceScope::ReadApprovals, now);
        assert!(devices.find_by_key(&phone_pub_b64).unwrap().is_none(), "pending ≠ trusted");

        // --- 2. Reconnect BEFORE approval must be refused ---
        assert!(
            reconnect(&desktop, &devices, &mut phone, &phone_pub_b64).is_err(),
            "an unapproved device cannot reconnect"
        );

        // --- 3. Human approves on the desktop → trusted ---
        devices.approve_pairing(&pid, now).unwrap();
        assert!(devices.find_by_key(&phone_pub_b64).unwrap().is_some());

        // --- 4. Reconnect (KK, no QR) now authenticates and carries E2E traffic ---
        let (mut p_t, mut d_t) =
            reconnect(&desktop, &devices, &mut phone, &phone_pub_b64).expect("approved reconnect works");
        let ct = encrypt(&mut p_t, b"approve the deploy?").unwrap();
        assert_eq!(decrypt(&mut d_t, &ct).unwrap(), b"approve the deploy?");
        let ct2 = encrypt(&mut d_t, b"deploying now").unwrap();
        assert_eq!(decrypt(&mut p_t, &ct2).unwrap(), b"deploying now");

        // --- 5. Revoke → the pinned key matches nothing → reconnect refused ---
        let dev = devices.find_by_key(&phone_pub_b64).unwrap().unwrap();
        devices.revoke(&dev.id).unwrap();
        assert!(
            reconnect(&desktop, &devices, &mut phone, &phone_pub_b64).is_err(),
            "a revoked device cannot reconnect"
        );
    }

    #[test]
    fn reconnect_with_wrong_key_is_rejected() {
        let _env = crate::test_support::lock_env();
        isolated_data_dir();
        let now = 1_700_000_000_000_i64;
        let desktop = PairingService::load().unwrap();
        let devices = PairedDevicesService::new();

        // Pair + approve a legit phone.
        let offer = desktop.start_pairing(now, 60);
        let mut phone = PhoneClient::new();
        let init = phone.pair_initiator(&offer);
        let resp = desktop.pairing_responder(now).unwrap();
        let (_i, resp_hs) = drive_handshake(init, resp).unwrap();
        let phone_pub_b64 = BASE64URL_NOPAD.encode(resp_hs.get_remote_static().unwrap());
        let pid = devices.propose_pairing("phone", &phone_pub_b64, DeviceScope::Full, now);
        devices.approve_pairing(&pid, now).unwrap();

        // An attacker that knows the (public) device key but NOT its private key
        // cannot complete the KK reconnect: the desktop expects the real key.
        let attacker = generate_static().unwrap();
        let attacker_init =
            build_initiator_known(&attacker.private, &desktop.identity_public()).unwrap();
        let real_pub = BASE64URL_NOPAD.decode(phone_pub_b64.as_bytes()).unwrap();
        let desktop_resp = desktop.reconnect_responder(&real_pub).unwrap();
        assert!(
            drive_handshake(attacker_init, desktop_resp).is_err(),
            "KK reconnect must fail when the initiator isn't the pinned device"
        );
    }

    /// Look the device up by its pinned key (the auth gate), then run the KK
    /// reconnect handshake and return both transport states. Errors if the device
    /// is unknown/revoked or the handshake fails.
    fn reconnect(
        desktop: &PairingService,
        devices: &PairedDevicesService,
        phone: &mut PhoneClient,
        phone_pub_b64: &str,
    ) -> AppResult<(snow::TransportState, snow::TransportState)> {
        let dev = devices
            .find_by_key(phone_pub_b64)?
            .ok_or_else(|| AppError::Other("unknown/revoked device".into()))?;
        let device_pub = BASE64URL_NOPAD
            .decode(dev.public_key.as_bytes())
            .map_err(|e| AppError::Other(format!("decode device key: {e}")))?;
        let initiator = phone.reconnect_initiator();
        let responder = desktop.reconnect_responder(&device_pub)?;
        let (init_hs, resp_hs) = drive_handshake(initiator, responder)?;
        let p_t = init_hs
            .into_transport_mode()
            .map_err(|e| AppError::Other(format!("phone transport: {e}")))?;
        let d_t = resp_hs
            .into_transport_mode()
            .map_err(|e| AppError::Other(format!("desktop transport: {e}")))?;
        Ok((p_t, d_t))
    }
}
