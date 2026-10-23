// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::error::NetError;
use tetherly_core::action::{
    ActionListReqFrame, ActionListRespFrame, ActionResultFrame, ActionTriggerFrame,
};
use tetherly_core::audio_routing::{
    AudioRoutingAnnounceFrame, AudioRoutingAttachFrame, AudioRoutingChunkFrame,
    AudioRoutingDetachFrame,
};
use tetherly_core::battery::BatteryStateFrame;
use tetherly_core::camera::{
    CameraAnnounceFrame, CameraAttachFrame, CameraDetachFrame, CameraFrameChunk,
};
use tetherly_core::focus::{FocusStateFrame, FocusSyncReqFrame};
use tetherly_core::frame::{
    decode_unix_ms, CapsUpdate, ClipSet, FileDecision, FileDone, FileOffer, InnerFrame,
    NotifyDismiss, NotifyPush, NotifyReply, ResourceHandoffFrame, TextMessage,
    TYPE_ACTION_LIST_REQ, TYPE_ACTION_LIST_RESP, TYPE_ACTION_RESULT, TYPE_ACTION_TRIGGER,
    TYPE_AUDIO_ROUTING_ANNOUNCE, TYPE_AUDIO_ROUTING_ATTACH, TYPE_AUDIO_ROUTING_CHUNK,
    TYPE_AUDIO_ROUTING_DETACH, TYPE_BATTERY_STATE, TYPE_CAMERA_ANNOUNCE, TYPE_CAMERA_ATTACH,
    TYPE_CAMERA_DETACH, TYPE_CAMERA_FRAME_CHUNK, TYPE_CAPS_UPDATE, TYPE_CLIP_SET, TYPE_FILE_ACCEPT,
    TYPE_FILE_DONE, TYPE_FILE_OFFER, TYPE_FILE_REJECT, TYPE_FOCUS_STATE, TYPE_FOCUS_SYNC_REQ,
    TYPE_INPUT_TUNNEL_HEARTBEAT, TYPE_INPUT_TUNNEL_RELEASE, TYPE_INPUT_TUNNEL_REQUEST,
    TYPE_INPUT_TUNNEL_RESP, TYPE_LOCATE_REQUEST, TYPE_LOCATE_STOP, TYPE_MEDIA_COMMAND,
    TYPE_MEDIA_STATE, TYPE_NOTIFY_DISMISS, TYPE_NOTIFY_PUSH, TYPE_NOTIFY_REPLY, TYPE_PING,
    TYPE_PONG, TYPE_RESOURCE_HANDOFF, TYPE_TEXT_MESSAGE, TYPE_TOPOLOGY_STATE,
    TYPE_TOPOLOGY_SYNC_REQ, TYPE_VIRTUAL_DISPLAY_ANNOUNCE, TYPE_VIRTUAL_DISPLAY_ATTACH,
    TYPE_VIRTUAL_DISPLAY_DETACH, TYPE_VIRTUAL_DISPLAY_FRAME_ACK,
};
use tetherly_core::input_tunnel::{
    InputTunnelHeartbeatFrame, InputTunnelReleaseFrame, InputTunnelRequestFrame,
    InputTunnelRespFrame,
};
use tetherly_core::locate::{LocateRequestFrame, LocateStopFrame};
use tetherly_core::media::{MediaCommandFrame, MediaStateFrame};
use tetherly_core::topology::{TopologyStateFrame, TopologySyncReqFrame};
use tetherly_core::virtual_display::{
    VirtualDisplayAnnounceFrame, VirtualDisplayAttachFrame, VirtualDisplayDetachFrame,
    VirtualDisplayFrameAck,
};

#[derive(Debug)]
pub enum SessionEvent {
    Ping(u64),
    Pong(u64),
    Notify(NotifyPush),
    Dismiss(NotifyDismiss),
    Reply(NotifyReply),
    Clip(ClipSet),
    TextMessage(TextMessage),
    ResourceHandoff(ResourceHandoffFrame),
    MediaCommand(MediaCommandFrame),
    MediaState(MediaStateFrame),
    BatteryState(BatteryStateFrame),
    LocateRequest(LocateRequestFrame),
    LocateStop(LocateStopFrame),
    ActionListReq(ActionListReqFrame),
    ActionListResp(ActionListRespFrame),
    ActionTrigger(ActionTriggerFrame),
    ActionResult(ActionResultFrame),
    FocusState(FocusStateFrame),
    FocusSyncReq(FocusSyncReqFrame),
    TopologyState(TopologyStateFrame),
    TopologySyncReq(TopologySyncReqFrame),
    InputTunnelRequest(InputTunnelRequestFrame),
    InputTunnelResp(InputTunnelRespFrame),
    InputTunnelRelease(InputTunnelReleaseFrame),
    InputTunnelHeartbeat(InputTunnelHeartbeatFrame),
    VirtualDisplayAnnounce(VirtualDisplayAnnounceFrame),
    VirtualDisplayAttach(VirtualDisplayAttachFrame),
    VirtualDisplayDetach(VirtualDisplayDetachFrame),
    VirtualDisplayFrameAck(VirtualDisplayFrameAck),
    AudioRoutingAnnounce(AudioRoutingAnnounceFrame),
    AudioRoutingAttach(AudioRoutingAttachFrame),
    AudioRoutingDetach(AudioRoutingDetachFrame),
    AudioRoutingChunk(AudioRoutingChunkFrame),
    CameraAnnounce(CameraAnnounceFrame),
    CameraAttach(CameraAttachFrame),
    CameraDetach(CameraDetachFrame),
    CameraFrameChunk(CameraFrameChunk),
    FileOffer(FileOffer),
    FileAccept(FileDecision),
    FileReject(FileDecision),
    FileDone(FileDone),
    Caps(CapsUpdate),
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
            TYPE_NOTIFY_REPLY => SessionEvent::Reply(NotifyReply::from_payload(&frame.payload)?),
            TYPE_CLIP_SET => SessionEvent::Clip(ClipSet::from_payload(&frame.payload)?),
            TYPE_TEXT_MESSAGE => {
                SessionEvent::TextMessage(TextMessage::from_payload(&frame.payload)?)
            }
            TYPE_RESOURCE_HANDOFF => {
                SessionEvent::ResourceHandoff(ResourceHandoffFrame::from_payload(&frame.payload)?)
            }
            TYPE_MEDIA_COMMAND => {
                SessionEvent::MediaCommand(MediaCommandFrame::from_payload(&frame.payload)?)
            }
            TYPE_MEDIA_STATE => {
                SessionEvent::MediaState(MediaStateFrame::from_payload(&frame.payload)?)
            }
            TYPE_BATTERY_STATE => {
                SessionEvent::BatteryState(BatteryStateFrame::from_payload(&frame.payload)?)
            }
            TYPE_LOCATE_REQUEST => {
                SessionEvent::LocateRequest(LocateRequestFrame::from_payload(&frame.payload)?)
            }
            TYPE_LOCATE_STOP => {
                SessionEvent::LocateStop(LocateStopFrame::from_payload(&frame.payload)?)
            }
            TYPE_ACTION_LIST_REQ => {
                SessionEvent::ActionListReq(ActionListReqFrame::from_payload(&frame.payload)?)
            }
            TYPE_ACTION_LIST_RESP => {
                SessionEvent::ActionListResp(ActionListRespFrame::from_payload(&frame.payload)?)
            }
            TYPE_ACTION_TRIGGER => {
                SessionEvent::ActionTrigger(ActionTriggerFrame::from_payload(&frame.payload)?)
            }
            TYPE_ACTION_RESULT => {
                SessionEvent::ActionResult(ActionResultFrame::from_payload(&frame.payload)?)
            }
            TYPE_FOCUS_STATE => {
                SessionEvent::FocusState(FocusStateFrame::from_payload(&frame.payload)?)
            }
            TYPE_FOCUS_SYNC_REQ => {
                SessionEvent::FocusSyncReq(FocusSyncReqFrame::from_payload(&frame.payload)?)
            }
            TYPE_TOPOLOGY_STATE => {
                SessionEvent::TopologyState(TopologyStateFrame::from_payload(&frame.payload)?)
            }
            TYPE_TOPOLOGY_SYNC_REQ => {
                SessionEvent::TopologySyncReq(TopologySyncReqFrame::from_payload(&frame.payload)?)
            }
            TYPE_INPUT_TUNNEL_REQUEST => SessionEvent::InputTunnelRequest(
                InputTunnelRequestFrame::from_payload(&frame.payload)?,
            ),
            TYPE_INPUT_TUNNEL_RESP => {
                SessionEvent::InputTunnelResp(InputTunnelRespFrame::from_payload(&frame.payload)?)
            }
            TYPE_INPUT_TUNNEL_RELEASE => SessionEvent::InputTunnelRelease(
                InputTunnelReleaseFrame::from_payload(&frame.payload)?,
            ),
            TYPE_INPUT_TUNNEL_HEARTBEAT => SessionEvent::InputTunnelHeartbeat(
                InputTunnelHeartbeatFrame::from_payload(&frame.payload)?,
            ),
            TYPE_VIRTUAL_DISPLAY_ANNOUNCE => SessionEvent::VirtualDisplayAnnounce(
                VirtualDisplayAnnounceFrame::from_payload(&frame.payload)?,
            ),
            TYPE_VIRTUAL_DISPLAY_ATTACH => SessionEvent::VirtualDisplayAttach(
                VirtualDisplayAttachFrame::from_payload(&frame.payload)?,
            ),
            TYPE_VIRTUAL_DISPLAY_DETACH => SessionEvent::VirtualDisplayDetach(
                VirtualDisplayDetachFrame::from_payload(&frame.payload)?,
            ),
            TYPE_VIRTUAL_DISPLAY_FRAME_ACK => SessionEvent::VirtualDisplayFrameAck(
                VirtualDisplayFrameAck::from_payload(&frame.payload)?,
            ),
            TYPE_AUDIO_ROUTING_ANNOUNCE => SessionEvent::AudioRoutingAnnounce(
                AudioRoutingAnnounceFrame::from_payload(&frame.payload)?,
            ),
            TYPE_AUDIO_ROUTING_ATTACH => SessionEvent::AudioRoutingAttach(
                AudioRoutingAttachFrame::from_payload(&frame.payload)?,
            ),
            TYPE_AUDIO_ROUTING_DETACH => SessionEvent::AudioRoutingDetach(
                AudioRoutingDetachFrame::from_payload(&frame.payload)?,
            ),
            TYPE_AUDIO_ROUTING_CHUNK => SessionEvent::AudioRoutingChunk(
                AudioRoutingChunkFrame::from_payload(&frame.payload)?,
            ),
            TYPE_CAMERA_ANNOUNCE => {
                SessionEvent::CameraAnnounce(CameraAnnounceFrame::from_payload(&frame.payload)?)
            }
            TYPE_CAMERA_ATTACH => {
                SessionEvent::CameraAttach(CameraAttachFrame::from_payload(&frame.payload)?)
            }
            TYPE_CAMERA_DETACH => {
                SessionEvent::CameraDetach(CameraDetachFrame::from_payload(&frame.payload)?)
            }
            TYPE_CAMERA_FRAME_CHUNK => {
                SessionEvent::CameraFrameChunk(CameraFrameChunk::from_payload(&frame.payload)?)
            }
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
