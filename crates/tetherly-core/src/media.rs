// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Cross-device media playback control, state synchronization, and volume governance.

use crate::error::CoreError;
use crate::frame::{InnerFrame, TYPE_MEDIA_COMMAND, TYPE_MEDIA_STATE};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

pub const MAX_MEDIA_TITLE_LEN: usize = 256;
pub const MAX_MEDIA_ARTIST_LEN: usize = 128;
pub const MAX_MEDIA_ALBUM_LEN: usize = 128;
pub const MAX_MEDIA_PLAYER_ID_LEN: usize = 64;

/// Media commands supported across the device mesh.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", content = "param", rename_all = "snake_case")]
pub enum MediaCommand {
    Play,
    Pause,
    PlayPause,
    Stop,
    Next,
    Previous,
    Seek { offset_ms: i64 },
    SetVolume { volume_percent: u8 },
    ToggleMute,
}

impl MediaCommand {
    pub fn action_name(&self) -> &'static str {
        match self {
            Self::Play => "play",
            Self::Pause => "pause",
            Self::PlayPause => "play_pause",
            Self::Stop => "stop",
            Self::Next => "next",
            Self::Previous => "previous",
            Self::Seek { .. } => "seek",
            Self::SetVolume { .. } => "set_volume",
            Self::ToggleMute => "toggle_mute",
        }
    }

    pub fn as_label(&self) -> &'static str {
        self.action_name()
    }
}

impl fmt::Display for MediaCommand {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Play => write!(f, "play"),
            Self::Pause => write!(f, "pause"),
            Self::PlayPause => write!(f, "play_pause"),
            Self::Stop => write!(f, "stop"),
            Self::Next => write!(f, "next"),
            Self::Previous => write!(f, "previous"),
            Self::Seek { offset_ms } => write!(f, "seek({offset_ms}ms)"),
            Self::SetVolume { volume_percent } => write!(f, "volume({volume_percent}%)"),
            Self::ToggleMute => write!(f, "toggle_mute"),
        }
    }
}

/// Current playback state of a media player.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PlaybackState {
    #[default]
    Stopped,
    Playing,
    Paused,
}

impl PlaybackState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stopped => "stopped",
            Self::Playing => "playing",
            Self::Paused => "paused",
        }
    }
}

impl FromStr for PlaybackState {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "stopped" | "stop" => Ok(Self::Stopped),
            "playing" | "play" => Ok(Self::Playing),
            "paused" | "pause" => Ok(Self::Paused),
            other => Err(format!(
                "invalid playback state '{other}', expected: stopped, playing, paused"
            )),
        }
    }
}

impl fmt::Display for PlaybackState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Ingress policy mode for receiving remote media control commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MediaControlMode {
    #[default]
    Allow,
    PromptOnly,
    Disabled,
}

impl MediaControlMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Allow => "allow",
            Self::PromptOnly => "prompt_only",
            Self::Disabled => "disabled",
        }
    }

    pub fn is_enabled(&self) -> bool {
        matches!(self, Self::Allow | Self::PromptOnly)
    }
}

impl FromStr for MediaControlMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "allow" | "auto" => Ok(Self::Allow),
            "prompt_only" | "prompt" => Ok(Self::PromptOnly),
            "disabled" | "disable" => Ok(Self::Disabled),
            other => Err(format!(
                "invalid media control mode '{other}', expected: allow, prompt_only, disabled"
            )),
        }
    }
}

impl fmt::Display for MediaControlMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A wire frame representing a remote media control command dispatched to a peer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaCommandFrame {
    pub id: String,
    pub command: MediaCommand,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_player: Option<String>,
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl MediaCommandFrame {
    pub fn new(
        id: impl Into<String>,
        command: MediaCommand,
        target_player: Option<String>,
        timestamp_ms: u64,
    ) -> Result<Self, CoreError> {
        let id_str = id.into();
        let target_sanitized =
            target_player.map(|p| MediaValidator::sanitize_text(&p, MAX_MEDIA_PLAYER_ID_LEN));
        Ok(Self {
            id: id_str,
            command,
            target_player: target_sanitized,
            timestamp_ms,
        })
    }

    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        InnerFrame::new(TYPE_MEDIA_COMMAND, serde_json::to_vec(self)?)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        let frame: Self = serde_json::from_slice(payload)?;
        Ok(frame)
    }
}

/// A wire frame representing current playback state and metadata broadcast by a peer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaStateFrame {
    pub player_id: String,
    pub state: PlaybackState,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub album: String,
    #[serde(default)]
    pub position_ms: u64,
    #[serde(default)]
    pub duration_ms: u64,
    #[serde(default = "default_volume")]
    pub volume_percent: u8,
    #[serde(default)]
    pub muted: bool,
    #[serde(default = "default_true")]
    pub can_play: bool,
    #[serde(default = "default_true")]
    pub can_pause: bool,
    #[serde(default = "default_true")]
    pub can_go_next: bool,
    #[serde(default = "default_true")]
    pub can_go_previous: bool,
    #[serde(default = "default_true")]
    pub can_seek: bool,
    #[serde(default)]
    pub timestamp_ms: u64,
}

fn default_volume() -> u8 {
    100
}

fn default_true() -> bool {
    true
}

impl MediaStateFrame {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        player_id: impl Into<String>,
        state: PlaybackState,
        title: impl Into<String>,
        artist: impl Into<String>,
        album: impl Into<String>,
        position_ms: u64,
        duration_ms: u64,
        volume_percent: u8,
        muted: bool,
        timestamp_ms: u64,
    ) -> Self {
        let mut frame = Self {
            player_id: MediaValidator::sanitize_text(&player_id.into(), MAX_MEDIA_PLAYER_ID_LEN),
            state,
            title: MediaValidator::sanitize_text(&title.into(), MAX_MEDIA_TITLE_LEN),
            artist: MediaValidator::sanitize_text(&artist.into(), MAX_MEDIA_ARTIST_LEN),
            album: MediaValidator::sanitize_text(&album.into(), MAX_MEDIA_ALBUM_LEN),
            position_ms,
            duration_ms,
            volume_percent: MediaValidator::clamp_volume(volume_percent),
            muted,
            can_play: true,
            can_pause: true,
            can_go_next: true,
            can_go_previous: true,
            can_seek: true,
            timestamp_ms,
        };
        frame.sanitize();
        frame
    }

    pub fn sanitize(&mut self) {
        self.player_id = MediaValidator::sanitize_text(&self.player_id, MAX_MEDIA_PLAYER_ID_LEN);
        self.title = MediaValidator::sanitize_text(&self.title, MAX_MEDIA_TITLE_LEN);
        self.artist = MediaValidator::sanitize_text(&self.artist, MAX_MEDIA_ARTIST_LEN);
        self.album = MediaValidator::sanitize_text(&self.album, MAX_MEDIA_ALBUM_LEN);
        self.volume_percent = MediaValidator::clamp_volume(self.volume_percent);
    }

    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        InnerFrame::new(TYPE_MEDIA_STATE, serde_json::to_vec(self)?)
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        let mut frame: Self = serde_json::from_slice(payload)?;
        frame.sanitize();
        Ok(frame)
    }
}

impl Default for MediaStateFrame {
    fn default() -> Self {
        Self {
            player_id: "default".to_string(),
            state: PlaybackState::Stopped,
            title: String::new(),
            artist: String::new(),
            album: String::new(),
            position_ms: 0,
            duration_ms: 0,
            volume_percent: 70,
            muted: false,
            can_play: true,
            can_pause: false,
            can_go_next: true,
            can_go_previous: true,
            can_seek: true,
            timestamp_ms: 0,
        }
    }
}

/// Zero-dependency validator for media titles, metadata, and volume limits.
pub struct MediaValidator;

impl MediaValidator {
    /// Sanitizes text by stripping ASCII control characters (`0x00..=0x1F`, `0x7F`)
    /// and truncating at UTF-8 char boundaries to `max_len` bytes.
    pub fn sanitize_text(raw: &str, max_len: usize) -> String {
        let mut cleaned = String::with_capacity(raw.len().min(max_len));
        for c in raw.chars() {
            if !c.is_control() {
                cleaned.push(c);
            }
        }
        let trimmed = cleaned.trim();
        if trimmed.len() <= max_len {
            trimmed.to_string()
        } else {
            let mut end = max_len;
            while end > 0 && !trimmed.is_char_boundary(end) {
                end -= 1;
            }
            trimmed[..end].to_string()
        }
    }

    /// Clamps volume to valid range `0..=100`.
    #[inline]
    pub fn clamp_volume(volume: u8) -> u8 {
        volume.min(100)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_media_command_serialization_and_display() {
        let cmd = MediaCommand::SetVolume { volume_percent: 75 };
        assert_eq!(cmd.action_name(), "set_volume");
        assert_eq!(format!("{cmd}"), "volume(75%)");

        let json = serde_json::to_string(&cmd).unwrap();
        let decoded: MediaCommand = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, cmd);
    }

    #[test]
    fn test_playback_state_and_mode_parsing() {
        assert_eq!(
            PlaybackState::from_str("playing").unwrap(),
            PlaybackState::Playing
        );
        assert_eq!(
            PlaybackState::from_str("PAUSE").unwrap(),
            PlaybackState::Paused
        );
        assert_eq!(
            PlaybackState::from_str("stopped").unwrap(),
            PlaybackState::Stopped
        );

        assert_eq!(
            MediaControlMode::from_str("allow").unwrap(),
            MediaControlMode::Allow
        );
        assert_eq!(
            MediaControlMode::from_str("prompt_only").unwrap(),
            MediaControlMode::PromptOnly
        );
        assert_eq!(
            MediaControlMode::from_str("disabled").unwrap(),
            MediaControlMode::Disabled
        );
    }

    #[test]
    fn test_media_validator_and_sanitization() {
        let dirty = "Hello\r\n\tWorld\0 with \u{001F}control";
        let cleaned = MediaValidator::sanitize_text(dirty, 50);
        assert_eq!(cleaned, "HelloWorld with control");

        assert_eq!(MediaValidator::clamp_volume(150), 100);
        assert_eq!(MediaValidator::clamp_volume(85), 85);
    }

    #[test]
    fn test_media_wire_frames_roundtrip() {
        let cmd_frame = MediaCommandFrame::new(
            "cmd_123",
            MediaCommand::PlayPause,
            Some("spotify".into()),
            1_760_000_000,
        )
        .unwrap();
        let wire = cmd_frame.to_frame().unwrap();
        assert_eq!(wire.ty, TYPE_MEDIA_COMMAND);
        let dec_cmd = MediaCommandFrame::from_payload(&wire.payload).unwrap();
        assert_eq!(dec_cmd, cmd_frame);

        let state_frame = MediaStateFrame::new(
            "mpv",
            PlaybackState::Playing,
            "Track 1\n",
            "Artist A",
            "Album B",
            120_000,
            240_000,
            120, // should clamp to 100
            false,
            1_760_000_000,
        );
        assert_eq!(state_frame.volume_percent, 100);
        assert_eq!(state_frame.title, "Track 1");

        let wire_state = state_frame.to_frame().unwrap();
        assert_eq!(wire_state.ty, TYPE_MEDIA_STATE);
        let dec_state = MediaStateFrame::from_payload(&wire_state.payload).unwrap();
        assert_eq!(dec_state.volume_percent, 100);
        assert_eq!(dec_state.title, "Track 1");
    }
}
