// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Cross-device battery state, power status synchronization, and low-power alerting.

use crate::error::CoreError;
use crate::frame::{InnerFrame, TYPE_BATTERY_STATE};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;

/// Power source type for a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PowerSource {
    #[default]
    Battery,
    Ac,
    Wireless,
    Unknown,
}

impl PowerSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Battery => "battery",
            Self::Ac => "ac",
            Self::Wireless => "wireless",
            Self::Unknown => "unknown",
        }
    }
}

impl FromStr for PowerSource {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "battery" | "bat" => Ok(Self::Battery),
            "ac" | "plugged" | "wall" | "charger" => Ok(Self::Ac),
            "wireless" | "qi" | "magsafe" => Ok(Self::Wireless),
            "unknown" => Ok(Self::Unknown),
            other => Err(format!(
                "invalid power source '{other}', expected: battery, ac, wireless, unknown"
            )),
        }
    }
}

impl fmt::Display for PowerSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Cross-device battery telemetry payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct BatteryStateFrame {
    /// Battery level percentage: 0..=100.
    pub battery_level: u8,
    /// Whether the device is actively charging.
    pub is_charging: bool,
    /// Power supply source.
    pub power_source: PowerSource,
    /// Whether the OS low-power / battery-saver mode is active.
    pub battery_saver: bool,
    /// Optional maximum capacity health percentage: 0..=100.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health_percent: Option<u8>,
    /// Milliseconds since UNIX epoch when this measurement was taken.
    #[serde(default)]
    pub timestamp_ms: u64,
}

impl BatteryStateFrame {
    pub fn new(
        battery_level: u8,
        is_charging: bool,
        power_source: PowerSource,
        battery_saver: bool,
        health_percent: Option<u8>,
        timestamp_ms: u64,
    ) -> Self {
        Self {
            battery_level: BatteryValidator::clamp_level(battery_level),
            is_charging,
            power_source,
            battery_saver,
            health_percent: BatteryValidator::clamp_health(health_percent),
            timestamp_ms,
        }
    }

    /// Evaluates if the battery is below the low-battery threshold and not charging.
    pub fn is_low(&self, threshold: u8) -> bool {
        !self.is_charging && self.battery_level <= threshold
    }

    /// Evaluates if the battery is below the critical battery threshold and not charging.
    pub fn is_critical(&self, threshold: u8) -> bool {
        !self.is_charging && self.battery_level <= threshold
    }

    pub fn sanitize(&mut self) {
        self.battery_level = BatteryValidator::clamp_level(self.battery_level);
        self.health_percent = BatteryValidator::clamp_health(self.health_percent);
    }

    pub fn encode(&self) -> Result<Vec<u8>, CoreError> {
        serde_json::to_vec(self).map_err(|e| CoreError::InvalidPayload(e.to_string()))
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, CoreError> {
        let mut frame: Self =
            serde_json::from_slice(bytes).map_err(|e| CoreError::InvalidPayload(e.to_string()))?;
        frame.sanitize();
        Ok(frame)
    }

    pub fn to_frame(&self) -> Result<InnerFrame, CoreError> {
        self.to_inner_frame()
    }

    pub fn from_payload(payload: &[u8]) -> Result<Self, CoreError> {
        Self::decode(payload)
    }

    pub fn to_inner_frame(&self) -> Result<InnerFrame, CoreError> {
        let payload = self.encode()?;
        InnerFrame::new(TYPE_BATTERY_STATE, payload)
    }

    pub fn from_inner_frame(frame: &InnerFrame) -> Result<Self, CoreError> {
        if frame.ty != TYPE_BATTERY_STATE {
            return Err(CoreError::InvalidPayload(format!(
                "expected frame type {TYPE_BATTERY_STATE:#06x}, got {:#06x}",
                frame.ty
            )));
        }
        Self::decode(&frame.payload)
    }
}

/// Zero-dependency validator for battery metrics.
pub struct BatteryValidator;

impl BatteryValidator {
    /// Clamps battery percentage to [0, 100].
    pub fn clamp_level(level: u8) -> u8 {
        level.min(100)
    }

    /// Clamps health percentage to [0, 100].
    pub fn clamp_health(health: Option<u8>) -> Option<u8> {
        health.map(|h| h.min(100))
    }

    /// Sanitizes and clamps all fields of a battery state frame.
    pub fn sanitize(mut frame: BatteryStateFrame) -> BatteryStateFrame {
        frame.battery_level = Self::clamp_level(frame.battery_level);
        frame.health_percent = Self::clamp_health(frame.health_percent);
        frame
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_power_source_parse_and_display() {
        assert_eq!(PowerSource::Battery.as_str(), "battery");
        assert_eq!(PowerSource::Ac.as_str(), "ac");
        assert_eq!(PowerSource::Wireless.as_str(), "wireless");
        assert_eq!(PowerSource::Unknown.as_str(), "unknown");

        assert_eq!(
            "battery".parse::<PowerSource>().unwrap(),
            PowerSource::Battery
        );
        assert_eq!("bat".parse::<PowerSource>().unwrap(), PowerSource::Battery);
        assert_eq!("ac".parse::<PowerSource>().unwrap(), PowerSource::Ac);
        assert_eq!("plugged".parse::<PowerSource>().unwrap(), PowerSource::Ac);
        assert_eq!(
            "wireless".parse::<PowerSource>().unwrap(),
            PowerSource::Wireless
        );
        assert_eq!(
            "unknown".parse::<PowerSource>().unwrap(),
            PowerSource::Unknown
        );
        assert!("invalid_source".parse::<PowerSource>().is_err());
    }

    #[test]
    fn test_battery_validator_clamping() {
        assert_eq!(BatteryValidator::clamp_level(85), 85);
        assert_eq!(BatteryValidator::clamp_level(100), 100);
        assert_eq!(BatteryValidator::clamp_level(105), 100);
        assert_eq!(BatteryValidator::clamp_level(255), 100);

        assert_eq!(BatteryValidator::clamp_health(Some(92)), Some(92));
        assert_eq!(BatteryValidator::clamp_health(Some(120)), Some(100));
        assert_eq!(BatteryValidator::clamp_health(None), None);
    }

    #[test]
    fn test_battery_frame_wire_roundtrip() {
        let frame =
            BatteryStateFrame::new(82, true, PowerSource::Ac, false, Some(98), 1700000000000);

        let inner = frame.to_inner_frame().unwrap();
        assert_eq!(inner.ty, TYPE_BATTERY_STATE);

        let decoded = BatteryStateFrame::from_inner_frame(&inner).unwrap();
        assert_eq!(frame, decoded);
    }

    #[test]
    fn test_low_and_critical_detection() {
        let charging_low = BatteryStateFrame::new(15, true, PowerSource::Ac, false, None, 1000);
        assert!(!charging_low.is_low(20));
        assert!(!charging_low.is_critical(10));

        let discharging_low =
            BatteryStateFrame::new(15, false, PowerSource::Battery, true, None, 1000);
        assert!(discharging_low.is_low(20));
        assert!(!discharging_low.is_critical(10));

        let discharging_crit =
            BatteryStateFrame::new(8, false, PowerSource::Battery, true, None, 1000);
        assert!(discharging_crit.is_low(20));
        assert!(discharging_crit.is_critical(10));
    }
}
