// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Linux/BSD X11 screen capture and presentation via `x11rb`. Self-contained:
//! it imports only absolute paths so this file compiles for its own target on
//! its own, the same way `cred/linux.rs` does.
//!
//! Capture reads the root window as a 24- or 32-bit ZPixmap, whose little-
//! endian layout is B,G,R,X — `PixelFormat::Bgra8` once the unused high byte is
//! filled opaque. Two transports share that decode:
//!
//! - **MIT-SHM** (`shm_create_segment` + `shm_get_image`): the X server writes
//!   the pixels straight into a segment we `mmap`, so a full-resolution frame
//!   never crosses the socket. This is the only path usable at frame rate for
//!   1080p and above.
//! - **plain `GetImage`** as a fallback for servers without usable SHM (remote
//!   X, hardened servers, fd-passing restrictions): the pixels come back in the
//!   reply, at the cost of a socket copy.
//!
//! Presentation copies each frame into an override-redirect window with
//! `PutImage` — a window the window manager ignores, which is what a plain
//! viewer wants.
//!
//! Wayland is *not* a supported target: it has no X11 root window to read and
//! the portal route is a separate iteration (spec §9.7). A Wayland session that
//! runs XWayland is captured through that X11 root; a pure Wayland session has
//! no capture path, and the connection failure is reported verbatim rather than
//! dressed up as a blank frame.

use std::sync::{Mutex, OnceLock};

use tetherly_core::{CoreError, PixelFormat, ScreenFrame, ScreenRect, ScreenSource};

use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::shm::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{self, ConnectionExt as _, CreateGCAux, CreateWindowAux, EventMask};
use x11rb::protocol::xproto::{ImageFormat, WindowClass};
use x11rb::rust_connection::RustConnection;

/// Bytes per pixel for every v1 screen format.
const BPP: usize = 4;

/// Connection plus the default screen number it refers to.
#[derive(Debug)]
struct X11 {
    conn: RustConnection,
    screen_num: usize,
}

impl X11 {
    fn connect() -> Result<Self, CoreError> {
        let (conn, screen_num) = x11rb::connect(None).map_err(|e| {
            CoreError::ScreenRefused(format!(
                "no X11 display ({e}); a Wayland session without XWayland has no capture path"
            ))
        })?;
        Ok(Self { conn, screen_num })
    }

    /// Root window, size and depth of the default screen.
    ///
    /// Refuses anything that cannot be read as BGRA: a MSB-first image byte
    /// order would give R,G,B,A in memory instead, and a root depth outside
    /// 24/32 has no per-pixel colour layout worth decoding.
    fn root(&self) -> Result<(u32, u16, u16, u8), CoreError> {
        let setup = self.conn.setup();
        if setup.image_byte_order != xproto::ImageOrder::LSB_FIRST {
            return Err(CoreError::ScreenRefused(
                "X11 image byte order is MSB-first; cannot decode as BGRA".into(),
            ));
        }
        let screen = setup.roots.get(self.screen_num).ok_or_else(|| {
            CoreError::ScreenRefused(format!("no X11 screen {}", self.screen_num))
        })?;
        if !matches!(screen.root_depth, 24 | 32) {
            return Err(CoreError::ScreenRefused(format!(
                "X11 root depth {} is not 24/32; cannot capture as BGRA",
                screen.root_depth
            )));
        }
        Ok((
            screen.root,
            screen.width_in_pixels,
            screen.height_in_pixels,
            screen.root_depth,
        ))
    }
}

/// The shared capture connection, opened lazily and reused. `RustConnection`
/// keeps its socket and protocol state behind its own mutexes, so one static
/// serves every caller.
fn capture_slot() -> &'static Mutex<Option<X11>> {
    static SLOT: OnceLock<Mutex<Option<X11>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// Fill the unused high byte of each 32-bit pixel opaque.
///
/// A 24-bit root leaves that byte undefined (BGRX); depth 32 already carries
/// real alpha and is left alone.
fn fix_alpha(bytes: &mut [u8], depth: u8) {
    if depth == 32 {
        return;
    }
    for alpha in bytes.iter_mut().skip(BPP - 1).step_by(BPP) {
        *alpha = 0xFF;
    }
}

/// Rearrange a `stride`-wide buffer into one packed BGRA frame.
///
/// The X server pads each scanline to the pixmap format's scanline pad, so the
/// padding is dropped here; a buffer too short for the rows it claims is
/// refused rather than read out of bounds.
fn to_frame(
    data: &[u8],
    w: u16,
    h: u16,
    depth: u8,
    stride: usize,
) -> Result<ScreenFrame, CoreError> {
    let row = w as usize * BPP;
    let need = stride
        .checked_mul(h as usize)
        .ok_or_else(|| CoreError::ScreenRefused("X11 stride overflows".into()))?;
    if stride < row || data.len() < need {
        return Err(CoreError::ScreenRefused(format!(
            "X11 buffer is {} bytes, need {need} for stride {stride}",
            data.len()
        )));
    }
    let mut bytes = Vec::with_capacity(row * h as usize);
    if stride == row {
        bytes.extend_from_slice(&data[..row * h as usize]);
    } else {
        for y in 0..h as usize {
            let start = y * stride;
            bytes.extend_from_slice(&data[start..start + row]);
        }
    }
    fix_alpha(&mut bytes, depth);
    let rect = ScreenRect::new(0, 0, w, h)?;
    ScreenFrame::raw(0, rect, PixelFormat::Bgra8, bytes)
}

/// A mapped MIT-SHM segment. `munmap` runs on drop, so the mapping never
/// outlives the grab that made it.
struct ShmSegment {
    addr: *mut u8,
    len: usize,
}

impl ShmSegment {
    /// Map `fd` for `len` bytes read-only.
    ///
    /// Read-only is enough because the server is the writer, and it keeps a
    /// stray write from corrupting the segment underneath the X server.
    fn map(fd: &std::os::unix::io::OwnedFd, len: usize) -> Result<Self, CoreError> {
        use std::os::unix::io::AsRawFd;
        // Safety: `fd` is a live descriptor the reply keeps open for the whole
        // call and `len` is the segment size the server just created, so the
        // request is in range. `MAP_FAILED` is checked before the pointer is
        // stored or used.
        let addr = unsafe {
            libc::mmap(
                std::ptr::null_mut(),
                len,
                libc::PROT_READ,
                libc::MAP_SHARED,
                fd.as_raw_fd(),
                0,
            )
        };
        if addr == libc::MAP_FAILED {
            return Err(CoreError::ScreenRefused(
                "mmap of the MIT-SHM segment failed".into(),
            ));
        }
        Ok(Self {
            addr: addr as *mut u8,
            len,
        })
    }

    /// Run `f` over the first `n` mapped bytes.
    fn with_bytes<T>(&self, n: usize, f: impl FnOnce(&[u8]) -> T) -> Result<T, CoreError> {
        if n > self.len {
            return Err(CoreError::ScreenRefused(format!(
                "SHM image is {n} bytes, segment is {}",
                self.len
            )));
        }
        // Safety: `addr` is a live mapping of `len` bytes (checked at map time)
        // and `n <= len`, so this slice is in bounds. The bytes were written by
        // the X server and `shm_detach` has already returned, so no writer is
        // active; this process only ever maps the segment read-only.
        let slice = unsafe { std::slice::from_raw_parts(self.addr, n) };
        Ok(f(slice))
    }
}

impl Drop for ShmSegment {
    fn drop(&mut self) {
        // Safety: `addr`/`len` are exactly what `map` returned and the mapping
        // is private to this struct, so this is a single in-bounds munmap.
        unsafe {
            libc::munmap(self.addr as *mut libc::c_void, self.len);
        }
    }
}

/// Grab the whole screen through MIT-SHM.
///
/// `Ok(None)` means SHM is unavailable or unusable and the caller should fall
/// back to plain `GetImage`; an `Err` means SHM was actually used and failed,
/// which is a capture failure rather than a missing capability.
fn grab_shm(x11: &X11) -> Result<Option<ScreenFrame>, CoreError> {
    let (root, w, h, depth) = x11.root()?;
    if w == 0 || h == 0 {
        return Ok(None);
    }
    let size = w as usize * h as usize * BPP;
    // The protocol carries the segment length as u32; refuse rather than
    // truncate on a display so large it cannot be expressed.
    let size_u32 = u32::try_from(size).map_err(|_| {
        CoreError::ScreenRefused(format!(
            "X11 screen of {size} bytes exceeds a u32 SHM segment"
        ))
    })?;

    // Presence of the extension is not the same as fd passing being allowed:
    // split segments arrived with MIT-SHM 1.2.
    let usable = match x11.conn.extension_information(shm::X11_EXTENSION_NAME) {
        Ok(Some(_)) => match x11
            .conn
            .shm_query_version()
            .ok()
            .and_then(|cookie| cookie.reply().ok())
        {
            Some(v) => v.major_version > 1 || (v.major_version == 1 && v.minor_version >= 2),
            None => false,
        },
        _ => false,
    };
    if !usable {
        return Ok(None);
    }

    let shmseg = x11
        .conn
        .generate_id()
        .map_err(|e| CoreError::ScreenRefused(format!("generate shmseg id: {e}")))?;
    // `read_only = false`: the server is the writer. A failure here is the
    // server refusing fd passing (container/sandbox policy), which is a
    // capability gap, not a capture error.
    let segment = match x11
        .conn
        .shm_create_segment(shmseg, size_u32, false)
        .ok()
        .and_then(|cookie| cookie.reply().ok())
    {
        Some(reply) => reply,
        None => return Ok(None),
    };

    let seg = ShmSegment::map(&segment.shm_fd, size)?;
    let image = x11
        .conn
        .shm_get_image(
            root,
            0,
            0,
            w,
            h,
            u32::MAX,
            u8::from(ImageFormat::Z_PIXMAP),
            shmseg,
            0,
        )
        .map_err(|e| CoreError::ScreenRefused(format!("shm_get_image: {e}")))?;
    let reply = image
        .reply()
        .map_err(|e| CoreError::ScreenRefused(format!("shm_get_image: {e}")))?;
    // The server keeps a reference to the segment until it detaches, so this
    // must happen before the fd and the mapping go away.
    let _ = x11.conn.shm_detach(shmseg);

    // Read `reply.size` bytes, not the whole segment: the server reports how
    // much it actually wrote, and a short count would otherwise leave stale
    // bytes from the previous frame in the tail of this one.
    let written = reply.size as usize;
    let stride = written
        .checked_div(h as usize)
        .filter(|s| *s >= w as usize * BPP)
        .ok_or_else(|| {
            CoreError::ScreenRefused(format!(
                "shm_get_image reported {written} bytes for {w}x{h}"
            ))
        })?;
    seg.with_bytes(written, |data| to_frame(data, w, h, depth, stride))?
        .map(Some)
}

/// Grab the whole screen through plain `GetImage`. Used only when MIT-SHM is
/// unavailable.
fn grab_plain(x11: &X11) -> Result<Option<ScreenFrame>, CoreError> {
    let (root, w, h, depth) = x11.root()?;
    if w == 0 || h == 0 {
        return Ok(None);
    }
    let image = x11
        .conn
        .get_image(ImageFormat::Z_PIXMAP, root, 0, 0, w, h, u32::MAX)
        .map_err(|e| {
            CoreError::ScreenRefused(format!(
                "x11 get_image: {e} (a full-resolution GetImage can exceed the server reply limit)"
            ))
        })?;
    let reply = image.reply().map_err(|e| {
        CoreError::ScreenRefused(format!(
            "x11 get_image: {e} (a full-resolution GetImage can exceed the server reply limit)"
        ))
    })?;
    // Derive the stride from what actually arrived rather than assuming rows
    // are packed; `to_frame` refuses a stride narrower than a row.
    let stride = reply.data.len() / h as usize;
    to_frame(&reply.data, w, h, depth, stride).map(Some)
}

/// X11 root-window capture source.
#[derive(Debug, Clone, Default)]
pub struct X11Source;

impl X11Source {
    pub fn new() -> Self {
        Self
    }
}

impl ScreenSource for X11Source {
    fn grab(&self) -> Result<Option<ScreenFrame>, CoreError> {
        let mut guard = capture_slot()
            .lock()
            .map_err(|_| CoreError::ScreenRefused("x11 capture mutex poisoned".into()))?;
        if guard.is_none() {
            *guard = Some(X11::connect()?);
        }
        let x11 = guard
            .as_ref()
            .ok_or_else(|| CoreError::ScreenRefused("no x11 connection".into()))?;
        match grab_shm(x11)? {
            Some(frame) => Ok(Some(frame)),
            None => grab_plain(x11),
        }
    }
}

/// A presentation window with its graphics context.
#[derive(Debug)]
struct Sink {
    conn: RustConnection,
    window: u32,
    gc: u32,
    w: u16,
    h: u16,
    depth: u8,
}

impl Sink {
    /// Map an override-redirect `w`x`h` window and a graphics context for it.
    /// The window manager ignores override-redirect windows, which is what a
    /// plain viewer wants: no decorations, no placement fight.
    fn open(w: u16, h: u16) -> Result<Self, CoreError> {
        let (conn, screen_num) = x11rb::connect(None)
            .map_err(|e| CoreError::ScreenRefused(format!("no X11 display for present: {e}")))?;
        let screen = &conn.setup().roots[screen_num];
        let root = screen.root;
        // The window inherits the root's depth (`COPY_DEPTH_FROM_PARENT`), and
        // `PutImage` must be told the drawable's real depth or the server
        // rejects it with `BadMatch`. Capturing it here is what keeps the blit
        // correct on both 24- and 32-bit roots.
        let depth = screen.root_depth;
        let window = conn
            .generate_id()
            .map_err(|e| CoreError::ScreenRefused(format!("generate window id: {e}")))?;
        let aux = CreateWindowAux::new()
            .background_pixel(0u32)
            .override_redirect(1u32)
            .event_mask(EventMask::EXPOSURE);
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            window,
            root,
            0,
            0,
            w,
            h,
            0,
            WindowClass::INPUT_OUTPUT,
            x11rb::COPY_FROM_PARENT,
            &aux,
        )
        .map_err(|e| CoreError::ScreenRefused(format!("create_window: {e}")))?;
        conn.map_window(window)
            .map_err(|e| CoreError::ScreenRefused(format!("map_window: {e}")))?;
        let gc = conn
            .generate_id()
            .map_err(|e| CoreError::ScreenRefused(format!("generate gc id: {e}")))?;
        conn.create_gc(gc, window, &CreateGCAux::new())
            .map_err(|e| CoreError::ScreenRefused(format!("create_gc: {e}")))?;
        conn.flush()
            .map_err(|e| CoreError::ScreenRefused(format!("x11 flush: {e}")))?;
        Ok(Self {
            conn,
            window,
            gc,
            w,
            h,
            depth,
        })
    }

    fn blit(&self, frame: &ScreenFrame) -> Result<(), CoreError> {
        self.conn
            .put_image(
                ImageFormat::Z_PIXMAP,
                self.window,
                self.gc,
                frame.rect.w,
                frame.rect.h,
                0,
                0,
                0,
                self.depth,
                &frame.bytes,
            )
            .map_err(|e| CoreError::ScreenRefused(format!("put_image: {e}")))?;
        self.conn
            .flush()
            .map_err(|e| CoreError::ScreenRefused(format!("x11 flush: {e}")))
    }

    /// Whether the frame no longer fits the mapped window.
    fn mismatched(&self, frame: &ScreenFrame) -> bool {
        frame.rect.w != self.w || frame.rect.h != self.h
    }
}

impl Drop for Sink {
    fn drop(&mut self) {
        let _ = self.conn.free_gc(self.gc);
        let _ = self.conn.destroy_window(self.window);
        let _ = self.conn.flush();
    }
}

/// The single presentation window, created on first present and replaced when
/// the frame size changes.
fn sink_slot() -> &'static Mutex<Option<Sink>> {
    static SLOT: OnceLock<Mutex<Option<Sink>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(None))
}

/// Present one frame on a local X11 window.
pub fn present(frame: &ScreenFrame) -> Result<(), CoreError> {
    if frame.format != PixelFormat::Bgra8 {
        return Err(CoreError::ScreenRefused(
            "X11 PutImage path expects Bgra8 frames".into(),
        ));
    }
    if frame.rect.w == 0 || frame.rect.h == 0 {
        return Ok(());
    }
    let mut guard = sink_slot()
        .lock()
        .map_err(|_| CoreError::ScreenRefused("x11 sink mutex poisoned".into()))?;
    let reopen = match guard.as_ref() {
        Some(sink) => sink.mismatched(frame),
        None => true,
    };
    if reopen {
        *guard = Some(Sink::open(frame.rect.w, frame.rect.h)?);
    }
    guard
        .as_ref()
        .ok_or_else(|| CoreError::ScreenRefused("no x11 sink".into()))?
        .blit(frame)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fix_alpha_fills_the_high_byte_of_24bit_pixels() {
        let mut px = vec![1u8, 2, 3, 0, 5, 6, 7, 0];
        fix_alpha(&mut px, 24);
        assert_eq!(px, vec![1, 2, 3, 0xFF, 5, 6, 7, 0xFF]);
    }

    #[test]
    fn fix_alpha_leaves_32bit_pixels_untouched() {
        let mut px = vec![1u8, 2, 3, 4];
        fix_alpha(&mut px, 32);
        assert_eq!(px, vec![1, 2, 3, 4]);
    }

    #[test]
    fn to_frame_unpads_a_padded_stride() {
        // Two 1-pixel rows, each padded out to 8 bytes.
        let mut src = vec![0u8; 16];
        src[0..4].copy_from_slice(&[1, 2, 3, 0]);
        src[8..12].copy_from_slice(&[5, 6, 7, 0]);
        let f = to_frame(&src, 1, 2, 24, 8).unwrap();
        assert_eq!(f.bytes, vec![1, 2, 3, 0xFF, 5, 6, 7, 0xFF]);
        assert_eq!(f.format, PixelFormat::Bgra8);
        assert_eq!((f.rect.w, f.rect.h), (1, 2));
    }

    #[test]
    fn to_frame_copies_packed_rows_verbatim() {
        let src = vec![1u8, 2, 3, 4, 5, 6, 7, 8];
        let f = to_frame(&src, 2, 1, 32, 8).unwrap();
        assert_eq!(f.bytes, src);
    }

    #[test]
    fn to_frame_rejects_a_truncated_buffer() {
        assert!(to_frame(&[0u8; 4], 2, 2, 24, 8).is_err());
    }

    #[test]
    fn to_frame_rejects_a_stride_smaller_than_a_row() {
        assert!(to_frame(&[0u8; 16], 2, 1, 24, 4).is_err());
    }

    #[test]
    fn present_rejects_a_non_bgra_frame() {
        let rect = ScreenRect::new(0, 0, 1, 1).unwrap();
        let f = ScreenFrame::raw(1, rect, PixelFormat::Rgba8, vec![0u8; 4]).unwrap();
        let err = present(&f).expect_err("present must reject Rgba8");
        match err {
            CoreError::ScreenRefused(msg) => assert!(msg.contains("Bgra8"), "{msg}"),
            other => panic!("unexpected error: {other:?}"),
        }
    }
}
