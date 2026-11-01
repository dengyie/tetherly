// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Battery state updates and power policy.

use super::*;

pub(super) async fn get_battery_state(
    _method: &str,
    query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let peer = qmap
        .get("peer")
        .or_else(|| qmap.get("target"))
        .map(String::as_str);
    match node.battery_get_state(peer) {
        Ok(report) => json_ok(&report),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn post_battery_state(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<BatteryStateUpdateBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<BatteryStateUpdateBody>(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json, expected BatteryStateUpdateBody"),
        }
    };
    let mut current = match node.battery_get_state(None) {
        Ok(rep) => rep.state.unwrap_or_default(),
        Err(e) => return err(400, &e.to_string()),
    };

    if let Some(ref b) = parsed_body {
        if let Some(lvl) = b.battery_level {
            current.battery_level = lvl.min(100);
        }
        if let Some(chg) = b.is_charging {
            current.is_charging = chg;
        }
        if let Some(ref src_str) = b.power_source {
            match src_str.parse::<PowerSource>() {
                Ok(ps) => current.power_source = ps,
                Err(e) => return err(400, &e),
            }
        }
        if let Some(saver) = b.battery_saver {
            current.battery_saver = saver;
        }
        if let Some(hp) = b.health_percent {
            current.health_percent = Some(hp.min(100));
        }
    }

    if let Some(lvl_str) = qmap
        .get("level")
        .or_else(|| qmap.get("battery_level"))
        .or_else(|| qmap.get("percent"))
    {
        if let Ok(lvl) = lvl_str.parse::<u8>() {
            current.battery_level = lvl.min(100);
        }
    }
    if let Some(chg_str) = qmap.get("charging").or_else(|| qmap.get("is_charging")) {
        if let Ok(chg) = chg_str.parse::<bool>() {
            current.is_charging = chg;
        }
    }
    if let Some(src_str) = qmap.get("source").or_else(|| qmap.get("power_source")) {
        match src_str.parse::<PowerSource>() {
            Ok(ps) => current.power_source = ps,
            Err(e) => return err(400, &e),
        }
    }
    if let Some(saver_str) = qmap.get("saver").or_else(|| qmap.get("battery_saver")) {
        if let Ok(saver) = saver_str.parse::<bool>() {
            current.battery_saver = saver;
        }
    }
    if let Some(health_str) = qmap.get("health").or_else(|| qmap.get("health_percent")) {
        if let Ok(hp) = health_str.parse::<u8>() {
            current.health_percent = Some(hp.min(100));
        }
    }

    match node.battery_update_local_state(current).await {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}

pub(super) async fn get_battery_policy(
    _method: &str,
    _query_str: Option<&str>,
    _body: &[u8],
    node: &Node,
) -> UiResponse {
    json_ok(&node.battery_policy_report())
}

pub(super) async fn post_battery_policy(
    _method: &str,
    query_str: Option<&str>,
    body: &[u8],
    node: &Node,
) -> UiResponse {
    let qmap = query_str.map(parse_query_map).unwrap_or_default();
    let parsed_body: Option<BatteryPolicyBody> = if body.is_empty() {
        None
    } else {
        match serde_json::from_slice::<BatteryPolicyBody>(body) {
            Ok(b) => Some(b),
            Err(_) => return err(400, "bad json, expected BatteryPolicyBody"),
        }
    };
    let mut policy = node.battery_policy();

    if let Some(ref b) = parsed_body {
        if let Some(low) = b.low_battery_threshold {
            policy.low_battery_threshold = low.min(100);
        }
        if let Some(crit) = b.critical_battery_threshold {
            policy.critical_battery_threshold = crit.min(100);
        }
        if let Some(alert) = b.alert_on_low_battery {
            policy.alert_on_low_battery = alert;
        }
        if let Some(throttle) = b.auto_throttle_on_battery_saver {
            policy.auto_throttle_on_battery_saver = throttle;
        }
        if let Some(ref grps) = b.allowed_groups {
            policy.allowed_groups = grps.clone();
        }
        if let Some(ref tags) = b.allowed_tags {
            policy.allowed_tags = tags.clone();
        }
        if let Some(ref types) = b.allowed_types {
            let mut dtypes = Vec::new();
            for t in types {
                match t.parse::<crate::store::DeviceType>() {
                    Ok(dt) => dtypes.push(dt),
                    Err(e) => return err(400, &format!("invalid device type '{t}': {e}")),
                }
            }
            policy.allowed_types = dtypes;
        }
        if b.clear_filters == Some(true) {
            policy.allowed_groups.clear();
            policy.allowed_tags.clear();
            policy.allowed_types.clear();
        }
    }

    if let Some(low_str) = qmap
        .get("low_battery_threshold")
        .or_else(|| qmap.get("low_threshold"))
        .or_else(|| qmap.get("low"))
    {
        if let Ok(l) = low_str.parse::<u8>() {
            policy.low_battery_threshold = l.min(100);
        }
    }
    if let Some(crit_str) = qmap
        .get("critical_battery_threshold")
        .or_else(|| qmap.get("critical_threshold"))
        .or_else(|| qmap.get("critical"))
    {
        if let Ok(c) = crit_str.parse::<u8>() {
            policy.critical_battery_threshold = c.min(100);
        }
    }
    if let Some(alert_str) = qmap
        .get("alert_on_low_battery")
        .or_else(|| qmap.get("alert"))
    {
        if let Ok(a) = alert_str.parse::<bool>() {
            policy.alert_on_low_battery = a;
        }
    }
    if let Some(throttle_str) = qmap
        .get("auto_throttle_on_battery_saver")
        .or_else(|| qmap.get("throttle"))
    {
        if let Ok(t) = throttle_str.parse::<bool>() {
            policy.auto_throttle_on_battery_saver = t;
        }
    }
    if let Some(g) = qmap.get("group") {
        if !g.trim().is_empty() && !policy.allowed_groups.contains(g) {
            policy.allowed_groups.push(g.clone());
        }
    }
    if let Some(t) = qmap.get("tag") {
        if !t.trim().is_empty() && !policy.allowed_tags.contains(t) {
            policy.allowed_tags.push(t.clone());
        }
    }
    if let Some(dt_str) = qmap.get("type").or_else(|| qmap.get("device_type")) {
        match dt_str.parse::<crate::store::DeviceType>() {
            Ok(dt) => {
                if !policy.allowed_types.contains(&dt) {
                    policy.allowed_types.push(dt);
                }
            }
            Err(e) => return err(400, &format!("invalid device type '{dt_str}': {e}")),
        }
    }
    if qmap
        .get("clear_filters")
        .map(|s| s == "true" || s == "1")
        .unwrap_or(false)
    {
        policy.allowed_groups.clear();
        policy.allowed_tags.clear();
        policy.allowed_types.clear();
    }

    match node.set_battery_policy(policy) {
        Ok(rep) => json_ok(&rep),
        Err(e) => err(400, &e.to_string()),
    }
}
