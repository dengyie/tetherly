//! v0.2.89 cross-screen file drag handoff.
//!
//! A file drag that crosses the screen edge can never be carried by the OS
//! drag loop itself: the source machine's drag session keeps running on the
//! source machine, and the cursor on the other machine is driven by injected
//! input that no OS drag session knows about. So the drag is converted into
//! a transfer at the crossing instead:
//!
//! 1. **Probe** — when the cursor leaves its machine with the left button
//!    held, ask the OS which files the in-flight drag carries (macOS: the
//!    drag pasteboard, gated on its change count moving since the button
//!    went down; Windows: a short-lived invisible OLE drop target spawned
//!    into the console session under the cursor).
//! 2. **Cancel** — inject Esc locally so the source drag ends without
//!    dropping anything at the edge.
//! 3. **Hand off** — on release (controller side) or immediately (target
//!    side), stage the files into the daemon-owned send directory and offer
//!    them through the regular transfer path, announced by a
//!    `RemoteDragEnter` + `RemoteDropConfirm` pair so the receiver accepts
//!    them without a prompt and reveals the first one.

use super::*;
use std::path::{Path, PathBuf};
use tetherly_core::drag::{DragCancelReason, DragControlMsg};
use tetherly_core::frame::FileMeta;
use tetherly_core::DeviceId;
use tokio::sync::oneshot;
use tracing::{info, warn};

/// How long an outgoing drag id is remembered to swallow the receiver's
/// `RemoteDropConfirm` echo.
const OUTGOING_TTL_MS: u64 = 60_000;
/// How long a requested (shelf fetch) offer is awaited.
const EXPECTED_TTL_MS: u64 = 120_000;
/// Files carried by one handoff (the drag control budget is per drop).
pub(crate) const HANDOFF_MAX_FILES: usize = 64;
/// Staged copies older than this are swept on the next staging.
const STAGE_SWEEP_MS: u64 = 6 * 60 * 60 * 1000;
/// Windows probe budget: the probe nudges for ~720 ms and posts back.
#[cfg(windows)]
const PROBE_WAIT_MS: u64 = 1_500;
/// Win32 virtual-key code of Escape (SystemInputSink takes VK codes on
/// every platform).
const VK_ESCAPE: u16 = 0x1B;

#[derive(Debug, Clone)]
struct ExpectedOffer {
    peer: DeviceId,
    name: String,
    size: u64,
    registered_ms: u64,
}

/// A file drag that left this (controller) machine and has not been
/// released yet.
#[derive(Debug, Clone)]
pub(crate) struct OutgoingDrag {
    pub(crate) peer: DeviceId,
    /// `None` while the probe is still running.
    pub(crate) files: Option<Vec<PathBuf>>,
    /// LMB went up on the remote screen before the probe answered.
    pub(crate) released: bool,
    /// The drag announced to the peer at the crossing (`RemoteDragEnter`),
    /// so its screen shows the carried files until release or return.
    pub(crate) announced: Option<u64>,
    /// v0.3.0: the receiver pulled the files for a native drag; they are
    /// (or will be) offered without waiting for the release.
    pub(crate) pulled: bool,
}

#[derive(Debug, Default)]
pub(crate) struct DragHandoffState {
    outgoing: HashMap<u64, u64>,
    expected: Vec<ExpectedOffer>,
    probe_waiters: HashMap<u64, oneshot::Sender<Vec<String>>>,
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    next_probe: u64,
    /// Physical LMB state while the seat is local (controller side).
    pub(crate) local_lmb_down: bool,
    /// Drag pasteboard change count captured while no drag could be in
    /// flight (macOS): at every LMB release on the controller (v0.2.99),
    /// at the injected LMB down on the controlled side.
    pub(crate) pb_baseline: Option<i64>,
    pub(crate) out: Option<OutgoingDrag>,
    /// Carries the cursor brought back (drag id → when). A pull honoured
    /// just before the return must not re-announce the carry afterwards:
    /// v0.3.2 field log, the late enter made the receiver take the same
    /// drag over twice (two helpers, `DoDragDrop` E_FAIL, Explorer popped).
    returned: HashMap<u64, u64>,
}

impl DragHandoffState {
    pub(crate) fn note_outgoing(&mut self, drag_id: u64, now: u64) {
        self.outgoing
            .retain(|_, at| now.saturating_sub(*at) <= OUTGOING_TTL_MS);
        self.outgoing.insert(drag_id, now);
    }

    pub(crate) fn note_returned(&mut self, drag_id: u64, now: u64) {
        self.returned
            .retain(|_, at| now.saturating_sub(*at) <= OUTGOING_TTL_MS);
        self.returned.insert(drag_id, now);
    }

    pub(crate) fn was_returned(&mut self, drag_id: u64, now: u64) -> bool {
        self.returned
            .retain(|_, at| now.saturating_sub(*at) <= OUTGOING_TTL_MS);
        self.returned.contains_key(&drag_id)
    }

    pub(crate) fn is_outgoing(&mut self, drag_id: u64, now: u64) -> bool {
        self.outgoing
            .retain(|_, at| now.saturating_sub(*at) <= OUTGOING_TTL_MS);
        self.outgoing.contains_key(&drag_id)
    }

    pub(crate) fn expect(&mut self, peer: &DeviceId, name: &str, size: u64, now: u64) {
        self.expected
            .retain(|e| now.saturating_sub(e.registered_ms) <= EXPECTED_TTL_MS);
        self.expected.push(ExpectedOffer {
            peer: peer.clone(),
            name: name.to_string(),
            size,
            registered_ms: now,
        });
    }

    /// Claim an inbound offer when every file in it was requested from that
    /// peer. Matched requests are consumed; a partial match claims nothing.
    pub(crate) fn claim(&mut self, peer: &DeviceId, files: &[FileMeta], now: u64) -> bool {
        self.expected
            .retain(|e| now.saturating_sub(e.registered_ms) <= EXPECTED_TTL_MS);
        if files.is_empty() {
            return false;
        }
        let mut taken: Vec<usize> = Vec::with_capacity(files.len());
        for f in files {
            let hit = self.expected.iter().enumerate().position(|(i, e)| {
                !taken.contains(&i) && e.peer == *peer && e.name == f.name && e.size == f.size
            });
            match hit {
                Some(i) => taken.push(i),
                None => return false,
            }
        }
        taken.sort_unstable();
        for i in taken.into_iter().rev() {
            self.expected.remove(i);
        }
        true
    }

    #[cfg_attr(target_os = "macos", allow(dead_code))]
    fn register_probe(&mut self) -> (u64, oneshot::Receiver<Vec<String>>) {
        self.next_probe = self.next_probe.wrapping_add(1).max(1);
        let id = self.next_probe;
        let (tx, rx) = oneshot::channel();
        self.probe_waiters.insert(id, tx);
        (id, rx)
    }
}

/// `RemoteDragEnter` for a carry announced before staging: names and sizes
/// straight from the dragged paths.
fn carry_enter(drag_id: u64, files: &[PathBuf]) -> DragControlMsg {
    let first_file_name = files
        .first()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    DragControlMsg::RemoteDragEnter {
        drag_id,
        file_count: files.len().min(HANDOFF_MAX_FILES) as u32,
        total_bytes: files
            .iter()
            .take(HANDOFF_MAX_FILES)
            .filter_map(|p| std::fs::metadata(p).ok())
            .map(|m| m.len())
            .sum(),
        first_file_name,
        norm_x: 0.5,
        norm_y: 0.5,
        has_directory: files.iter().any(|p| p.is_dir()),
    }
}

fn random_u64() -> u64 {
    let mut b = [0u8; 8];
    if getrandom::getrandom(&mut b).is_err() {
        return std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(1);
    }
    u64::from_le_bytes(b).max(1)
}

/// Human-readable failure for a source file the daemon cannot read. On
/// macOS the usual cause is TCC: a daemon without Full Disk Access cannot
/// open files under ~/Desktop, ~/Documents or ~/Downloads.
fn unreadable_message(src: &Path, e: &std::io::Error) -> String {
    if e.kind() == std::io::ErrorKind::PermissionDenied && cfg!(target_os = "macos") {
        format!(
            "无法读取 {}：需要在「系统设置 → 隐私与安全性 → 完全磁盘访问权限」中授权 Tetherly",
            src.display()
        )
    } else {
        format!("无法读取 {}：{e}", src.display())
    }
}

/// Copy (or hard-link) a user file into `<staging>/drag/<rand>/<name>` so
/// the transfer path, which only sends from the daemon-owned directory, can
/// offer it. Returns the staged path and the sanitized file name.
pub(crate) fn stage_user_file(
    staging_root: &Path,
    src: &Path,
    now_ms: u64,
) -> Result<(PathBuf, String), String> {
    let meta = std::fs::metadata(src).map_err(|e| unreadable_message(src, &e))?;
    if !meta.is_file() {
        return Err(format!("{} 不是普通文件（暂不支持文件夹）", src.display()));
    }
    if meta.len() > tetherly_core::FILE_MAX {
        return Err(format!("{} 超过单文件上限", src.display()));
    }
    let raw_name = src
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("file.bin");
    let name = tetherly_core::sanitize_file_name(raw_name).map_err(|e| e.to_string())?;
    let drag_root = staging_root.join("drag");
    sweep_stale_stages(&drag_root, now_ms);
    let dir = drag_root.join(format!("{:016x}", random_u64()));
    std::fs::create_dir_all(&dir).map_err(|e| format!("创建暂存目录失败：{e}"))?;
    let dst = dir.join(&name);
    if std::fs::hard_link(src, &dst).is_err() {
        std::fs::copy(src, &dst).map_err(|e| {
            let _ = std::fs::remove_dir_all(&dir);
            unreadable_message(src, &e)
        })?;
    }
    Ok((dst, name))
}

fn sweep_stale_stages(drag_root: &Path, now_ms: u64) {
    let Ok(rd) = std::fs::read_dir(drag_root) else {
        return;
    };
    for entry in rd.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_dir() {
            continue;
        }
        let modified_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        if now_ms.saturating_sub(modified_ms) > STAGE_SWEEP_MS {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

impl Node {
    /// The receiver echoes `RemoteDropConfirm` back to the sender; for a
    /// handoff this node started, the files are already on their way.
    pub(crate) fn is_outgoing_drag_echo(&self, drag_id: u64) -> bool {
        let now = self.now_ms();
        self.shared
            .drag_handoff
            .lock()
            .expect("drag_handoff")
            .is_outgoing(drag_id, now)
    }

    /// Remember that `peer` is about to offer `name`/`size` because this node
    /// asked for it (a shelf fetch), so the offer is accepted unprompted.
    pub(crate) fn expect_offer(&self, peer: &DeviceId, name: &str, size: u64) {
        let now = self.now_ms();
        self.shared
            .drag_handoff
            .lock()
            .expect("drag_handoff")
            .expect(peer, name, size, now);
    }

    pub(crate) fn claim_expected_offer(
        &self,
        peer: &DeviceId,
        t: &tetherly_core::IncomingTransfer,
    ) -> bool {
        let now = self.now_ms();
        self.shared
            .drag_handoff
            .lock()
            .expect("drag_handoff")
            .claim(peer, &t.files, now)
    }

    /// `POST /api/v1/drag/probe`: the Windows probe helper reports the files
    /// it saw. Returns false for an unknown (expired) probe id.
    pub fn complete_drag_probe(&self, probe_id: u64, paths: Vec<String>) -> bool {
        let tx = self
            .shared
            .drag_handoff
            .lock()
            .expect("drag_handoff")
            .probe_waiters
            .remove(&probe_id);
        match tx {
            Some(tx) => tx.send(paths).is_ok(),
            None => false,
        }
    }

    /// Capture the drag pasteboard change count before a drag can start
    /// (macOS); a probe later only trusts the pasteboard when it moved.
    pub(crate) fn record_drag_baseline(&self) {
        let cc = crate::platform::drag_probe::drag_pasteboard_change_count();
        self.shared
            .drag_handoff
            .lock()
            .expect("drag_handoff")
            .pb_baseline = cc;
    }

    /// The current pasteboard baseline (see [`DragHandoffState::pb_baseline`]).
    pub(crate) fn drag_baseline(&self) -> Option<i64> {
        self.shared
            .drag_handoff
            .lock()
            .expect("drag_handoff")
            .pb_baseline
    }

    /// Ask the OS which files the drag in flight under the cursor carries.
    /// `baseline` is the pasteboard count from before this gesture (macOS);
    /// callers snapshot it synchronously so a concurrent refresh cannot
    /// replace it with a post-drag value.
    #[cfg_attr(not(target_os = "macos"), allow(unused_variables))]
    pub(crate) async fn probe_drag_files(&self, baseline: Option<i64>) -> Vec<PathBuf> {
        #[cfg(target_os = "macos")]
        {
            let Some(baseline) = baseline else {
                info!("drag probe: no pasteboard baseline yet (first press after start)");
                return Vec::new();
            };
            let (cc, found, types) = tokio::task::spawn_blocking(move || {
                use crate::platform::drag_probe as p;
                let cc = p::drag_pasteboard_change_count();
                match cc {
                    Some(c) if c != baseline => {
                        let files = p::drag_pasteboard_files();
                        let types = if files.is_empty() {
                            p::drag_pasteboard_types()
                        } else {
                            Vec::new()
                        };
                        (cc, files, types)
                    }
                    _ => (cc, Vec::new(), Vec::new()),
                }
            })
            .await
            .unwrap_or_default();
            if found.is_empty() {
                // Tells "not a drag" (count unchanged) from "a drag with no
                // file URLs" (types say what it carried) after the fact.
                info!(
                    baseline,
                    change_count = ?cc,
                    types = ?types,
                    "drag probe: no file URLs on the drag pasteboard"
                );
            }
            found.into_iter().map(PathBuf::from).collect()
        }
        #[cfg(windows)]
        {
            let (id, rx) = self
                .shared
                .drag_handoff
                .lock()
                .expect("drag_handoff")
                .register_probe();
            if let Err(e) = crate::platform::drag_probe::spawn_drag_probe(self.ui_port(), id) {
                warn!(error = %e, "drag probe spawn failed");
                self.shared
                    .drag_handoff
                    .lock()
                    .expect("drag_handoff")
                    .probe_waiters
                    .remove(&id);
                return Vec::new();
            }
            let res =
                tokio::time::timeout(std::time::Duration::from_millis(PROBE_WAIT_MS), rx).await;
            self.shared
                .drag_handoff
                .lock()
                .expect("drag_handoff")
                .probe_waiters
                .remove(&id);
            match res {
                Ok(Ok(paths)) => paths.into_iter().map(PathBuf::from).collect(),
                _ => Vec::new(),
            }
        }
        #[cfg(not(any(target_os = "macos", windows)))]
        {
            let _ = self
                .shared
                .drag_handoff
                .lock()
                .expect("drag_handoff")
                .register_probe();
            Vec::new()
        }
    }

    /// Inject a local key tap / button release through the real sink
    /// (injected events bypass this node's own capture hooks).
    fn handoff_inject(&self, esc: bool, lmb_up: bool) {
        let Some(real) = &self.shared.input_real_sink else {
            return;
        };
        let Ok(mut sink) = real.lock() else {
            return;
        };
        if esc {
            sink.key(VK_ESCAPE, true, 0);
            sink.key(VK_ESCAPE, false, 0);
        }
        if lmb_up {
            sink.button(tetherly_core::BTN_LEFT, false);
        }
    }

    /// Target side: cancel the local drag the controller is carrying away
    /// (Esc) before its Leave releases the button over the edge.
    pub(crate) fn cancel_local_drag(&self) {
        self.handoff_inject(true, false);
    }

    /// Controller side: physical LMB observed (any seat).
    pub(crate) fn drag_handoff_on_lmb(&self, seat: CursorSeat, down: bool) {
        // v0.2.99: the baseline is refreshed on every RELEASE (any seat).
        // No OS drag can start between a release and the next press, so the
        // count read here predates the next gesture's drag however late the
        // blocking read runs. Reading it at the press (pre-v0.2.99) raced
        // the drag itself: a delayed read saw the post-drag count, and the
        // probe reported "no files" (production 2026-11-26, daemon under
        // launchd Background throttling). A press only fills a missing one.
        // Only macOS has a drag pasteboard; elsewhere the baseline stays None.
        let refresh = cfg!(target_os = "macos")
            && (!down
                || self
                    .shared
                    .drag_handoff
                    .lock()
                    .expect("drag_handoff")
                    .pb_baseline
                    .is_none());
        if refresh {
            // Off the hook thread: the pasteboard read is an IPC.
            let node = self.clone_handle();
            let rt = tokio::runtime::Handle::try_current()
                .ok()
                .or_else(|| self.shared.rt_handle.lock().expect("rt_handle").clone());
            if let Some(h) = rt {
                h.spawn_blocking(move || node.record_drag_baseline());
            }
        }
        if seat != CursorSeat::Remote {
            let mut st = self.shared.drag_handoff.lock().expect("drag_handoff");
            st.local_lmb_down = down;
            if down {
                st.out = None;
            }
            return;
        }
        if down {
            return;
        }
        let mut st = self.shared.drag_handoff.lock().expect("drag_handoff");
        st.local_lmb_down = false;
        let Some(mut out) = st.out.take() else {
            return;
        };
        if out.pulled {
            // v0.3.0: already offered. The receiver's drag source sees the
            // forwarded button-up itself; the confirm is the backstop for a
            // release that beat the helper's own press.
            drop(st);
            if let Some(drag_id) = out.announced {
                let node = self.clone_handle();
                let rt = tokio::runtime::Handle::try_current()
                    .ok()
                    .or_else(|| self.shared.rt_handle.lock().expect("rt_handle").clone());
                if let Some(h) = rt {
                    h.spawn(async move {
                        let confirm = DragControlMsg::RemoteDropConfirm {
                            drag_id,
                            drop_norm_x: 0.5,
                            drop_norm_y: 0.5,
                        };
                        if let Err(e) = node.send_drag_control(&out.peer, &confirm).await {
                            warn!(peer = %out.peer, error = %e, "native drag: release notice failed");
                        }
                    });
                }
            }
            return;
        }
        match out.files.take() {
            Some(files) => {
                drop(st);
                self.spawn_handoff(out.peer, files, out.announced);
            }
            None => {
                out.released = true;
                st.out = Some(out);
            }
        }
    }

    /// Controller side: the seat just moved to Remote. With the left button
    /// held this may be a file drag leaving the screen.
    pub(crate) fn drag_handoff_on_enter(&self) {
        let (peer, baseline) = {
            let mut st = self.shared.drag_handoff.lock().expect("drag_handoff");
            if !st.local_lmb_down {
                return;
            }
            let Some(peer) = self.shared.input_peer.lock().expect("input_peer").clone() else {
                return;
            };
            st.out = Some(OutgoingDrag {
                peer: peer.clone(),
                files: None,
                released: false,
                announced: None,
                pulled: false,
            });
            (peer, st.pb_baseline)
        };
        let node = self.clone_handle();
        let rt = tokio::runtime::Handle::try_current()
            .ok()
            .or_else(|| self.shared.rt_handle.lock().expect("rt_handle").clone());
        let Some(h) = rt else { return };
        h.spawn(async move {
            let files = node.probe_drag_files(baseline).await;
            // The physical release never reaches this machine's OS while the
            // seat is remote: end the local drag (Esc so nothing drops at
            // the edge) and release the button it still believes is held.
            node.handoff_inject(!files.is_empty(), true);
            if files.is_empty() {
                info!(peer = %peer, "lmb held across the edge; no dragged files on the pasteboard");
            }
            let now = node.now_ms();
            // Still held: tell the peer what is being carried now, so its
            // screen shows the files while the cursor moves over it. The
            // files are published only after the enter is queued, so a
            // release in between hands off behind it (frames keep order).
            let announce = {
                let mut st = node.shared.drag_handoff.lock().expect("drag_handoff");
                let Some(mut out) = st.out.take() else {
                    return; // cancelled (returned to local) while probing
                };
                if out.peer != peer || files.is_empty() {
                    return;
                }
                info!(peer = %peer, count = files.len(), "file drag left the screen");
                if out.released {
                    None
                } else {
                    let drag_id = random_u64();
                    out.announced = Some(drag_id);
                    st.out = Some(out);
                    st.note_outgoing(drag_id, now);
                    Some(drag_id)
                }
            };
            let Some(drag_id) = announce else {
                node.spawn_handoff(peer, files, None);
                return;
            };
            let enter = carry_enter(drag_id, &files);
            if let Err(e) = node.send_drag_control(&peer, &enter).await {
                warn!(peer = %peer, error = %e, "drag handoff: carry announce failed");
            }
            let released = {
                let mut st = node.shared.drag_handoff.lock().expect("drag_handoff");
                let Some(mut out) = st.out.take() else {
                    return; // returned to local meanwhile (cancel sent there)
                };
                if out.released {
                    true
                } else if out.pulled {
                    // The pull overtook the files: hand off right now.
                    st.out = Some(out);
                    drop(st);
                    node.spawn_pulled_handoff(peer, files, drag_id);
                    return;
                } else {
                    out.files = Some(files.clone());
                    st.out = Some(out);
                    false
                }
            };
            if released {
                node.spawn_handoff(peer, files, Some(drag_id));
            }
        });
    }

    /// Controller side: the cursor came back without releasing.
    pub(crate) fn drag_handoff_on_return(&self) {
        let now = self.now_ms();
        let out = {
            let mut st = self.shared.drag_handoff.lock().expect("drag_handoff");
            let out = st.out.take();
            if let Some(id) = out.as_ref().and_then(|o| o.announced) {
                st.note_returned(id, now);
            }
            out
        };
        let Some(OutgoingDrag {
            peer,
            announced: Some(drag_id),
            ..
        }) = out
        else {
            return;
        };
        let node = self.clone_handle();
        let rt = tokio::runtime::Handle::try_current()
            .ok()
            .or_else(|| self.shared.rt_handle.lock().expect("rt_handle").clone());
        if let Some(h) = rt {
            h.spawn(async move {
                let cancel = DragControlMsg::RemoteDragCancel {
                    drag_id,
                    reason: DragCancelReason::DraggedBackToLocal,
                };
                if let Err(e) = node.send_drag_control(&peer, &cancel).await {
                    warn!(peer = %peer, error = %e, "drag handoff: carry cancel failed");
                }
            });
        }
    }

    /// Controller side: `RemoteDragPull` — the receiver turned the carry
    /// into a native drag and wants the bytes before the release.
    pub(crate) fn drag_handoff_on_pull(&self, peer: &DeviceId, drag_id: u64) {
        let files = {
            let mut st = self.shared.drag_handoff.lock().expect("drag_handoff");
            let Some(out) = st
                .out
                .as_mut()
                .filter(|o| o.peer == *peer && o.announced == Some(drag_id))
            else {
                warn!(peer = %peer, drag_id, "drag pull for an unknown carry");
                return;
            };
            if out.pulled {
                return;
            }
            out.pulled = true;
            // `None`: the probe result is still being published; the enter
            // task hands off when it stores the files.
            out.files.take()
        };
        if let Some(files) = files {
            self.spawn_pulled_handoff(peer.clone(), files, drag_id);
        }
    }

    fn spawn_pulled_handoff(&self, peer: DeviceId, files: Vec<PathBuf>, drag_id: u64) {
        let node = self.clone_handle();
        let rt = tokio::runtime::Handle::try_current()
            .ok()
            .or_else(|| self.shared.rt_handle.lock().expect("rt_handle").clone());
        if let Some(h) = rt {
            h.spawn(async move {
                node.hand_off_drag_files_with(&peer, files, Some(drag_id), false)
                    .await
            });
        }
    }

    pub(crate) fn spawn_handoff(&self, peer: DeviceId, files: Vec<PathBuf>, drag_id: Option<u64>) {
        let node = self.clone_handle();
        let rt = tokio::runtime::Handle::try_current()
            .ok()
            .or_else(|| self.shared.rt_handle.lock().expect("rt_handle").clone());
        if let Some(h) = rt {
            h.spawn(async move { node.hand_off_drag_files(&peer, files, drag_id).await });
        }
    }

    /// Stage the dragged files and offer them to `peer` as one drop. `drag_id`
    /// is the carry already announced at the crossing, if any; the enter is
    /// re-sent with the staged totals either way (it also refreshes a carry
    /// the peer may have timed out while the button was held).
    pub(crate) async fn hand_off_drag_files(
        &self,
        peer: &DeviceId,
        paths: Vec<PathBuf>,
        drag_id: Option<u64>,
    ) {
        self.hand_off_drag_files_with(peer, paths, drag_id, true)
            .await
    }

    /// `confirm == false` is the v0.3.0 pull: the button is still held, so
    /// only the refreshed enter (final count) precedes the offers.
    async fn hand_off_drag_files_with(
        &self,
        peer: &DeviceId,
        paths: Vec<PathBuf>,
        drag_id: Option<u64>,
        confirm: bool,
    ) {
        let staging = self.send_staging_dir();
        let now = self.now_ms();
        let staged: Vec<Result<(PathBuf, String, u64), String>> =
            tokio::task::spawn_blocking(move || {
                paths
                    .into_iter()
                    .take(HANDOFF_MAX_FILES)
                    .map(|src| {
                        let (dst, name) = stage_user_file(&staging, &src, now)?;
                        let size = std::fs::metadata(&dst).map(|m| m.len()).unwrap_or(0);
                        Ok((dst, name, size))
                    })
                    .collect()
            })
            .await
            .unwrap_or_default();
        let mut ok: Vec<(PathBuf, String, u64)> = Vec::new();
        for r in staged {
            match r {
                Ok(v) => ok.push(v),
                Err(e) => {
                    warn!(peer = %peer, error = %e, "drag handoff: file skipped");
                    self.emit(UiEvent::Status { message: e });
                }
            }
        }
        if ok.is_empty() {
            return;
        }
        let drag_id = drag_id.unwrap_or_else(random_u64);
        {
            let mut st = self.shared.drag_handoff.lock().expect("drag_handoff");
            if !confirm && st.was_returned(drag_id, self.now_ms()) {
                // The pull raced the cursor coming back: the carry is over.
                info!(peer = %peer, drag_id, "drag handoff: pulled carry returned; not offered");
                return;
            }
            st.note_outgoing(drag_id, now);
        }
        let enter = DragControlMsg::RemoteDragEnter {
            drag_id,
            file_count: ok.len() as u32,
            total_bytes: ok.iter().map(|f| f.2).sum(),
            first_file_name: ok[0].1.clone(),
            norm_x: 0.5,
            norm_y: 0.5,
            has_directory: false,
        };
        let confirm_msg = DragControlMsg::RemoteDropConfirm {
            drag_id,
            drop_norm_x: 0.5,
            drop_norm_y: 0.5,
        };
        let msgs: &[&DragControlMsg] = if confirm {
            &[&enter, &confirm_msg]
        } else {
            &[&enter]
        };
        for msg in msgs {
            if let Err(e) = self.send_drag_control(peer, msg).await {
                warn!(peer = %peer, error = %e, "drag handoff: control send failed");
                return;
            }
        }
        info!(peer = %peer, drag_id, count = ok.len(), "drag handoff: offering files");
        // The offer only announces each file; the bytes are read when the
        // peer accepts (and again on a resume). The staged copy stays until
        // `sweep_stale_stages` ages it out.
        for (path, name, _) in ok {
            if let Err(e) = self.offer_and_send_file(peer, &path, Some(&name)).await {
                warn!(peer = %peer, file = %name, error = %e, "drag handoff: send failed");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(name: &str, size: u64) -> FileMeta {
        FileMeta {
            name: name.into(),
            size,
            sha256: String::new(),
        }
    }

    #[test]
    fn expected_offer_claims_only_full_matches_from_that_peer() {
        let a = DeviceId::from_id_pk(&[1u8; 32]);
        let b = DeviceId::from_id_pk(&[2u8; 32]);
        let mut st = DragHandoffState::default();
        st.expect(&a, "x.txt", 5, 1_000);
        assert!(!st.claim(&b, &[meta("x.txt", 5)], 1_001), "other peer");
        assert!(!st.claim(&a, &[meta("x.txt", 6)], 1_001), "size differs");
        assert!(
            !st.claim(&a, &[meta("x.txt", 5), meta("y", 1)], 1_001),
            "partial match claims nothing"
        );
        assert!(st.claim(&a, &[meta("x.txt", 5)], 1_002));
        assert!(!st.claim(&a, &[meta("x.txt", 5)], 1_003), "consumed");
        st.expect(&a, "late", 1, 0);
        assert!(
            !st.claim(&a, &[meta("late", 1)], EXPECTED_TTL_MS + 1),
            "expired"
        );
    }

    #[test]
    fn outgoing_echo_is_remembered_then_expires() {
        let mut st = DragHandoffState::default();
        st.note_outgoing(9, 100);
        assert!(st.is_outgoing(9, 200));
        assert!(!st.is_outgoing(8, 200));
        assert!(!st.is_outgoing(9, 100 + OUTGOING_TTL_MS + 1));
    }

    #[test]
    fn staging_copies_regular_files_and_rejects_directories() {
        let root = std::env::temp_dir().join(format!("tetherly-stage-{:x}", random_u64()));
        let src_dir = root.join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        let src = src_dir.join("报告 v1.txt");
        std::fs::write(&src, b"hello").unwrap();
        let staging = root.join("send");
        let (dst, name) = stage_user_file(&staging, &src, 0).unwrap();
        assert!(dst.starts_with(staging.join("drag")));
        assert_eq!(std::fs::read(&dst).unwrap(), b"hello");
        assert_eq!(
            dst.file_name().and_then(|n| n.to_str()),
            Some(name.as_str())
        );
        assert!(stage_user_file(&staging, &src_dir, 0).is_err());
        assert!(stage_user_file(&staging, &root.join("missing"), 0).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    /// §240: the offer only announces the file; its bytes are read when the
    /// peer accepts, later. The staged copy must outlive the offer call
    /// (production: "file data send failed: No such file or directory").
    /// v0.2.99: the pasteboard baseline is taken at release, never
    /// overwritten at a press (where it raced the drag it guards).
    #[cfg(target_os = "macos")]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn baseline_refreshes_on_release_not_on_press() {
        let (node, dir) = crate::runtime::clipboard_policy_tests::temp_test_node("drag-baseline");
        let wait = || tokio::time::sleep(std::time::Duration::from_millis(300));
        node.shared.drag_handoff.lock().unwrap().pb_baseline = Some(-7);
        node.drag_handoff_on_lmb(CursorSeat::Local, true);
        wait().await;
        assert_eq!(
            node.drag_baseline(),
            Some(-7),
            "press keeps the release baseline"
        );
        node.drag_handoff_on_lmb(CursorSeat::Local, false);
        wait().await;
        let after = node.drag_baseline();
        assert!(
            after.is_some() && after != Some(-7),
            "release re-reads the count: {after:?}"
        );
        // A press with no baseline yet (first gesture after start) fills it.
        node.shared.drag_handoff.lock().unwrap().pb_baseline = None;
        node.drag_handoff_on_lmb(CursorSeat::Local, true);
        wait().await;
        assert!(node.drag_baseline().is_some());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// v0.3.0: a receiver pull offers the carried files at once (enter, no
    /// confirm); the later release only sends the confirm, never a second
    /// set of offers.
    #[tokio::test]
    async fn pull_offers_once_and_release_only_confirms() {
        use tetherly_core::drag::DragControlMsg;
        let (node, dir) = crate::runtime::clipboard_policy_tests::temp_test_node("drag-pull");
        let peer = DeviceId::from_id_pk(&[43u8; 32]);
        let (tx, mut rx) = tokio::sync::mpsc::channel::<tetherly_core::frame::InnerFrame>(64);
        let live = crate::runtime::LivePeer {
            device_id: peer.clone(),
            name: "pull-peer".into(),
            platform: "test".into(),
            addr: "127.0.0.1:1".parse().unwrap(),
            handshake_hash: [4u8; 32],
            file_port: None,
            input_port: None,
            screen_port: None,
            caps: vec![tetherly_core::drag::DRAG_PULL_CAP.into()],
            path: tetherly_core::PathKind::Lan,
            rtt_ms: None,
            last_seen_ms: 0,
        };
        node.register_session(live, tx).await;
        let src = dir.join("pulled.txt");
        std::fs::write(&src, b"bytes").unwrap();
        node.shared.drag_handoff.lock().unwrap().out = Some(OutgoingDrag {
            peer: peer.clone(),
            files: Some(vec![src]),
            released: false,
            announced: Some(77),
            pulled: false,
        });
        node.drag_handoff_on_pull(&DeviceId::from_id_pk(&[1u8; 32]), 77);
        assert!(
            !node
                .shared
                .drag_handoff
                .lock()
                .unwrap()
                .out
                .as_ref()
                .unwrap()
                .pulled,
            "a pull from another peer is ignored"
        );
        node.drag_handoff_on_pull(&peer, 77);
        let offers = || node.shared.pending_out.lock().unwrap().len();
        for _ in 0..200 {
            if offers() == 1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(offers(), 1, "pulled files offered before the release");
        node.drag_handoff_on_lmb(CursorSeat::Remote, false);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(offers(), 1, "release does not offer again");
        let mut msgs = Vec::new();
        while let Ok(f) = rx.try_recv() {
            if let Ok(m) = DragControlMsg::from_payload(&f.payload) {
                msgs.push(m);
            }
        }
        let confirms = msgs
            .iter()
            .filter(|m| matches!(m, DragControlMsg::RemoteDropConfirm { drag_id: 77, .. }))
            .count();
        let enters = msgs
            .iter()
            .filter(|m| matches!(m, DragControlMsg::RemoteDragEnter { drag_id: 77, .. }))
            .count();
        assert_eq!((enters, confirms), (1, 1), "{msgs:?}");
        let _ = std::fs::remove_dir_all(dir);
    }

    /// A pull honoured just before the cursor came back must not announce
    /// the carry again after the return's cancel.
    #[tokio::test]
    async fn pull_racing_the_return_offers_nothing() {
        use tetherly_core::drag::DragControlMsg;
        let (node, dir) = crate::runtime::clipboard_policy_tests::temp_test_node("drag-pull-ret");
        let peer = DeviceId::from_id_pk(&[44u8; 32]);
        let (tx, mut rx) = tokio::sync::mpsc::channel::<tetherly_core::frame::InnerFrame>(64);
        let live = crate::runtime::LivePeer {
            device_id: peer.clone(),
            name: "pull-peer".into(),
            platform: "test".into(),
            addr: "127.0.0.1:1".parse().unwrap(),
            handshake_hash: [4u8; 32],
            file_port: None,
            input_port: None,
            screen_port: None,
            caps: vec![tetherly_core::drag::DRAG_PULL_CAP.into()],
            path: tetherly_core::PathKind::Lan,
            rtt_ms: None,
            last_seen_ms: 0,
        };
        node.register_session(live, tx).await;
        let src = dir.join("returned.txt");
        std::fs::write(&src, b"bytes").unwrap();
        node.shared.drag_handoff.lock().unwrap().out = Some(OutgoingDrag {
            peer: peer.clone(),
            files: None,
            released: false,
            announced: Some(78),
            pulled: true,
        });
        node.drag_handoff_on_return();
        // The pulled handoff that was already under way runs after it.
        node.hand_off_drag_files_with(&peer, vec![src], Some(78), false)
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(node.shared.pending_out.lock().unwrap().len(), 0, "no offer");
        let mut msgs = Vec::new();
        while let Ok(f) = rx.try_recv() {
            if let Ok(m) = DragControlMsg::from_payload(&f.payload) {
                msgs.push(m);
            }
        }
        assert!(
            !msgs
                .iter()
                .any(|m| matches!(m, DragControlMsg::RemoteDragEnter { .. })),
            "no late enter: {msgs:?}"
        );
        assert!(
            msgs.iter()
                .any(|m| matches!(m, DragControlMsg::RemoteDragCancel { drag_id: 78, .. })),
            "the return still cancels: {msgs:?}"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn handoff_keeps_the_staged_file_until_the_peer_pulls_it() {
        use crate::runtime::storage::OutboundSource;
        let (node, dir) =
            crate::runtime::clipboard_policy_tests::temp_test_node("drag-handoff-stage");
        let peer = DeviceId::from_id_pk(&[42u8; 32]);
        let (tx, _rx) = tokio::sync::mpsc::channel::<tetherly_core::frame::InnerFrame>(16);
        let live = crate::runtime::LivePeer {
            device_id: peer.clone(),
            name: "drop-peer".into(),
            platform: "test".into(),
            addr: "127.0.0.1:1".parse().unwrap(),
            handshake_hash: [3u8; 32],
            file_port: None,
            input_port: None,
            screen_port: None,
            caps: Vec::new(),
            path: tetherly_core::PathKind::Lan,
            rtt_ms: None,
            last_seen_ms: 0,
        };
        node.register_session(live, tx).await;
        let src = dir.join("dragged.txt");
        std::fs::write(&src, b"payload").unwrap();

        node.hand_off_drag_files(&peer, vec![src], None).await;

        let sources: Vec<PathBuf> = node
            .shared
            .pending_out
            .lock()
            .unwrap()
            .values()
            .filter_map(|o| match &o.source {
                OutboundSource::Path(p) => Some(p.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(sources.len(), 1, "one offer pending");
        assert_eq!(std::fs::read(&sources[0]).unwrap(), b"payload");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test]
    async fn probe_completion_routes_paths_to_the_waiter() {
        let (node, _dir) =
            crate::runtime::clipboard_policy_tests::temp_test_node("drag-probe-complete");
        let (id, rx) = node.shared.drag_handoff.lock().unwrap().register_probe();
        assert!(node.complete_drag_probe(id, vec!["C:\\a.txt".into()]));
        assert_eq!(rx.await.unwrap(), vec!["C:\\a.txt".to_string()]);
        assert!(!node.complete_drag_probe(id, vec![]), "single use");
    }
}
