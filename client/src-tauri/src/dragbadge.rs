//! Drag-carry badge: while a file drag from the peer is carried across this
//! screen, a small click-through window follows the cursor and names the
//! files, so the receiving side sees what it is about to drop.
//!
//! The daemon announces the carry as a `diagnostic` event
//! `{kind: "drag_carry", payload: {active, file_count, first_file_name}}`;
//! this module
//! parses it, shows/hides the badge and moves it with the cursor.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use tauri::{AppHandle, Manager, Runtime};

const LABEL: &str = "drag-badge";
const BADGE_W: f64 = 280.0;
const BADGE_H: f64 = 40.0;
/// Offset from the cursor hotspot (physical px) so the badge never sits
/// under the pointer.
const OFFSET: (f64, f64) = (18.0, 22.0);
const FOLLOW_EVERY: Duration = Duration::from_millis(16);
/// Safety net: a lost "inactive" frame must not leave the badge up forever.
const MAX_SHOWN: Duration = Duration::from_secs(60);

/// What the daemon said about the carried drag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Carry {
    Active {
        file_count: u32,
        first_file_name: String,
    },
    Inactive,
}

/// Parse one forwarded event frame; `None` when it is not a drag-carry event.
pub fn carry_from_frame(ty: &str, data: &serde_json::Value) -> Option<Carry> {
    if ty != "diagnostic" || data.get("kind").and_then(|k| k.as_str()) != Some("drag_carry") {
        return None;
    }
    // The wire frame is `{kind, payload}` (flattened DiagnosticFrame); the
    // payload carries the event's own fields.
    let data = data.get("payload").unwrap_or(data);
    if !data
        .get("active")
        .and_then(|a| a.as_bool())
        .unwrap_or(false)
    {
        return Some(Carry::Inactive);
    }
    let file_count = data
        .get("file_count")
        .and_then(|n| n.as_u64())
        .map(|n| n.min(u32::MAX as u64) as u32)
        .unwrap_or(0);
    let first_file_name = data
        .get("first_file_name")
        .and_then(|n| n.as_str())
        .unwrap_or_default()
        .to_string();
    Some(Carry::Active {
        file_count,
        first_file_name,
    })
}

/// The badge text: the file name, plus the count when there is more than one.
pub fn badge_label(file_count: u32, first_file_name: &str) -> String {
    let name = if first_file_name.is_empty() {
        "文件"
    } else {
        first_file_name
    };
    if file_count > 1 {
        format!("{name} 等 {file_count} 个文件")
    } else {
        name.to_string()
    }
}

/// Percent-encode for a URL query value (RFC 3986 unreserved kept).
fn query_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Each show bumps the generation; a follow loop stops once it is stale.
static GENERATION: AtomicU64 = AtomicU64::new(0);

/// Apply one carry update.
pub fn apply<R: Runtime>(app: &AppHandle<R>, carry: Carry) {
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    match carry {
        Carry::Inactive => {
            if let Some(w) = app.get_webview_window(LABEL) {
                let _ = w.hide();
            }
        }
        Carry::Active {
            file_count,
            first_file_name,
        } => {
            let label = badge_label(file_count, &first_file_name);
            let Some(w) = badge_window(app, &label) else {
                return;
            };
            let mut last = place(app, &w, None);
            let _ = w.show();
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let started = std::time::Instant::now();
                while GENERATION.load(Ordering::SeqCst) == generation {
                    if started.elapsed() >= MAX_SHOWN {
                        tracing::info!("drag badge: safety timeout -> hide");
                        let _ = w.hide();
                        break;
                    }
                    last = place(&app, &w, last);
                    tokio::time::sleep(FOLLOW_EVERY).await;
                }
            });
        }
    }
}

/// Move the badge next to the cursor; returns where it now is. A resting
/// cursor costs no window move (each one is a main-thread round trip and a
/// compositor update — 60/s of them made the carry feel sluggish).
fn place<R: Runtime>(
    app: &AppHandle<R>,
    w: &tauri::WebviewWindow<R>,
    last: Option<(i32, i32)>,
) -> Option<(i32, i32)> {
    let Ok(p) = app.cursor_position() else {
        return last;
    };
    let at = badge_origin(p.x, p.y);
    if last != Some(at) {
        let _ = w.set_position(tauri::PhysicalPosition::new(at.0, at.1));
    }
    Some(at)
}

fn badge_origin(x: f64, y: f64) -> (i32, i32) {
    ((x + OFFSET.0).round() as i32, (y + OFFSET.1).round() as i32)
}

/// The badge window, created on first use with `label` preloaded (the page
/// may not have loaded yet), relabelled via `__setBadge` afterwards.
fn badge_window<R: Runtime>(app: &AppHandle<R>, label: &str) -> Option<tauri::WebviewWindow<R>> {
    if let Some(w) = app.get_webview_window(LABEL) {
        let text = serde_json::to_string(label).unwrap_or_else(|_| "\"\"".into());
        let _ = w.eval(format!("window.__setBadge && window.__setBadge({text})"));
        return Some(w);
    }
    let url = format!("drag-badge.html?t={}", query_encode(label));
    let b = tauri::WebviewWindowBuilder::new(app, LABEL, tauri::WebviewUrl::App(url.into()))
        .title("Tetherly drag")
        .decorations(false)
        .resizable(false)
        .skip_taskbar(true)
        .shadow(false)
        .always_on_top(true)
        .focused(false)
        .focusable(false)
        .visible_on_all_workspaces(true)
        .visible(false)
        .inner_size(BADGE_W, BADGE_H);
    #[cfg(not(target_os = "macos"))]
    let b = b.transparent(true);
    match b.build() {
        Ok(w) => {
            let _ = w.set_ignore_cursor_events(true);
            Some(w)
        }
        Err(e) => {
            tracing::warn!("drag badge build failed: {e}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn active_carry_frame_is_parsed() {
        let data = json!({"kind": "drag_carry", "payload": {"kind": "drag_carry", "active": true, "file_count": 3, "first_file_name": "报告.pdf"}});
        assert_eq!(
            carry_from_frame("diagnostic", &data),
            Some(Carry::Active {
                file_count: 3,
                first_file_name: "报告.pdf".into()
            })
        );
    }

    #[test]
    fn inactive_carry_frame_hides() {
        let data =
            json!({"kind": "drag_carry", "active": false, "file_count": 0, "first_file_name": ""});
        assert_eq!(carry_from_frame("diagnostic", &data), Some(Carry::Inactive));
    }

    #[test]
    fn other_frames_are_ignored() {
        assert_eq!(
            carry_from_frame("diagnostic", &json!({"kind": "cursor_edge_crossed"})),
            None
        );
        assert_eq!(
            carry_from_frame("seat", &json!({"kind": "drag_carry", "active": true})),
            None
        );
    }

    #[test]
    fn label_names_the_file_and_counts_extras() {
        assert_eq!(badge_label(1, "a.txt"), "a.txt");
        assert_eq!(badge_label(4, "a.txt"), "a.txt 等 4 个文件");
        assert_eq!(badge_label(1, ""), "文件");
    }

    #[test]
    fn badge_origin_is_stable_for_a_resting_cursor() {
        assert_eq!(badge_origin(100.2, 50.4), badge_origin(100.4, 50.2));
        assert_eq!(badge_origin(100.0, 50.0), (118, 72));
    }

    #[test]
    fn query_values_are_percent_encoded() {
        assert_eq!(query_encode("a b&c"), "a%20b%26c");
        assert_eq!(query_encode("文"), "%E6%96%87");
    }
}
