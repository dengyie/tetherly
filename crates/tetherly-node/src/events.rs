// SPDX-License-Identifier: Apache-2.0 OR MIT
//! UI-facing events. Bodies, OTP digits, and clipboard text never appear here.

use serde::Serialize;
use tetherly_core::{CandidateView, DeviceId, IncomingTransfer};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UiEvent {
    Candidate {
        candidate: CandidateViewDto,
    },
    CandidateExpired {
        id: String,
    },
    FileOffered {
        transfer_id: String,
        source: String,
        files: Vec<FileView>,
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
}

#[derive(Debug, Clone, Serialize)]
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
        }
    }
}

#[derive(Debug, Clone, Serialize)]
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
