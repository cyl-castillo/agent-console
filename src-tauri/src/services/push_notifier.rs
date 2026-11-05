//! When a decision arrives and nobody answers it at the keyboard, knock on
//! the paired devices (MOVIL F1b). The rules that keep this from becoming
//! noise, or a leak:
//!
//! - **Debounce.** A request is pushed only if it is still pending after
//!   `debounce` (15 s in the app): the human at the keyboard answers in
//!   seconds (p50 2 s in the ledger) and must never get a buzz for that.
//! - **Rate.** At most one push per device per `min_interval` (60 s). A
//!   burst of requests is one knock with the count.
//! - **Content.** The payload is `{"type":"decisions","pending":N}`. The
//!   push service sees a count, never a command.
//! - **Dead subscriptions** (404/410) are dropped from the device record.
//!
//! The sender is a trait so the tests knock without a push service.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::error::AppResult;
use crate::services::paired_devices_service::PairedDevicesService;
use crate::services::push_service::{self, Delivery, PushSubscription, VapidKey};

pub trait PushSender: Send + Sync + 'static {
    fn send(&self, vapid: &VapidKey, sub: &PushSubscription, payload: &[u8])
        -> AppResult<Delivery>;
}

/// The real thing: RFC 8030 over HTTPS.
pub struct HttpSender;

impl PushSender for HttpSender {
    fn send(
        &self,
        vapid: &VapidKey,
        sub: &PushSubscription,
        payload: &[u8],
    ) -> AppResult<Delivery> {
        push_service::send(vapid, sub, payload)
    }
}

#[derive(Default)]
struct State {
    /// A knock is scheduled and will count the queue when it fires.
    scheduled: bool,
    last_sent: HashMap<String, Instant>,
}

pub struct PushNotifier {
    devices: Arc<PairedDevicesService>,
    sender: Arc<dyn PushSender>,
    /// Loaded on first use (keystore or file); `None` when neither answered,
    /// in which case there is no push and `push_info` says so.
    vapid: OnceLock<Option<VapidKey>>,
    state: Mutex<State>,
    debounce: Duration,
    min_interval: Duration,
}

/// What the app uses: 15 s before the first knock, one knock a minute.
pub const DEBOUNCE: Duration = Duration::from_secs(15);
pub const MIN_INTERVAL: Duration = Duration::from_secs(60);

impl PushNotifier {
    pub fn new(devices: Arc<PairedDevicesService>) -> Self {
        Self::with_sender(devices, Arc::new(HttpSender), DEBOUNCE, MIN_INTERVAL)
    }

    pub fn with_sender(
        devices: Arc<PairedDevicesService>,
        sender: Arc<dyn PushSender>,
        debounce: Duration,
        min_interval: Duration,
    ) -> Self {
        Self {
            devices,
            sender,
            vapid: OnceLock::new(),
            state: Mutex::new(State::default()),
            debounce,
            min_interval,
        }
    }

    /// Pin a key (tests; the app loads from the keystore on first use).
    #[cfg(test)]
    pub fn set_vapid(&self, key: VapidKey) {
        let _ = self.vapid.set(Some(key));
    }

    fn vapid(&self) -> Option<&VapidKey> {
        self.vapid
            .get_or_init(|| VapidKey::load_or_create().ok())
            .as_ref()
    }

    /// The `applicationServerKey` a browser subscribes with, base64url.
    pub fn public_key(&self) -> Option<String> {
        self.vapid().map(VapidKey::public_b64)
    }

    /// A decision just arrived. `count_now` is asked when the debounce
    /// elapses — if the queue emptied meanwhile, nothing is sent. Returns
    /// at once; the wait and the sends happen on a thread.
    pub fn knock(self: &Arc<Self>, count_now: Box<dyn Fn() -> usize + Send + Sync>) {
        {
            let mut st = self.state.lock().unwrap();
            if st.scheduled {
                return;
            }
            st.scheduled = true;
        }
        let me = self.clone();
        std::thread::Builder::new()
            .name("push-knock".into())
            .spawn(move || {
                std::thread::sleep(me.debounce);
                me.state.lock().unwrap().scheduled = false;
                let n = count_now();
                if n > 0 {
                    me.send_all(n);
                }
            })
            .ok();
    }

    /// Push "N decisions wait" to every subscribed device not knocked
    /// within `min_interval`. Returns the device ids pushed.
    pub fn send_all(&self, pending: usize) -> Vec<String> {
        let Some(vapid) = self.vapid() else {
            return Vec::new();
        };
        let devices = self.devices.list_devices().unwrap_or_default();
        let payload = push_service::decisions_payload(pending);
        let mut pushed = Vec::new();
        for d in devices {
            let Some(sub) = d.push.as_ref() else { continue };
            {
                let st = self.state.lock().unwrap();
                if let Some(t) = st.last_sent.get(&d.id) {
                    if t.elapsed() < self.min_interval {
                        continue;
                    }
                }
            }
            match self.sender.send(vapid, sub, &payload) {
                Ok(Delivery::Sent) => {
                    self.state
                        .lock()
                        .unwrap()
                        .last_sent
                        .insert(d.id.clone(), Instant::now());
                    pushed.push(d.id.clone());
                }
                Ok(Delivery::Gone) => {
                    let _ = self.devices.set_push(&d.id, None);
                }
                Ok(Delivery::Failed(_)) | Err(_) => {
                    // Transient: the next knock tries again. Nothing to log
                    // beyond the attempt; the payload holds no content.
                }
            }
        }
        pushed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::paired_devices_service::DeviceScope;

    struct FakeSender {
        calls: Mutex<Vec<(String, Vec<u8>)>>,
        reply: Mutex<Delivery>,
    }
    impl PushSender for FakeSender {
        fn send(
            &self,
            _vapid: &VapidKey,
            sub: &PushSubscription,
            payload: &[u8],
        ) -> AppResult<Delivery> {
            self.calls
                .lock()
                .unwrap()
                .push((sub.endpoint.clone(), payload.to_vec()));
            Ok(self.reply.lock().unwrap().clone())
        }
    }

    fn sub(n: u8) -> PushSubscription {
        PushSubscription {
            endpoint: format!("https://push.example/{n}"),
            p256dh: "BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-JvLexhqUzORcxaOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4".into(),
            auth: "BTBZMqHH6r4Tts7J_aSIgg".into(),
        }
    }

    fn isolated_data_dir() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let base =
            std::env::temp_dir().join(format!("ac-push-test-{}-{}", std::process::id(), nanos));
        std::fs::create_dir_all(&base).unwrap();
        std::env::set_var("XDG_DATA_HOME", &base);
    }

    /// Two devices, one subscribed: a knock after the debounce pushes the
    /// count to the subscribed one only; a second knock inside the interval
    /// is swallowed; a knock whose queue emptied sends nothing; a dead
    /// subscription is dropped.
    #[test]
    fn knocks_are_debounced_rate_limited_and_content_free() {
        let _env = crate::test_support::lock_env();
        isolated_data_dir();
        let devices = Arc::new(PairedDevicesService::new());
        let p1 = devices.propose_pairing("phone", "k1", DeviceScope::ReadOnly, 1);
        let d1 = devices.approve_pairing(&p1, 1).unwrap();
        let p2 = devices.propose_pairing("tablet", "k2", DeviceScope::ReadOnly, 1);
        devices.approve_pairing(&p2, 1).unwrap();
        devices.set_push(&d1.id, Some(sub(1))).unwrap();
        let sender = Arc::new(FakeSender {
            calls: Mutex::new(Vec::new()),
            reply: Mutex::new(Delivery::Sent),
        });
        let notifier = Arc::new(PushNotifier::with_sender(
            devices.clone(),
            sender.clone(),
            Duration::from_millis(30),
            Duration::from_millis(200),
        ));
        notifier.set_vapid(VapidKey::generate());

        // The queue still has 3 when the debounce elapses → one push.
        notifier.knock(Box::new(|| 3));
        notifier.knock(Box::new(|| 3)); // coalesced into the scheduled one
        std::thread::sleep(Duration::from_millis(120));
        {
            let calls = sender.calls.lock().unwrap();
            assert_eq!(calls.len(), 1, "one knock, one push");
            assert_eq!(calls[0].0, "https://push.example/1");
            assert_eq!(
                String::from_utf8(calls[0].1.clone()).unwrap(),
                r#"{"pending":3,"type":"decisions"}"#
            );
        }
        // Inside the interval: swallowed.
        assert!(notifier.send_all(4).is_empty());
        // The queue emptied before the debounce: nothing.
        notifier.knock(Box::new(|| 0));
        std::thread::sleep(Duration::from_millis(80));
        assert_eq!(sender.calls.lock().unwrap().len(), 1);
        // After the interval, a dead subscription is dropped on 410.
        std::thread::sleep(Duration::from_millis(200));
        *sender.reply.lock().unwrap() = Delivery::Gone;
        assert!(notifier.send_all(1).is_empty());
        let d = devices
            .list_devices()
            .unwrap()
            .into_iter()
            .find(|d| d.id == d1.id)
            .unwrap();
        assert!(d.push.is_none(), "410 drops the subscription");
    }
}
