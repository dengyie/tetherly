// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::error::NetError;
use tetherly_core::frame::{
    decode_unix_ms, CapsUpdate, ClipSet, FileDecision, FileDone, FileOffer, InnerFrame,
    NotifyDismiss, NotifyPush, TYPE_CAPS_UPDATE, TYPE_CLIP_SET, TYPE_FILE_ACCEPT, TYPE_FILE_DONE,
    TYPE_FILE_OFFER, TYPE_FILE_REJECT, TYPE_NOTIFY_DISMISS, TYPE_NOTIFY_PUSH, TYPE_NOTIFY_REPLY,
    TYPE_PING, TYPE_PONG,
};

#[derive(Debug)]
pub enum SessionEvent {
    Ping(u64),
    Pong(u64),
    Notify(NotifyPush),
    Dismiss(NotifyDismiss),
    Clip(ClipSet),
    FileOffer(FileOffer),
    FileAccept(FileDecision),
    FileReject(FileDecision),
    FileDone(FileDone),
    Caps(CapsUpdate),
    ReplyStub,
    Unknown(u16),
}

impl SessionEvent {
    pub fn from_frame(frame: InnerFrame) -> Result<Self, NetError> {
        Ok(match frame.ty {
            TYPE_PING => {
                SessionEvent::Ping(decode_unix_ms(&frame.payload).ok_or(NetError::Handshake)?)
            }
            TYPE_PONG => {
                SessionEvent::Pong(decode_unix_ms(&frame.payload).ok_or(NetError::Handshake)?)
            }
            TYPE_NOTIFY_PUSH => SessionEvent::Notify(NotifyPush::from_payload(&frame.payload)?),
            TYPE_NOTIFY_DISMISS => {
                SessionEvent::Dismiss(NotifyDismiss::from_payload(&frame.payload)?)
            }
            TYPE_NOTIFY_REPLY => SessionEvent::ReplyStub,
            TYPE_CLIP_SET => SessionEvent::Clip(ClipSet::from_payload(&frame.payload)?),
            TYPE_FILE_OFFER => SessionEvent::FileOffer(FileOffer::from_payload(&frame.payload)?),
            TYPE_FILE_ACCEPT => {
                SessionEvent::FileAccept(FileDecision::from_payload(&frame.payload)?)
            }
            TYPE_FILE_REJECT => {
                SessionEvent::FileReject(FileDecision::from_payload(&frame.payload)?)
            }
            TYPE_FILE_DONE => SessionEvent::FileDone(FileDone::from_payload(&frame.payload)?),
            TYPE_CAPS_UPDATE => SessionEvent::Caps(CapsUpdate::from_payload(&frame.payload)?),
            other => SessionEvent::Unknown(other),
        })
    }
}
