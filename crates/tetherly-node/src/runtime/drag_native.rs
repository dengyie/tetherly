//! v0.3.0 native receive-side drag: a file drag carried in from the
//! controller becomes a real OS drag on this (Windows) screen.
//!
//! Pre-v0.3.0 the carried files were only transferred on release and landed
//! in the inbox, revealed in Explorer; the user then had to drag the copy a
//! second time to where it was wanted. Now the receiver takes over the drag
//! while the button is still held:
//!
//! 1. **Pull** — on the carry `RemoteDragEnter` the receiver registers the
//!    drop (so the offers are auto-accepted), asks the controller for the
//!    bytes right away with `RemoteDragPull`, and spawns the console-session
//!    `tetherly drag-source` helper.
//! 2. **Fixed paths** — each claimed offer is bound to a predetermined path
//!    under `inbox/.drag/<drag_id>/`, so the helper can put the final
//!    `CF_HDROP` on its OLE data object before the bytes have landed.
//! 3. **Drag** — the helper presses the button on its own invisible window
//!    and runs `DoDragDrop`. The controller's forwarded release ends it; if
//!    the transfer is still running the helper holds the drop until it is
//!    complete (user decision: wait on release).
//! 4. **Outcome** — a drop that landed leaves the copy in place (the target
//!    may copy asynchronously; the directory is swept later). A miss, Esc or
//!    an early release falls back to the pre-v0.3.0 delivery: the files move
//!    into the inbox and the first is revealed. A drag carried back to the
//!    controller moves them into the inbox without opening Explorer.
//!
//! INVARIANTS:
//! - `RemoteDragPull` only goes to peers advertising `DRAG_PULL_CAP`.
//! - A native drag never deletes a delivered file; the fallback moves it.
//! - The helper only ever sees paths inside the native drag directory.

use super::*;
use std::path::{Path, PathBuf};
use tetherly_core::drag::{DragCancelReason, DragControlMsg, DRAG_DIR_CAP, DRAG_PULL_CAP};
use tetherly_core::DeviceId;
use tracing::{info, warn};

/// Directory under the inbox holding native drag payloads.
pub(crate) const NATIVE_DRAG_DIR: &str = ".drag";
/// Delivered native drag directories older than this are swept.
const NATIVE_SWEEP_MS: u64 = 60 * 60 * 1000;
/// Names are settled once every announced file was offered, or this long
/// after the enter when fewer arrived (the controller skipped a file).
const NAMES_SETTLE_MS: u64 = 8_000;
/// How long a carried-back drag id is remembered (stale enters dropped).
const RETURNED_TTL_MS: u64 = 30_000;

#[derive(Debug, Clone)]
pub(crate) struct NativeFile {
    pub(crate) transfer_id: String,
    pub(crate) path: PathBuf,
    pub(crate) done: bool,
}

/// What happens to the payload once the transfer completes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NativeOutcome {
    /// Dropped on a target: leave the copy where the target read it.
    Dropped,
    /// Pre-v0.3.0 delivery: move into the inbox, reveal when asked.
    Fallback { reveal: bool },
}

#[derive(Debug, Clone)]
pub(crate) struct NativeDrag {
    pub(crate) drag_id: u64,
    pub(crate) peer: DeviceId,
    pub(crate) dir: PathBuf,
    pub(crate) expected: u32,
    pub(crate) files: Vec<NativeFile>,
    /// The controller released the button (forwarded LMB up or a late
    /// `RemoteDropConfirm`).
    pub(crate) released: bool,
    /// The drag must end without a drop (carried back, peer gone).
    pub(crate) cancelled: bool,
    pub(crate) outcome: Option<NativeOutcome>,
    /// Start of the name-settling window: the enter, then the pulled
    /// handoff's refreshed enter (staging a large file can take a while).
    pub(crate) settle_from_ms: u64,
}

impl NativeDrag {
    fn names_settled(&self, now: u64) -> bool {
        let n = self.files.len() as u32;
        n >= self.expected || (n > 0 && now.saturating_sub(self.settle_from_ms) >= NAMES_SETTLE_MS)
    }

    fn all_done(&self) -> bool {
        !self.files.is_empty() && self.files.iter().all(|f| f.done)
    }
}

#[derive(Debug, Default)]
pub(crate) struct NativeDragState {
    pub(crate) cur: Option<NativeDrag>,
    /// Native drags the controller carried back (drag id → when). The
    /// controller mints a new id per crossing, so a later enter with one of
    /// these ids is a stale echo of a pull that raced the return.
    returned: HashMap<u64, u64>,
}

/// Helper-facing view of the drag (`POST /api/v1/drag/native/poll`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct NativeDragView {
    /// `pending` | `ready` | `cancel` | `failed`.
    pub(crate) state: &'static str,
    pub(crate) released: bool,
    /// Final payload paths, published once the names are settled.
    pub(crate) paths: Vec<String>,
}

/// Unique file name inside one drag directory (two dragged files may share
/// a name when they came from different folders).
fn unique_in(dir: &Path, taken: &[NativeFile], name: &str) -> PathBuf {
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, Some(ext)),
        _ => (name, None),
    };
    for n in 0u32.. {
        let candidate = if n == 0 {
            dir.join(name)
        } else {
            match ext {
                Some(ext) => dir.join(format!("{stem} ({n}).{ext}")),
                None => dir.join(format!("{stem} ({n})")),
            }
        };
        if !taken.iter().any(|f| f.path == candidate) {
            return candidate;
        }
    }
    unreachable!("u32 counter exhausted")
}

fn sweep_native_dirs(root: &Path, now_ms: u64) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for e in entries.flatten() {
        let age_ok = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| now_ms.saturating_sub(d.as_millis() as u64) > NATIVE_SWEEP_MS)
            .unwrap_or(false);
        if age_ok {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}

impl Node {
    /// Whether this node turns carried drags into native OS drags: Windows
    /// only (the `drag-source` helper), opt-out via
    /// `TETHERLY_NO_NATIVE_DRAG=1`.
    /// Off in unit tests: `begin` spawns the real helper (the test binary
    /// would be re-executed as `drag-source`); the state machine is tested
    /// through `native_drag_install` directly.
    pub(crate) fn native_drag_supported(&self) -> bool {
        cfg!(windows) && !cfg!(test) && std::env::var_os("TETHERLY_NO_NATIVE_DRAG").is_none()
    }

    pub(crate) fn peer_has_cap(&self, peer: &DeviceId, cap: &str) -> bool {
        self.shared
            .live
            .lock()
            .expect("live")
            .get(peer)
            .is_some_and(|p| p.caps.iter().any(|c| c == cap))
    }

    /// Whether `peer` can answer a pull for this carry: it must understand
    /// `RemoteDragPull`, and for a carry with a folder also send folders.
    pub(crate) fn native_drag_peer_ok(&self, peer: &DeviceId, has_directory: bool) -> bool {
        self.peer_has_cap(peer, DRAG_PULL_CAP)
            && (!has_directory || self.peer_has_cap(peer, DRAG_DIR_CAP))
    }

    fn native_drag_root(&self) -> PathBuf {
        self.shared.cfg.data_dir.join("inbox").join(NATIVE_DRAG_DIR)
    }

    /// Receiver: a carry entered this screen. Returns true when it became a
    /// native drag (pull sent, helper spawned).
    pub(crate) async fn native_drag_begin(
        &self,
        peer: &DeviceId,
        drag_id: u64,
        file_count: u32,
        has_directory: bool,
    ) -> bool {
        if file_count == 0 || !self.native_drag_supported() {
            return false;
        }
        if !self.native_drag_peer_ok(peer, has_directory) {
            info!(peer = %peer, drag_id, has_directory, "native drag: peer cannot serve this carry");
            return false;
        }
        let now = self.now_ms();
        if !self.native_drag_install(peer, drag_id, file_count, now) {
            return false;
        }
        let pull = DragControlMsg::RemoteDragPull { drag_id };
        if let Err(e) = self.send_drag_control(peer, &pull).await {
            warn!(peer = %peer, drag_id, error = %e, "native drag: pull failed");
            self.shared.native_drag.lock().expect("native_drag").cur = None;
            return false;
        }
        if let Err(e) = crate::platform::drag_source::spawn_drag_source(self.ui_port(), drag_id) {
            warn!(drag_id, error = %e, "native drag: helper spawn failed");
            // The files still arrive; deliver them the pre-v0.3.0 way.
            self.native_drag_resolve(drag_id, NativeOutcome::Fallback { reveal: true });
        }
        info!(peer = %peer, drag_id, file_count, "native drag: carry taken over");
        true
    }

    /// State half of [`Self::native_drag_begin`] (no I/O beyond the sweep),
    /// split out so the state machine is testable off Windows.
    pub(crate) fn native_drag_install(
        &self,
        peer: &DeviceId,
        drag_id: u64,
        file_count: u32,
        now: u64,
    ) -> bool {
        let root = self.native_drag_root();
        sweep_native_dirs(&root, now);
        let dir = root.join(drag_id.to_string());
        if let Err(e) = std::fs::create_dir_all(&dir) {
            warn!(drag_id, error = %e, "native drag: no payload directory");
            return false;
        }
        // Any older native drag is superseded (one drag per pointer).
        let prev = {
            let mut st = self.shared.native_drag.lock().expect("native_drag");
            let prev = st.cur.take();
            st.cur = Some(NativeDrag {
                drag_id,
                peer: peer.clone(),
                dir,
                expected: file_count,
                files: Vec::new(),
                released: false,
                cancelled: false,
                outcome: None,
                settle_from_ms: now,
            });
            prev
        };
        if let Some(prev) = prev {
            self.native_drag_finish(prev, NativeOutcome::Fallback { reveal: false });
        }
        // Register the drop now: the controller pushes the offers without
        // a `RemoteDropConfirm`, and they must be accepted unprompted.
        self.shared
            .pending_drag_drops
            .lock()
            .expect("pending_drag_drops")
            .insert(
                drag_id,
                PendingDragDrop {
                    peer: peer.clone(),
                    registered_ms: now,
                    remaining: file_count,
                },
            );
        true
    }

    pub(crate) fn is_native_drag(&self, drag_id: u64) -> bool {
        self.shared
            .native_drag
            .lock()
            .expect("native_drag")
            .cur
            .as_ref()
            .is_some_and(|d| d.drag_id == drag_id)
    }

    /// Offer handler: a claimed offer of the native drag gets its fixed path.
    pub(crate) fn native_drag_bind(&self, transfer_id: &str, name: &str) {
        let drag_id = self
            .shared
            .transfer_drag_ids
            .lock()
            .expect("transfer_drag_ids")
            .get(transfer_id)
            .map(|v| v.1);
        let Some(drag_id) = drag_id else {
            return;
        };
        let mut st = self.shared.native_drag.lock().expect("native_drag");
        let Some(d) = st.cur.as_mut().filter(|d| d.drag_id == drag_id) else {
            return;
        };
        if d.files.iter().any(|f| f.transfer_id == transfer_id) {
            return;
        }
        let path = unique_in(&d.dir, &d.files, name);
        d.files.push(NativeFile {
            transfer_id: transfer_id.to_string(),
            path,
            done: false,
        });
    }

    /// `accept_file`: where a native drag transfer is installed, if it is one.
    pub(crate) fn native_drag_path(&self, transfer_id: &str) -> Option<PathBuf> {
        let st = self.shared.native_drag.lock().expect("native_drag");
        st.cur
            .as_ref()?
            .files
            .iter()
            .find(|f| f.transfer_id == transfer_id)
            .map(|f| f.path.clone())
    }

    /// Completion hook (from `settle_drag_for_transfer`). Returns true when
    /// the transfer belonged to the native drag (no reveal here).
    pub(crate) fn native_drag_delivered(&self, transfer_id: &str) -> bool {
        let finished = {
            let mut st = self.shared.native_drag.lock().expect("native_drag");
            let Some(d) = st.cur.as_mut() else {
                return false;
            };
            let Some(f) = d.files.iter_mut().find(|f| f.transfer_id == transfer_id) else {
                return false;
            };
            f.done = true;
            let now = self.now_ms();
            match d.outcome {
                Some(o) if d.all_done() && d.names_settled(now) => st.cur.take().map(|d| (d, o)),
                _ => None,
            }
        };
        if let Some((d, o)) = finished {
            self.native_drag_finish(d, o);
        }
        true
    }

    /// The pulled handoff re-sends the enter with the staged count (files
    /// it could not read are skipped). Returns true for the native drag.
    pub(crate) fn native_drag_refresh(&self, drag_id: u64, file_count: u32) -> bool {
        let now = self.now_ms();
        let claimed = {
            let mut st = self.shared.native_drag.lock().expect("native_drag");
            match st.cur.as_mut().filter(|d| d.drag_id == drag_id) {
                Some(d) => {
                    d.expected = file_count.max(1);
                    d.settle_from_ms = now;
                    d.files.len() as u32
                }
                None => return false,
            }
        };
        // The offers follow this enter: keep the drop registration fresh
        // (its TTL ran from the crossing) and sized to the staged count.
        let mut pending = self
            .shared
            .pending_drag_drops
            .lock()
            .expect("pending_drag_drops");
        let remaining = file_count.max(1).saturating_sub(claimed);
        if remaining == 0 {
            pending.remove(&drag_id);
        } else if let Some(p) = pending.get_mut(&drag_id) {
            p.registered_ms = now;
            p.remaining = remaining;
        }
        true
    }

    /// `RemoteDropConfirm` for the native drag: the controller released
    /// before it honoured the pull. Returns true when it was the native one.
    pub(crate) fn native_drag_confirmed(&self, drag_id: u64) -> bool {
        let mut st = self.shared.native_drag.lock().expect("native_drag");
        match st.cur.as_mut().filter(|d| d.drag_id == drag_id) {
            Some(d) => {
                d.released = true;
                true
            }
            None => false,
        }
    }

    /// A carry enter for a native drag the controller already took back.
    pub(crate) fn native_drag_is_stale_enter(&self, drag_id: u64) -> bool {
        let now = self.now_ms();
        let mut st = self.shared.native_drag.lock().expect("native_drag");
        st.returned
            .retain(|_, at| now.saturating_sub(*at) <= RETURNED_TTL_MS);
        st.returned.contains_key(&drag_id) && st.cur.as_ref().is_none_or(|d| d.drag_id != drag_id)
    }

    /// `RemoteDragCancel` for the native drag.
    pub(crate) fn native_drag_cancelled(&self, drag_id: u64, reason: DragCancelReason) -> bool {
        let back = matches!(reason, DragCancelReason::DraggedBackToLocal);
        let reveal = !back;
        {
            let now = self.now_ms();
            let mut st = self.shared.native_drag.lock().expect("native_drag");
            match st.cur.as_mut().filter(|d| d.drag_id == drag_id) {
                Some(d) => d.cancelled = true,
                None => return false,
            }
            if back {
                st.returned.insert(drag_id, now);
            }
        }
        self.native_drag_resolve(drag_id, NativeOutcome::Fallback { reveal });
        true
    }

    /// The peer went away: whatever arrived is delivered to the inbox.
    pub(crate) fn native_drag_peer_down(&self, peer: &DeviceId) {
        let id = {
            let mut st = self.shared.native_drag.lock().expect("native_drag");
            match st.cur.as_mut().filter(|d| d.peer == *peer) {
                Some(d) => {
                    d.cancelled = true;
                    // Unfinished transfers will not complete now.
                    d.files.retain(|f| f.done);
                    d.drag_id
                }
                None => return,
            }
        };
        self.native_drag_resolve(id, NativeOutcome::Fallback { reveal: true });
    }

    /// Fix the outcome; it applies now when every file has landed, else on
    /// the last completion.
    pub(crate) fn native_drag_resolve(&self, drag_id: u64, outcome: NativeOutcome) {
        let now = self.now_ms();
        let (finished, legacy) = {
            let mut st = self.shared.native_drag.lock().expect("native_drag");
            let Some(d) = st.cur.as_mut().filter(|d| d.drag_id == drag_id) else {
                return;
            };
            if d.outcome.is_none() {
                d.outcome = Some(outcome);
            }
            let o = d.outcome.unwrap_or(outcome);
            if d.all_done() && d.names_settled(now) {
                (st.cur.take().map(|d| (d, o)), None)
            } else if d.files.is_empty() {
                // No offer arrived yet: nothing to move later, so hand the
                // drop back to the pre-v0.3.0 path (inbox, reveal).
                let reveal = matches!(o, NativeOutcome::Fallback { reveal: true });
                (None, st.cur.take().map(|d| (d, reveal)))
            } else {
                (None, None)
            }
        };
        if let Some((d, o)) = finished {
            self.native_drag_finish(d, o);
        }
        if let Some((d, reveal)) = legacy {
            self.native_drag_to_legacy(d, reveal, now);
        }
    }

    /// The offers of `d` have not arrived: they keep their drop
    /// registration and settle through `settle_drag_for_transfer` (inbox,
    /// and a reveal unless `reveal` is false).
    fn native_drag_to_legacy(&self, d: NativeDrag, reveal: bool, now: u64) {
        {
            let mut target = self.shared.drag_target.lock().expect("drag_target");
            let res = if reveal {
                target
                    .handle_drop_confirm(d.drag_id, 0.5, 0.5, now)
                    .map(|_| ())
            } else {
                target.handle_drag_cancel(d.drag_id, DragCancelReason::DraggedBackToLocal)
            };
            if let Err(e) = res {
                warn!(drag_id = d.drag_id, error = %e, "native drag: legacy handover");
            }
        }
        if let Some(p) = self
            .shared
            .pending_drag_drops
            .lock()
            .expect("pending_drag_drops")
            .get_mut(&d.drag_id)
        {
            p.registered_ms = now;
        }
        self.emit(UiEvent::DragCarry {
            active: false,
            file_count: 0,
            first_file_name: String::new(),
        });
        let _ = std::fs::remove_dir_all(&d.dir);
        info!(
            drag_id = d.drag_id,
            reveal, "native drag handed back before any offer"
        );
    }

    fn native_drag_finish(&self, d: NativeDrag, outcome: NativeOutcome) {
        let _ = self
            .shared
            .drag_target
            .lock()
            .expect("drag_target")
            .handle_drag_cancel(d.drag_id, DragCancelReason::UserEscPressed);
        self.shared
            .pending_drag_drops
            .lock()
            .expect("pending_drag_drops")
            .remove(&d.drag_id);
        self.emit(UiEvent::DragCarry {
            active: false,
            file_count: 0,
            first_file_name: String::new(),
        });
        match outcome {
            NativeOutcome::Dropped => {
                info!(
                    drag_id = d.drag_id,
                    files = d.files.len(),
                    "native drag dropped"
                );
            }
            NativeOutcome::Fallback { reveal } => {
                let inbox = self.shared.cfg.data_dir.join("inbox");
                let mut first: Option<PathBuf> = None;
                for f in d.files.iter().filter(|f| f.done) {
                    let name = f
                        .path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "file.bin".into());
                    let moved = if f.path.is_dir() {
                        super::bundle::move_dir_into(&f.path, &inbox, &name)
                    } else {
                        super::files::reserve_inbox_target(&f.path, &inbox, &name).inspect(|_| {
                            let _ = std::fs::remove_file(&f.path);
                        })
                    };
                    match moved {
                        Ok(dst) => {
                            first.get_or_insert(dst);
                        }
                        Err(e) => {
                            warn!(drag_id = d.drag_id, error = %e, "native drag: inbox move failed")
                        }
                    }
                }
                let _ = std::fs::remove_dir_all(&d.dir);
                info!(
                    drag_id = d.drag_id,
                    reveal, "native drag delivered to the inbox"
                );
                if let (true, Some(p)) = (reveal, first) {
                    if let Err(e) = super::shell_reveal::reveal_in_file_manager(&p) {
                        warn!(error = %e, "native drag: reveal failed");
                    }
                }
            }
        }
    }

    /// Helper poll: what the drag source may do next.
    pub(crate) fn native_drag_view(&self, drag_id: u64) -> NativeDragView {
        let now = self.now_ms();
        let st = self.shared.native_drag.lock().expect("native_drag");
        let Some(d) = st.cur.as_ref().filter(|d| d.drag_id == drag_id) else {
            return NativeDragView {
                state: "cancel",
                released: true,
                paths: Vec::new(),
            };
        };
        let failed = {
            let transfers = self.shared.active_transfers.lock().expect("transfers");
            d.files.iter().any(|f| {
                transfers
                    .get(&f.transfer_id)
                    .is_some_and(|t| t.phase == TransferPhase::Failed)
            })
        };
        let settled = d.names_settled(now);
        let state = if d.cancelled || d.outcome.is_some() {
            "cancel"
        } else if failed {
            "failed"
        } else if settled && d.all_done() {
            "ready"
        } else {
            "pending"
        };
        NativeDragView {
            state,
            released: d.released,
            paths: if settled {
                d.files
                    .iter()
                    .map(|f| f.path.to_string_lossy().into_owned())
                    .collect()
            } else {
                Vec::new()
            },
        }
    }

    /// Helper result: `dropped` | `missed` | `esc` | `released` |
    /// `cancelled` | `failed`. Returns false for an unknown drag.
    pub(crate) fn native_drag_report(&self, drag_id: u64, outcome: &str) -> bool {
        if !self.is_native_drag(drag_id) {
            return false;
        }
        info!(drag_id, outcome, "native drag: helper finished");
        let o = match outcome {
            "dropped" => NativeOutcome::Dropped,
            // Carried back: `native_drag_cancelled` already chose the outcome.
            "cancelled" => NativeOutcome::Fallback { reveal: false },
            _ => NativeOutcome::Fallback { reveal: true },
        };
        self.native_drag_resolve(drag_id, o);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::clipboard_policy_tests::temp_test_node;

    fn peer() -> DeviceId {
        DeviceId::from_id_pk(&[5u8; 32])
    }

    fn land(node: &Node, tid: &str, bytes: &[u8]) {
        let p = node.native_drag_path(tid).expect("bound path");
        std::fs::write(&p, bytes).unwrap();
        assert!(node.native_drag_delivered(tid));
    }

    /// Offers bind to fixed paths, names publish once complete, and a drop
    /// that landed leaves the payload where the target read it.
    #[test]
    fn native_drag_publishes_paths_then_ready_then_drop_keeps_copy() {
        let (node, _dir) = temp_test_node("native_drop");
        let p = peer();
        assert!(node.native_drag_install(&p, 11, 2, node.now_ms()));
        assert!(node.claim_drag_offer(&p, "t1"));
        node.native_drag_bind("t1", "a.txt");
        let v = node.native_drag_view(11);
        assert_eq!(
            (v.state, v.paths.len()),
            ("pending", 0),
            "names not settled"
        );
        assert!(node.claim_drag_offer(&p, "t2"));
        node.native_drag_bind("t2", "a.txt");
        let v = node.native_drag_view(11);
        assert_eq!(v.state, "pending");
        assert_eq!(v.paths.len(), 2);
        assert!(
            v.paths[1].ends_with("a (1).txt"),
            "duplicate names stay apart"
        );
        land(&node, "t1", b"1");
        land(&node, "t2", b"2");
        assert_eq!(node.native_drag_view(11).state, "ready");
        assert!(node.native_drag_confirmed(11));
        assert!(node.native_drag_view(11).released);
        assert!(node.native_drag_report(11, "dropped"));
        assert!(!node.is_native_drag(11));
        assert!(
            std::path::Path::new(&v.paths[0]).exists(),
            "dropped copy kept"
        );
    }

    /// A miss before the bytes landed waits for them, then moves them into
    /// the inbox (pre-v0.3.0 delivery).
    #[test]
    fn missed_drop_moves_payload_to_inbox_once_complete() {
        let (node, _dir) = temp_test_node("native_miss");
        let p = peer();
        assert!(node.native_drag_install(&p, 12, 1, node.now_ms()));
        assert!(node.claim_drag_offer(&p, "t1"));
        node.native_drag_bind("t1", "b.txt");
        assert!(node.native_drag_report(12, "missed"));
        assert!(node.is_native_drag(12), "waits for the transfer");
        assert_eq!(node.native_drag_view(12).state, "cancel");
        land(&node, "t1", b"x");
        assert!(!node.is_native_drag(12));
        let inbox = node.shared.cfg.data_dir.join("inbox");
        assert_eq!(std::fs::read(inbox.join("b.txt")).unwrap(), b"x");
        assert!(!inbox.join(NATIVE_DRAG_DIR).join("12").exists());
    }

    /// v0.3.3: a bundled folder lands as a directory at its bound path; a
    /// missed drop moves the whole folder into the inbox.
    #[test]
    fn missed_folder_drop_moves_the_folder_to_the_inbox() {
        let (node, _dir) = temp_test_node("native_dir");
        let p = peer();
        assert!(node.native_drag_install(&p, 17, 1, node.now_ms()));
        assert!(node.claim_drag_offer(&p, "t1"));
        node.native_drag_bind("t1", "proj");
        assert!(node.native_drag_report(17, "missed"));
        let fixed = node.native_drag_path("t1").expect("bound path");
        std::fs::create_dir_all(fixed.join("sub")).unwrap();
        std::fs::write(fixed.join("sub/x.txt"), b"x").unwrap();
        assert!(node.native_drag_delivered("t1"));
        let inbox = node.shared.cfg.data_dir.join("inbox");
        assert_eq!(std::fs::read(inbox.join("proj/sub/x.txt")).unwrap(), b"x");
        assert!(!inbox.join(NATIVE_DRAG_DIR).join("17").exists());
    }

    /// v0.3.3: a folder carry goes native only for a sender that sends
    /// folders; a plain-file carry needs just the pull capability.
    #[tokio::test]
    async fn folder_carry_needs_the_folder_capability() {
        let (node, _dir) = temp_test_node("native_dircap");
        let register = |id: u8, caps: Vec<String>| {
            let node = &node;
            async move {
                let peer = DeviceId::from_id_pk(&[id; 32]);
                let (tx, _rx) = tokio::sync::mpsc::channel::<tetherly_core::frame::InnerFrame>(4);
                let live = crate::runtime::LivePeer {
                    device_id: peer.clone(),
                    name: format!("p{id}"),
                    platform: "test".into(),
                    addr: "127.0.0.1:1".parse().unwrap(),
                    handshake_hash: [id; 32],
                    file_port: None,
                    input_port: None,
                    screen_port: None,
                    caps,
                    path: tetherly_core::PathKind::Lan,
                    rtt_ms: None,
                    last_seen_ms: 0,
                };
                node.register_session(live, tx).await;
                peer
            }
        };
        let old = register(61, vec![DRAG_PULL_CAP.into()]).await;
        let new = register(62, vec![DRAG_PULL_CAP.into(), DRAG_DIR_CAP.into()]).await;
        let none = register(63, vec![]).await;
        assert!(node.native_drag_peer_ok(&old, false));
        assert!(
            !node.native_drag_peer_ok(&old, true),
            "old sender skips folders"
        );
        assert!(node.native_drag_peer_ok(&new, true));
        assert!(!node.native_drag_peer_ok(&none, false));
        assert!(node
            .shared
            .local_caps
            .lock()
            .unwrap()
            .contains(DRAG_DIR_CAP));
    }

    /// Carried back to the controller: delivered quietly, and an unknown
    /// drag reads as cancelled to a stray helper.
    #[test]
    fn carried_back_drag_cancels_and_unknown_drag_reads_cancel() {
        let (node, _dir) = temp_test_node("native_back");
        let p = peer();
        assert!(node.native_drag_install(&p, 13, 1, node.now_ms()));
        assert!(node.claim_drag_offer(&p, "t1"));
        node.native_drag_bind("t1", "c.txt");
        assert!(node.native_drag_cancelled(13, DragCancelReason::DraggedBackToLocal));
        assert_eq!(node.native_drag_view(13).state, "cancel");
        land(&node, "t1", b"c");
        assert!(node
            .shared
            .cfg
            .data_dir
            .join("inbox")
            .join("c.txt")
            .exists());
        assert_eq!(node.native_drag_view(99).state, "cancel");
        assert!(!node.native_drag_report(99, "dropped"));
        // A pull that raced the return re-announces 13: stale, not a new carry.
        assert!(node.native_drag_is_stale_enter(13));
        assert!(!node.native_drag_is_stale_enter(14));
    }

    /// Released before any offer arrived: the drop stays registered, so the
    /// offers still auto-accept and settle the pre-v0.3.0 way.
    #[test]
    fn release_before_any_offer_hands_the_drop_back() {
        let (node, _dir) = temp_test_node("native_legacy");
        let p = peer();
        assert!(node.native_drag_install(&p, 16, 1, node.now_ms()));
        assert!(node.native_drag_report(16, "released"));
        assert!(!node.is_native_drag(16));
        assert!(node.claim_drag_offer(&p, "t1"), "offer still claimed");
        node.native_drag_bind("t1", "late.txt");
        assert!(node.native_drag_path("t1").is_none(), "goes to the inbox");
        assert!(!node.native_drag_delivered("t1"));
    }

    /// The release path's late confirm marks the drag released; a non-native
    /// transfer is not swallowed by the completion hook.
    #[test]
    fn late_confirm_marks_release_and_foreign_transfers_pass_through() {
        let (node, _dir) = temp_test_node("native_confirm");
        let p = peer();
        assert!(node.native_drag_install(&p, 14, 1, node.now_ms()));
        assert!(node.native_drag_refresh(14, 3));
        assert!(!node.native_drag_refresh(15, 3));
        assert!(node.native_drag_confirmed(14));
        assert!(!node.native_drag_confirmed(15));
        assert!(node.native_drag_view(14).released);
        assert!(!node.native_drag_delivered("unrelated"));
    }
}
