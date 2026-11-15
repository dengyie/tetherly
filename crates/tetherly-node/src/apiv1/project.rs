// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Pure projection of internal [`UiEvent`]s onto "which v1 resource is now
//! stale" marks. The hub resolves marks into full-state v1 events, so this
//! module never touches `Node` and is exhaustively unit-testable.

use crate::events::UiEvent;
use tetherly_api::Topic;

/// A v1 resource whose pushed state may be stale.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Key {
    Device(String),
    /// Every device (resync after the node event channel lagged).
    Devices,
    System,
    Input,
    Pairing,
    Invalidate(Topic),
}

/// One staleness mark. `noisy` marks (latency, link quality, interface
/// rescans) are throttled; any other mark flushes on the next tick.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mark {
    pub key: Key,
    pub noisy: bool,
}

impl Mark {
    pub fn new(key: Key, noisy: bool) -> Self {
        Self { key, noisy }
    }
}

fn one(key: Key) -> Vec<Mark> {
    vec![Mark::new(key, false)]
}

fn invalidate(topic: Topic) -> Vec<Mark> {
    one(Key::Invalidate(topic))
}

fn device(id: &str) -> Vec<Mark> {
    one(Key::Device(id.to_string()))
}

/// Device whose online/blocked state changed: the device plus the system
/// peer counts.
fn device_and_counts(id: &str) -> Vec<Mark> {
    vec![
        Mark::new(Key::Device(id.to_string()), false),
        Mark::new(Key::System, false),
    ]
}

/// Map one internal event to the v1 resources it may have changed.
/// Exhaustive on purpose: a new [`UiEvent`] must decide its v1 exposure.
pub fn project(ev: &UiEvent) -> Vec<Mark> {
    use UiEvent as E;
    match ev {
        E::PeerUp { device_id, .. }
        | E::PeerDown { device_id }
        | E::PeerBlocked { device_id, .. }
        | E::PeerUnblocked { device_id } => device_and_counts(device_id),
        E::PeerAliasChanged { device_id, .. }
        | E::PeerCapsChanged { device_id, .. }
        | E::PeerDiscovered { device_id, .. }
        | E::PeerExpired { device_id }
        | E::PeerProfileUpdated { device_id, .. }
        | E::RouteChanged { device_id, .. } => device(device_id),
        E::PeerLatency { device_id, .. } | E::PeerLinkQualityRefreshed { device_id, .. } => {
            vec![Mark::new(Key::Device(device_id.clone()), true)]
        }
        E::PinStateChanged { .. } => one(Key::Pairing),
        E::InputSeatChanged { .. }
        | E::InputTunnelSessionStarted { .. }
        | E::InputTunnelSessionClosed { .. } => one(Key::Input),
        E::Overlay { .. } | E::OverlayStateChanged { .. } => one(Key::System),
        E::NetworkRefreshed { .. } => vec![Mark::new(Key::System, true)],
        E::ConfigChanged { .. } => vec![
            Mark::new(Key::System, false),
            Mark::new(Key::Invalidate(Topic::Settings), false),
        ],
        E::TopologyChanged { .. }
        | E::PeerTopologyUpdated { .. }
        | E::TopologyPolicyChanged { .. } => invalidate(Topic::Layout),
        E::FileOffered { .. }
        | E::FileRejected { .. }
        | E::FileDone { .. }
        | E::MultiDispatchCompleted { .. }
        | E::OutboxUpdated { .. }
        | E::OutboxCleared { .. } => invalidate(Topic::Transfers),
        E::InboxFileDeleted { .. }
        | E::InboxCleared { .. }
        | E::InboxFileOpened { .. }
        | E::HandoffInboxUpdated { .. } => invalidate(Topic::Inbox),
        E::Candidate { .. }
        | E::CandidateExpired { .. }
        | E::CandidateDismissed { .. }
        | E::ReplyReceived { .. }
        | E::MessageReceived { .. }
        | E::MessageSent { .. }
        | E::ResourceHandoffReceived { .. }
        | E::ResourceHandoffDispatched { .. }
        | E::NotificationBannerSent { .. } => invalidate(Topic::Activity),
        E::ClipboardSynced { .. } | E::ClipboardCleared { .. } | E::ClipboardSyncBlocked { .. } => {
            invalidate(Topic::Clipboard)
        }
        E::FileAutoAcceptChanged { .. }
        | E::AncsRuleChanged { .. }
        | E::NotifyConfigChanged { .. }
        | E::HeartbeatConfigChanged { .. }
        | E::AutoAcceptPolicyChanged { .. }
        | E::ClipboardPolicyChanged { .. }
        | E::HandoffPolicyChanged { .. }
        | E::MediaPolicyChanged { .. }
        | E::BatteryPolicyChanged { .. }
        | E::LocatePolicyChanged { .. }
        | E::ActionPolicyChanged { .. }
        | E::LocalActionsChanged { .. }
        | E::FocusPolicyChanged { .. }
        | E::InputTunnelPolicyChanged { .. }
        | E::VirtualDisplayPolicyChanged { .. }
        | E::AudioRoutingPolicyChanged { .. }
        | E::CameraPolicyChanged { .. }
        | E::MicrophonePolicyChanged { .. }
        | E::BenchPolicyChanged { .. } => invalidate(Topic::Settings),
        // Diagnostics, transient sessions and feature telemetry: not part of
        // the v1 resource model (yet). Legacy `/api/events` still carries them.
        E::PairingNeeded { .. }
        | E::Status { .. }
        | E::MetricsReset { .. }
        | E::ScreenStateChanged { .. }
        | E::AncsStateChanged { .. }
        | E::SystemMetricsRefreshed { .. }
        | E::AuditEventRecorded { .. }
        | E::MediaCommandReceived { .. }
        | E::MediaCommandDispatched { .. }
        | E::MediaStateChanged { .. }
        | E::BatteryStateChanged { .. }
        | E::BatteryLowAlert { .. }
        | E::LocateRingTriggered { .. }
        | E::LocateRingStopped { .. }
        | E::ActionListRequested { .. }
        | E::ActionListReceived { .. }
        | E::ActionTriggered { .. }
        | E::ActionResultReceived { .. }
        | E::FocusStateChanged { .. }
        | E::PeerFocusStateChanged { .. }
        | E::CursorEdgeCrossed { .. }
        | E::VirtualDisplaySessionStarted { .. }
        | E::VirtualDisplaySessionStopped { .. }
        | E::AudioRoutingSessionStarted { .. }
        | E::AudioRoutingSessionStopped { .. }
        | E::CameraSessionStarted { .. }
        | E::CameraSessionStopped { .. }
        | E::MicrophoneSessionStarted { .. }
        | E::MicrophoneSessionStopped { .. }
        | E::BenchSessionStarted { .. }
        | E::BenchSessionFinished { .. }
        | E::BandwidthProfileUpdated { .. } => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn marks(ev: UiEvent) -> Vec<Mark> {
        project(&ev)
    }

    #[test]
    fn peer_lifecycle_marks_the_device_and_system_counts() {
        let m = marks(UiEvent::PeerUp {
            device_id: "tdev_a".into(),
            name: "a".into(),
            platform: "macos".into(),
        });
        assert!(m.contains(&Mark::new(Key::Device("tdev_a".into()), false)));
        assert!(m.contains(&Mark::new(Key::System, false)));
        let m = marks(UiEvent::PeerDown {
            device_id: "tdev_a".into(),
        });
        assert!(m.contains(&Mark::new(Key::Device("tdev_a".into()), false)));
    }

    #[test]
    fn latency_marks_are_noisy() {
        assert_eq!(
            marks(UiEvent::PeerLatency {
                device_id: "d".into(),
                rtt_ms: 3
            }),
            vec![Mark::new(Key::Device("d".into()), true)]
        );
    }

    #[test]
    fn alias_caps_block_and_discovery_mark_the_device() {
        for ev in [
            UiEvent::PeerAliasChanged {
                device_id: "d".into(),
                alias: "x".into(),
            },
            UiEvent::PeerCapsChanged {
                device_id: "d".into(),
                caps: vec![],
                features: vec![],
            },
            UiEvent::PeerBlocked {
                device_id: "d".into(),
                reason: None,
            },
            UiEvent::PeerUnblocked {
                device_id: "d".into(),
            },
            UiEvent::PeerExpired {
                device_id: "d".into(),
            },
            UiEvent::RouteChanged {
                device_id: "d".into(),
                addr: "a".into(),
                action: "x".into(),
            },
        ] {
            assert!(
                project(&ev).contains(&Mark::new(Key::Device("d".into()), false)),
                "{ev:?}"
            );
        }
    }

    #[test]
    fn pin_input_and_overlay_mark_their_resources() {
        assert_eq!(
            marks(UiEvent::PinStateChanged { active: true }),
            vec![Mark::new(Key::Pairing, false)]
        );
        assert_eq!(
            marks(UiEvent::InputSeatChanged {
                seat: "remote".into(),
                channel_open: true,
                peer: None
            }),
            vec![Mark::new(Key::Input, false)]
        );
        assert_eq!(
            marks(UiEvent::Overlay {
                present: true,
                lan_only: false
            }),
            vec![Mark::new(Key::System, false)]
        );
    }

    #[test]
    fn coarse_resources_are_invalidated() {
        assert_eq!(
            marks(UiEvent::FileDone {
                transfer_id: "t".into()
            }),
            vec![Mark::new(Key::Invalidate(Topic::Transfers), false)]
        );
        assert_eq!(
            marks(UiEvent::InboxCleared {
                deleted_count: 1,
                freed_bytes: 1,
                timestamp_ms: 1
            }),
            vec![Mark::new(Key::Invalidate(Topic::Inbox), false)]
        );
        assert_eq!(
            marks(UiEvent::ClipboardCleared { timestamp_ms: 1 }),
            vec![Mark::new(Key::Invalidate(Topic::Clipboard), false)]
        );
        assert_eq!(
            marks(UiEvent::CandidateExpired { id: "c".into() }),
            vec![Mark::new(Key::Invalidate(Topic::Activity), false)]
        );
        assert_eq!(
            marks(UiEvent::FileAutoAcceptChanged { enabled: true }),
            vec![Mark::new(Key::Invalidate(Topic::Settings), false)]
        );
    }

    #[test]
    fn diagnostics_only_events_are_not_exposed() {
        assert!(marks(UiEvent::Status {
            message: "x".into()
        })
        .is_empty());
        assert!(marks(UiEvent::MetricsReset { timestamp_ms: 1 }).is_empty());
    }
}
