// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Local action registry, remote action list, run/history/policy.

use super::util::{
    loopback_delete, loopback_events_connect, loopback_get, loopback_post, url_encode,
};
use super::*;

/// `tetherly-cli action` -- Action arm of the top-level dispatch.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn action(action: ActionCmd) -> Result<()> {
    match action {
        ActionCmd::Local { sub } => match sub {
            ActionLocalSubCmd::List { json, ui_port } => {
                do_action_local_list(ui_port, json).await?;
            }
            ActionLocalSubCmd::Add {
                id,
                name,
                cmd,
                args,
                desc,
                icon,
                timeout,
                enabled,
                json,
                ui_port,
            } => {
                do_action_local_add(
                    ui_port,
                    &id,
                    &name,
                    &cmd,
                    args,
                    desc.as_deref(),
                    icon.as_deref(),
                    Some(timeout),
                    Some(enabled),
                    json,
                )
                .await?;
            }
            ActionLocalSubCmd::Remove { id, json, ui_port } => {
                do_action_local_remove(ui_port, &id, json).await?;
            }
        },
        ActionCmd::Remote {
            peer,
            refresh,
            json,
            ui_port,
        } => {
            do_action_remote_list(ui_port, &peer, refresh, json).await?;
        }
        ActionCmd::Run {
            peer,
            action_id,
            json,
            ui_port,
        } => {
            do_action_run(ui_port, &peer, &action_id, json).await?;
        }
        ActionCmd::History { json, ui_port } => {
            do_action_history(ui_port, json).await?;
        }
        ActionCmd::Policy {
            action,
            mode,
            max_timeout_secs,
            expose,
            group,
            tag,
            device_type,
            clear_filters,
            json,
            ui_port,
        } => {
            do_action_policy(
                ui_port,
                action.as_deref(),
                mode.as_deref(),
                max_timeout_secs,
                expose,
                group,
                tag,
                device_type,
                clear_filters,
                json,
            )
            .await?;
        }
        ActionCmd::Watch { json, ui_port } => {
            do_action_watch(ui_port, json).await?;
        }
    }
    Ok(())
}

pub(crate) async fn do_action_local_list(ui_port: u16, json: bool) -> Result<()> {
    let (code, resp) = loopback_get(ui_port, "/api/action/local")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;
    if code != 200 {
        bail!(
            "failed to list local actions (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }

    let val: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
    let actions = if let Some(arr) = val.get("actions").and_then(|a| a.as_array()) {
        arr.clone()
    } else if let Some(arr) = val.as_array() {
        arr.clone()
    } else {
        Vec::new()
    };

    println!("Local Predefined Actions ({})", actions.len());
    if actions.is_empty() {
        println!("  (none configured)");
        return Ok(());
    }

    for a in actions {
        let id = a["id"].as_str().unwrap_or("");
        let name = a["name"].as_str().unwrap_or("");
        let cmd = a["command"].as_str().unwrap_or("");
        let args = a["args"]
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str())
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        let timeout = a["timeout_secs"].as_u64().unwrap_or(10);
        let enabled = a["enabled"].as_bool().unwrap_or(true);
        let desc = a["description"].as_str().unwrap_or("-");
        let icon = a["icon"].as_str().unwrap_or("-");

        println!("  - ID:          {id}");
        println!("    Name:        {name}");
        println!("    Command:     {cmd} {args}");
        println!("    Timeout:     {timeout}s (enabled: {enabled})");
        println!("    Icon/Desc:   [{icon}] {desc}");
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn do_action_local_add(
    ui_port: u16,
    id: &str,
    name: &str,
    cmd: &str,
    args: Option<Vec<String>>,
    desc: Option<&str>,
    icon: Option<&str>,
    timeout: Option<u32>,
    enabled: Option<bool>,
    json: bool,
) -> Result<()> {
    let mut payload = serde_json::json!({
        "id": id,
        "name": name,
        "command": cmd,
        "args": args.unwrap_or_default(),
    });
    if let Some(d) = desc {
        payload["description"] = serde_json::json!(d);
    }
    if let Some(i) = icon {
        payload["icon"] = serde_json::json!(i);
    }
    if let Some(t) = timeout {
        payload["timeout_secs"] = serde_json::json!(t);
    }
    if let Some(en) = enabled {
        payload["enabled"] = serde_json::json!(en);
    }

    let (code, resp) = loopback_post(ui_port, "/api/action/local", &serde_json::to_vec(&payload)?)
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
            )
        })?;

    if code != 200 {
        bail!(
            "failed to register local action (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }

    println!("Local Action Registered:");
    println!("  ID:      {id}");
    println!("  Name:    {name}");
    println!("  Command: {cmd}");
    Ok(())
}

pub(crate) async fn do_action_local_remove(ui_port: u16, id: &str, json: bool) -> Result<()> {
    let path = format!("/api/action/local?id={}", url_encode(id));
    let (code, resp) = loopback_delete(ui_port, &path).await.map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;

    if code != 200 {
        bail!(
            "failed to remove local action (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }

    println!("Local Action Removed: {id}");
    Ok(())
}

pub(crate) async fn do_action_remote_list(
    ui_port: u16,
    peer: &str,
    refresh: bool,
    json: bool,
) -> Result<()> {
    if refresh {
        let refresh_path = format!("/api/action/remote/refresh?peer={}", url_encode(peer));
        let (rcode, rresp) = loopback_post(ui_port, &refresh_path, &[])
            .await
            .map_err(|e| {
                anyhow::anyhow!(
                    "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                     start one with `tetherly --data-dir <dir> run` first"
                )
            })?;
        if rcode != 200 {
            bail!(
                "failed to request remote actions refresh (HTTP {rcode}): {}",
                String::from_utf8_lossy(&rresp)
            );
        }
    }

    let path = format!("/api/action/remote?peer={}", url_encode(peer));
    let (code, resp) = loopback_get(ui_port, &path).await.map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;

    if code != 200 {
        bail!(
            "failed to query remote actions (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }

    let val: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;

    println!("Remote Advertised Actions for '{peer}':");
    let actions = if let Some(arr) = val.get("actions").and_then(|a| a.as_array()) {
        arr.clone()
    } else if let Some(arr) = val.as_array() {
        arr.clone()
    } else {
        Vec::new()
    };

    if actions.is_empty() {
        println!("  (no advertised actions recorded yet; use --refresh to query peer)");
        return Ok(());
    }

    for act in actions {
        let aid = act["id"].as_str().unwrap_or("");
        let aname = act["name"].as_str().unwrap_or("");
        let adesc = act["description"].as_str().unwrap_or("-");
        let aicon = act["icon"].as_str().unwrap_or("-");
        println!("  - Action: [{aid}] \"{aname}\" (icon: {aicon})");
        if adesc != "-" {
            println!("    Description: {adesc}");
        }
    }
    Ok(())
}

pub(crate) async fn do_action_run(
    ui_port: u16,
    peer: &str,
    action_id: &str,
    json: bool,
) -> Result<()> {
    let payload = serde_json::json!({
        "peer": peer,
        "action_id": action_id,
    });
    let (code, resp) = loopback_post(
        ui_port,
        "/api/action/trigger",
        &serde_json::to_vec(&payload)?,
    )
    .await
    .map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;

    if code != 200 {
        bail!(
            "failed to trigger remote action (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }

    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
    let trig_id = v["trigger_id"].as_str().unwrap_or("");
    let pid = v["peer_id"].as_str().unwrap_or(peer);
    let aid = v["action_id"].as_str().unwrap_or(action_id);
    let status = v["status"].as_str().unwrap_or("dispatched");

    println!("Action Trigger Dispatched:");
    println!("  Trigger ID: {trig_id}");
    println!("  Target:     {pid}");
    println!("  Action ID:  {aid}");
    println!("  Status:     {status}");
    println!("  (run `tetherly action watch` or check `tetherly action history` for result)");
    Ok(())
}

pub(crate) async fn do_action_history(ui_port: u16, json: bool) -> Result<()> {
    let (code, resp) = loopback_get(ui_port, "/api/action/history")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;

    if code != 200 {
        bail!(
            "failed to query action history (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }

    let val: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
    let history = if let Some(arr) = val.get("results").and_then(|r| r.as_array()) {
        arr.clone()
    } else if let Some(arr) = val.as_array() {
        arr.clone()
    } else {
        Vec::new()
    };

    println!("Action Execution History ({})", history.len());
    if history.is_empty() {
        println!("  (no action executions recorded)");
        return Ok(());
    }

    for item in history {
        let tid = item["trigger_id"].as_str().unwrap_or("");
        let aid = item["action_id"].as_str().unwrap_or("");
        let dir = item["direction"].as_str().unwrap_or("inbound");
        let pid = item["peer_id"].as_str().unwrap_or("");
        let alias = item["peer_alias"].as_str().unwrap_or("");
        let success = item["success"].as_bool().unwrap_or(false);
        let exit_code = item["exit_code"].as_i64();
        let dur = item["duration_ms"].as_u64().unwrap_or(0);
        let out = item["output_summary"].as_str().unwrap_or("");
        let err = item["error_message"].as_str().unwrap_or("");

        let status_str = if success { "SUCCESS" } else { "FAILED" };
        let exit_str = exit_code
            .map(|c| format!("exit: {c}"))
            .unwrap_or_else(|| "no exit code".to_string());

        println!("  - [{status_str}] ID: {tid} Action: {aid} ({dir})");
        println!("    Peer:     {alias} ({pid})");
        println!("    Metrics:  {dur}ms | {exit_str}");
        if !out.is_empty() {
            println!("    Output:   {out}");
        }
        if !err.is_empty() {
            println!("    Error:    {err}");
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn do_action_policy(
    ui_port: u16,
    action: Option<&str>,
    mode: Option<&str>,
    max_timeout_secs: Option<u32>,
    expose: Option<bool>,
    group: Option<Vec<String>>,
    tag: Option<Vec<String>>,
    device_type: Option<Vec<String>>,
    clear_filters: bool,
    json: bool,
) -> Result<()> {
    let is_mutation = action == Some("set")
        || mode.is_some()
        || max_timeout_secs.is_some()
        || expose.is_some()
        || group.is_some()
        || tag.is_some()
        || device_type.is_some()
        || clear_filters;

    if is_mutation {
        let mut payload = serde_json::json!({});
        if let Some(m) = mode {
            payload["mode"] = serde_json::json!(m);
        }
        if let Some(t) = max_timeout_secs {
            payload["max_timeout_secs"] = serde_json::json!(t);
        }
        if let Some(exp) = expose {
            payload["expose_action_list"] = serde_json::json!(exp);
        }
        if let Some(grps) = group {
            payload["allowed_groups"] = serde_json::json!(grps);
        }
        if let Some(tags) = tag {
            payload["allowed_tags"] = serde_json::json!(tags);
        }
        if let Some(types) = device_type {
            payload["allowed_types"] = serde_json::json!(types);
        }
        if clear_filters {
            payload["clear_filters"] = serde_json::json!(true);
        }

        let (code, resp) = loopback_post(
            ui_port,
            "/api/action/policy",
            &serde_json::to_vec(&payload)?,
        )
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;

        if code != 200 {
            bail!(
                "failed to update action policy (HTTP {code}): {}",
                String::from_utf8_lossy(&resp)
            );
        }
        if json {
            println!("{}", String::from_utf8_lossy(&resp));
            return Ok(());
        }
        let v: serde_json::Value =
            serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
        println!("Action Policy Updated Successfully:");
        print_action_policy(&v);
        return Ok(());
    }

    let (code, resp) = loopback_get(ui_port, "/api/action/policy")
        .await
        .map_err(|e| {
            anyhow::anyhow!(
                "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
                 start one with `tetherly --data-dir <dir> run` first"
            )
        })?;

    if code != 200 {
        bail!(
            "failed to query action policy (HTTP {code}): {}",
            String::from_utf8_lossy(&resp)
        );
    }
    if json {
        println!("{}", String::from_utf8_lossy(&resp));
        return Ok(());
    }

    let v: serde_json::Value =
        serde_json::from_slice(&resp).map_err(|e| anyhow::anyhow!("bad response JSON: {e}"))?;
    println!("Action Governance Policy:");
    print_action_policy(&v);
    Ok(())
}

pub(crate) fn print_action_policy(v: &serde_json::Value) {
    let pol = v.get("policy").unwrap_or(v);
    let mode = pol["mode"].as_str().unwrap_or("allow");
    let max_timeout = pol["max_timeout_secs"].as_u64().unwrap_or(30);
    let expose = pol["expose_action_list"].as_bool().unwrap_or(true);
    let grps = pol["allowed_groups"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let tags = pol["allowed_tags"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let types = pol["allowed_types"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    println!("  Mode:                       {mode}");
    println!("  Max Timeout:                {max_timeout}s");
    println!("  Expose Action List:         {expose}");
    println!(
        "  Allowed Groups:             {}",
        if grps.is_empty() {
            "(all / unrestricted)".to_string()
        } else {
            format!("[{}]", grps.join(", "))
        }
    );
    println!(
        "  Allowed Tags:               {}",
        if tags.is_empty() {
            "(all / unrestricted)".to_string()
        } else {
            format!("[{}]", tags.join(", "))
        }
    );
    println!(
        "  Allowed Types:              {}",
        if types.is_empty() {
            "(all / unrestricted)".to_string()
        } else {
            format!("[{}]", types.join(", "))
        }
    );
}

pub(crate) async fn do_action_watch(ui_port: u16, json: bool) -> Result<()> {
    let mut sse = loopback_events_connect(ui_port).await.map_err(|e| {
        anyhow::anyhow!(
            "no running node reachable on 127.0.0.1:{ui_port} ({e}); \
             start one with `tetherly --data-dir <dir> run` first"
        )
    })?;
    if !json {
        println!("Streaming live action events on 127.0.0.1:{ui_port}...");
    }
    while let Ok(Some(raw)) = sse.next_event().await {
        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) {
            let kind = v["kind"].as_str().unwrap_or("");
            if kind == "action_list_requested"
                || kind == "action_list_received"
                || kind == "action_triggered"
                || kind == "action_result_received"
                || kind == "action_policy_changed"
                || kind == "local_actions_changed"
            {
                if json {
                    println!("{raw}");
                } else {
                    match kind {
                        "action_list_requested" => {
                            let peer = v["peer_alias"].as_str().unwrap_or("unknown");
                            let pid = v["peer_id"].as_str().unwrap_or("");
                            println!("[action_list_requested] peer={peer} ({pid}) requested available actions");
                        }
                        "action_list_received" => {
                            let peer = v["peer_alias"].as_str().unwrap_or("unknown");
                            let pid = v["peer_id"].as_str().unwrap_or("");
                            let count = v["count"].as_u64().unwrap_or(0);
                            println!("[action_list_received] peer={peer} ({pid}) advertised {count} actions");
                        }
                        "action_triggered" => {
                            let tid = v["trigger_id"].as_str().unwrap_or("");
                            let aid = v["action_id"].as_str().unwrap_or("");
                            let peer = v["peer_alias"].as_str().unwrap_or("unknown");
                            let pid = v["peer_id"].as_str().unwrap_or("");
                            println!("[action_triggered] trigger_id={tid} action_id={aid} from peer={peer} ({pid})");
                        }
                        "action_result_received" => {
                            let tid = v["trigger_id"].as_str().unwrap_or("");
                            let aid = v["action_id"].as_str().unwrap_or("");
                            let peer = v["peer_alias"].as_str().unwrap_or("unknown");
                            let succ = v["success"].as_bool().unwrap_or(false);
                            let dur = v["duration_ms"].as_u64().unwrap_or(0);
                            let exit = v["exit_code"]
                                .as_i64()
                                .map(|c| c.to_string())
                                .unwrap_or_else(|| "none".to_string());
                            let status = if succ { "SUCCESS" } else { "FAILED" };
                            println!("[action_result_received] trigger_id={tid} action={aid} peer={peer} status={status} duration={dur}ms exit={exit}");
                        }
                        "action_policy_changed" => {
                            let mode = v["mode"].as_str().unwrap_or("");
                            let max_t = v["max_timeout_secs"].as_u64().unwrap_or(0);
                            println!("[action_policy_changed] mode={mode} max_timeout={max_t}s");
                        }
                        "local_actions_changed" => {
                            let count = v["count"].as_u64().unwrap_or(0);
                            println!("[local_actions_changed] local action definitions updated: {count} configured");
                        }
                        _ => println!("[{kind}] {raw}"),
                    }
                }
            }
        }
    }
    Ok(())
}
