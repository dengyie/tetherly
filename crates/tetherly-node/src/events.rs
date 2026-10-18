// SPDX-License-Identifier: Apache-2.0 OR MIT
//! UI-facing events. Bodies, OTP digits, and clipboard text never appear here.

use serde::{Deserialize, Serialize};
use tetherly_core::{CandidateView, DeviceId, IncomingTransfer, ReplyView};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UiEvent {
    Candidate {
        candidate: CandidateViewDto,
    },
    CandidateExpired {
        id: String,
    },
    CandidateDismissed {
        id: String,
        source: String,
        app_id: String,
        uid: String,
    },
    ReplyReceived {
        reply: ReplyViewDto,
    },
    FileOffered {
        transfer_id: String,
        source: String,
        files: Vec<FileView>,
    },
    FileRejected {
        transfer_id: String,
        source: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    FileAutoAcceptChanged {
        enabled: bool,
    },
    FileDone {
        transfer_id: String,
    },
    PeerUp {
        device_id: String,
        name: String,
        platform: String,
    },
    PeerDown {
        device_id: String,
    },
    PairingNeeded {
        device_id: String,
        fp: String,
    },
    Status {
        message: String,
    },
    Overlay {
        present: bool,
        lan_only: bool,
    },
    PeerLatency {
        device_id: String,
        rtt_ms: u64,
    },
    PinStateChanged {
        active: bool,
    },
    PeerAliasChanged {
        device_id: String,
        alias: String,
    },
    ConfigChanged {
        name: String,
        advertise: bool,
        reconnect: bool,
    },
    RouteChanged {
        device_id: String,
        addr: String,
        action: String,
    },
    MetricsReset {
        timestamp_ms: u64,
    },
    PeerCapsChanged {
        device_id: String,
        caps: Vec<String>,
        features: Vec<String>,
    },
    PeerBlocked {
        device_id: String,
        reason: Option<String>,
    },
    PeerUnblocked {
        device_id: String,
    },
    ClipboardSynced {
        seq: u64,
        source: String,
        direction: String,
        bytes: usize,
        hash: String,
        timestamp_ms: u64,
        sent_peers: usize,
    },
    ClipboardCleared {
        timestamp_ms: u64,
    },
    ScreenStateChanged {
        state: String,
        allowed: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        peer: Option<String>,
    },
    InputSeatChanged {
        seat: String,
        channel_open: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        peer: Option<String>,
    },
    NetworkRefreshed {
        iface_count: usize,
        listen_count: usize,
        overlay_present: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        overlay_source: Option<String>,
    },
    OverlayStateChanged {
        present: bool,
        source: String,
        local_ips: Vec<String>,
        peer_count: usize,
    },
}

impl UiEvent {
    pub fn screen_state_changed(
        state: impl Into<String>,
        allowed: bool,
        peer: Option<String>,
    ) -> Self {
        screen_state_changed(state, allowed, peer)
    }

    pub fn input_seat_changed(
        seat: impl Into<String>,
        channel_open: bool,
        peer: Option<String>,
    ) -> Self {
        input_seat_changed(seat, channel_open, peer)
    }

    pub fn network_refreshed(
        iface_count: usize,
        listen_count: usize,
        overlay_present: bool,
        overlay_source: Option<String>,
    ) -> Self {
        network_refreshed(iface_count, listen_count, overlay_present, overlay_source)
    }

    pub fn overlay_state_changed(
        present: bool,
        source: impl Into<String>,
        local_ips: Vec<String>,
        peer_count: usize,
    ) -> Self {
        overlay_state_changed(present, source, local_ips, peer_count)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CandidateViewDto {
    pub id: String,
    pub source: String,
    pub app_id: String,
    pub app_name: String,
    pub uid: String,
    pub title: String,
    pub has_otp: bool,
    pub created_ms: u64,
    pub expires_ms: u64,
    pub actions: Vec<String>,
}

impl From<CandidateView> for CandidateViewDto {
    fn from(v: CandidateView) -> Self {
        Self {
            id: v.id,
            source: v.source.to_string(),
            app_id: v.app_id,
            app_name: v.app_name,
            uid: v.uid,
            title: v.title,
            has_otp: v.has_otp,
            created_ms: v.created_ms,
            expires_ms: v.expires_ms,
            actions: v.actions,
        }
    }
}

/// A queued `notify.reply`, shown so the user knows a reply is waiting. The
/// text is not carried here (see `ReplyView`), matching the no-content rule.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReplyViewDto {
    pub id: String,
    pub source: String,
    pub app_id: String,
    pub uid: String,
}

impl From<ReplyView> for ReplyViewDto {
    fn from(v: ReplyView) -> Self {
        Self {
            id: v.id,
            source: v.source.to_string(),
            app_id: v.app_id,
            uid: v.uid,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileView {
    pub name: String,
    pub size: u64,
}

pub fn file_offered(t: &IncomingTransfer) -> UiEvent {
    UiEvent::FileOffered {
        transfer_id: t.transfer_id.clone(),
        source: t.source.to_string(),
        files: t
            .files
            .iter()
            .map(|f| FileView {
                name: f.name.clone(),
                size: f.size,
            })
            .collect(),
    }
}

pub fn peer_up(id: &DeviceId, name: &str, platform: &str) -> UiEvent {
    UiEvent::PeerUp {
        device_id: id.to_string(),
        name: name.to_string(),
        platform: platform.to_string(),
    }
}

pub fn screen_state_changed(
    state: impl Into<String>,
    allowed: bool,
    peer: Option<String>,
) -> UiEvent {
    UiEvent::ScreenStateChanged {
        state: state.into(),
        allowed,
        peer,
    }
}

pub fn input_seat_changed(
    seat: impl Into<String>,
    channel_open: bool,
    peer: Option<String>,
) -> UiEvent {
    UiEvent::InputSeatChanged {
        seat: seat.into(),
        channel_open,
        peer,
    }
}

pub fn network_refreshed(
    iface_count: usize,
    listen_count: usize,
    overlay_present: bool,
    overlay_source: Option<String>,
) -> UiEvent {
    UiEvent::NetworkRefreshed {
        iface_count,
        listen_count,
        overlay_present,
        overlay_source,
    }
}

pub fn overlay_state_changed(
    present: bool,
    source: impl Into<String>,
    local_ips: Vec<String>,
    peer_count: usize,
) -> UiEvent {
    UiEvent::OverlayStateChanged {
        present,
        source: source.into(),
        local_ips,
        peer_count,
    }
}
