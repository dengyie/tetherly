// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The per-listener event hub: one pump task turns node events into
//! coalesced full-state v1 events; subscribers get replay or a snapshot.

use super::coalesce::Coalescer;
use super::journal::{Frame, Journal};
use super::model;
use super::project::{project, Key};
use crate::Node;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tetherly_api::{Device, Event, Topic};
use tokio::sync::broadcast;

/// Frames kept for `Last-Event-ID` replay.
pub const REPLAY_FRAMES: usize = 1_024;
/// Coalescing tick: the latency floor of a pushed change.
pub const FLUSH_TICK: Duration = Duration::from_millis(100);

pub struct EventHub {
    journal: Mutex<Journal>,
    tx: broadcast::Sender<Arc<Frame>>,
    /// Frozen `/api/*` route hits for `GET /api/v1/diagnostics`.
    pub legacy: super::legacy::LegacyRoutes,
}

/// A new subscriber's starting point. `replay` is `None` when the client
/// must be resynced with a snapshot taken at `current`.
pub struct Subscription {
    pub rx: broadcast::Receiver<Arc<Frame>>,
    pub current: u64,
    pub replay: Option<Vec<Arc<Frame>>>,
}

impl EventHub {
    pub fn new(base: u64) -> Arc<Self> {
        let (tx, _) = broadcast::channel(REPLAY_FRAMES);
        Arc::new(Self {
            journal: Mutex::new(Journal::new(base, REPLAY_FRAMES)),
            tx,
            legacy: Default::default(),
        })
    }

    /// A hub whose sequence starts above any earlier daemon run's, with its
    /// pump task attached to `node` until shutdown.
    pub fn spawn(node: Node) -> Arc<Self> {
        let base = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis() as u64)
            .saturating_mul(1_000);
        let hub = Self::new(base);
        tokio::spawn(pump(hub.clone(), node));
        hub
    }

    pub fn publish(&self, event: &Event) {
        let mut j = self.journal.lock().unwrap_or_else(|e| e.into_inner());
        let frame = j.push(event);
        // Sent under the lock so receivers see frames in seq order.
        let _ = self.tx.send(frame);
    }

    /// Sequence number of the newest frame.
    pub fn current(&self) -> u64 {
        self.journal
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .current()
    }

    pub fn subscribe(&self, last: Option<u64>) -> Subscription {
        let j = self.journal.lock().unwrap_or_else(|e| e.into_inner());
        Subscription {
            rx: self.tx.subscribe(),
            current: j.current(),
            replay: last.and_then(|l| j.since(l)),
        }
    }
}

/// Compare `current` against what clients were last told. `only` scopes the
/// comparison to one id (a targeted mark); `None` is a full resync.
pub fn diff_devices(
    known: &mut HashMap<String, Device>,
    current: Vec<Device>,
    only: Option<&str>,
) -> Vec<Event> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for d in current {
        if only.is_some_and(|id| id != d.id) {
            continue;
        }
        seen.insert(d.id.clone());
        if known.get(&d.id) != Some(&d) {
            known.insert(d.id.clone(), d.clone());
            out.push(Event::DeviceUpdated { device: d });
        }
    }
    let mut gone: Vec<String> = known
        .keys()
        .filter(|id| only.is_none_or(|o| o == id.as_str()) && !seen.contains(*id))
        .cloned()
        .collect();
    gone.sort();
    for id in gone {
        known.remove(&id);
        out.push(Event::DeviceRemoved { id });
    }
    out
}

/// Everything a lagged pump may have missed.
fn mark_everything(c: &mut Coalescer) {
    for key in [Key::Devices, Key::System, Key::Input, Key::Pairing] {
        c.mark(key, false);
    }
    for t in Topic::ALL {
        if !matches!(
            t,
            Topic::Devices | Topic::System | Topic::Input | Topic::Pairing
        ) {
            c.mark(Key::Invalidate(t), false);
        }
    }
}

async fn pump(hub: Arc<EventHub>, node: Node) {
    let mut events = node.subscribe();
    let mut stop = node.shutdown_signal();
    let mut known: HashMap<String, Device> = model::devices(&node)
        .into_iter()
        .map(|d| (d.id.clone(), d))
        .collect();
    let mut c = Coalescer::default();
    let start = tokio::time::Instant::now();
    let mut tick = tokio::time::interval(FLUSH_TICK);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = stop.changed() => {
                if *stop.borrow() {
                    break;
                }
            }
            res = events.recv() => match res {
                Ok(ev) => {
                    for m in project(&ev) {
                        c.mark(m.key, m.noisy);
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => mark_everything(&mut c),
                Err(broadcast::error::RecvError::Closed) => break,
            },
            _ = tick.tick() => {
                let due = c.due(start.elapsed().as_millis() as u64);
                if !due.is_empty() {
                    resolve(&hub, &node, due, &mut known).await;
                }
            }
        }
    }
}

async fn resolve(hub: &EventHub, node: &Node, keys: Vec<Key>, known: &mut HashMap<String, Device>) {
    let mut devices: Option<Vec<Device>> = None;
    for key in keys {
        match key {
            Key::Device(id) => {
                let all = devices.get_or_insert_with(|| model::devices(node)).clone();
                for ev in diff_devices(known, all, Some(&id)) {
                    hub.publish(&ev);
                }
            }
            Key::Devices => {
                let all = devices.get_or_insert_with(|| model::devices(node)).clone();
                for ev in diff_devices(known, all, None) {
                    hub.publish(&ev);
                }
            }
            Key::System => hub.publish(&Event::SystemChanged {
                system: model::system(node),
            }),
            Key::Pairing => hub.publish(&Event::PairingChanged {
                pairing: model::pairing(node),
            }),
            Key::Input => hub.publish(&Event::InputChanged {
                input: model::input(node).await,
            }),
            Key::Invalidate(topic) => match topic {
                Topic::Transfers => {
                    hub.publish(&Event::TransfersChanged {
                        transfers: model::transfers(node),
                    });
                }
                Topic::Inbox => hub.publish(&Event::InboxChanged {
                    inbox: model::inbox(node),
                }),
                Topic::Layout => hub.publish(&Event::LayoutChanged {
                    layout: model::layout(node),
                }),
                Topic::Settings => hub.publish(&Event::SettingsChanged {
                    settings: model::settings(node),
                }),
                // No pushed form yet: keep asking the client to refetch.
                _ => hub.publish(&Event::Invalidated { topic }),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(id: &str, name: &str) -> Device {
        Device {
            id: id.into(),
            name: name.into(),
            alias: None,
            platform: None,
            device_type: "unknown".into(),
            online: true,
            trusted: true,
            blocked: false,
            path: None,
            addr: None,
            rtt_ms: None,
            last_seen_ms: None,
            caps: vec![],
            features: vec![],
            tags: vec![],
        }
    }

    #[test]
    fn device_diff_emits_only_changes_and_removals() {
        let mut known = HashMap::new();
        let ev = diff_devices(&mut known, vec![dev("a", "A"), dev("b", "B")], None);
        assert_eq!(ev.len(), 2);
        // unchanged -> nothing
        assert!(diff_devices(&mut known, vec![dev("a", "A"), dev("b", "B")], None).is_empty());
        // scoped to one id: b renamed, a gone but out of scope
        let ev = diff_devices(&mut known, vec![dev("b", "B2")], Some("b"));
        assert_eq!(
            ev,
            vec![Event::DeviceUpdated {
                device: dev("b", "B2")
            }]
        );
        // scoped to a: gone -> removed exactly once
        let ev = diff_devices(&mut known, vec![dev("b", "B2")], Some("a"));
        assert_eq!(ev, vec![Event::DeviceRemoved { id: "a".into() }]);
        assert!(diff_devices(&mut known, vec![dev("b", "B2")], Some("a")).is_empty());
        // full resync removes everything not present
        let ev = diff_devices(&mut known, vec![], None);
        assert_eq!(ev, vec![Event::DeviceRemoved { id: "b".into() }]);
    }

    #[tokio::test]
    async fn subscribers_get_replay_or_snapshot_and_live_frames() {
        let hub = EventHub::new(500);
        hub.publish(&Event::Invalidated {
            topic: Topic::Inbox,
        });
        hub.publish(&Event::Invalidated {
            topic: Topic::Layout,
        });

        let fresh = hub.subscribe(None);
        assert_eq!(fresh.current, 502);
        assert!(fresh.replay.is_none());

        let resumed = hub.subscribe(Some(501));
        let seqs: Vec<u64> = resumed.replay.unwrap().iter().map(|f| f.seq).collect();
        assert_eq!(seqs, vec![502]);

        assert!(hub.subscribe(Some(7)).replay.is_none());

        let mut live = hub.subscribe(Some(502));
        assert!(live.replay.as_ref().unwrap().is_empty());
        hub.publish(&Event::Invalidated {
            topic: Topic::Settings,
        });
        let f = live.rx.recv().await.unwrap();
        assert_eq!(f.seq, 503);
        assert_eq!(f.topic, Some(Topic::Settings));
    }

    /// M2c: the four state topics resolve to full-state events, not
    /// `invalidated`; other coarse topics keep the refetch form.
    #[tokio::test]
    async fn state_topics_project_full_state_and_others_stay_invalidated() {
        let (node, dir) = crate::runtime::clipboard_policy_tests::temp_test_node("hub_m2c");
        let hub = EventHub::spawn(node.clone_handle());

        // Let the pump's initial full-state burst flush; then drain whatever
        // arrived before we start asserting (order/count is a pump detail).
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        let sub = hub.subscribe(None);
        let mut rx = sub.rx;
        while let Ok(f) = rx.try_recv() {
            let _ = f;
        }

        // A settings-source event marks System + Settings; consume frames
        // until the settings form surfaces (system.changed flushes first).
        node.update_config(Some("hub-m2c-name"), None, None)
            .unwrap();
        let env = loop {
            let f = tokio::time::timeout_at(deadline, rx.recv())
                .await
                .unwrap()
                .unwrap();
            if f.name == "settings.changed" {
                let env: tetherly_api::EventEnvelope = serde_json::from_str(&f.data).unwrap();
                break env;
            }
        };
        match env.event {
            Event::SettingsChanged { settings } => {
                assert_eq!(settings.general.name, "hub-m2c-name");
            }
            other => panic!("expected SettingsChanged, got {other:?}"),
        }

        // A transfers-source event resolves to state too.
        node.emit(crate::events::UiEvent::FileDone {
            transfer_id: "t1".into(),
        });
        let mut got = None;
        while let Ok(f) = rx.try_recv() {
            if f.name == "transfers.changed" {
                got = Some(f);
            }
        }
        if got.is_none() {
            let f = tokio::time::timeout_at(deadline, rx.recv())
                .await
                .unwrap()
                .unwrap();
            assert_eq!(f.name, "transfers.changed", "{}", f.data);
        }

        // Clipboard has no pushed form: it must still come out invalidated.
        node.emit(crate::events::UiEvent::ClipboardCleared { timestamp_ms: 1 });
        let f = tokio::time::timeout_at(deadline, rx.recv())
            .await
            .unwrap()
            .unwrap();
        let env: tetherly_api::EventEnvelope = serde_json::from_str(&f.data).unwrap();
        match env.event {
            Event::Invalidated { topic } => assert_eq!(topic, Topic::Clipboard),
            other => panic!("expected Invalidated, got {other:?}"),
        }

        drop(rx);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
