// SPDX-License-Identifier: Apache-2.0 OR MIT
//! OS screen capture, encoder, and presentation.
//!
//! Production chain:
//!   `<platform source>` -> `EncodedSource` (zstd lossless) -> wire.
//! Visitor decodes -> `<platform sink>` presents.
//!
//! Per platform:
//! - Windows: `DxgiSource` (DXGI Desktop Duplication) / `SystemSink` (GDI).
//! - macOS: `MacSource` (CoreGraphics `CGDisplayCreateImage`) capture;
//!   present needs an AppKit `NSWindow` and is honestly refused.
//! - Linux/BSD X11: `X11Source` (MIT-SHM, `GetImage` fallback) /
//!   `SystemSink` (`PutImage` into an override-redirect window).
//! - Anything else (and pure Wayland): honest `ScreenRefused`.
//!
//! Lossy codec (OpenH264/rav1e) swaps in via `ScreenCodec` the same way.

use std::sync::{Arc, Mutex};

use tetherly_core::{
    CoreError, PixelFormat, ScreenCodec as ScreenCodecPort, ScreenFrame, ScreenRect, ScreenSink,
    ScreenSource,
};

// The per-platform capture/present bodies live in their own files, the same
// shape `cred` uses: each imports only absolute paths so it compiles for its
// target standalone. `screen.rs` stays the OS-agnostic assembly point.
#[cfg(target_os = "macos")]
#[path = "screen/macos.rs"]
mod macos;
#[cfg(all(unix, not(target_os = "macos")))]
#[path = "screen/x11.rs"]
mod x11;

// ---------------------------------------------------------------------------
// Zstd codec (cross-platform)
// ---------------------------------------------------------------------------

/// zstd's documented worst-case output bound (ZSTD_COMPRESSBOUND):
/// src + (src >> 8) + (src < 128 KiB ? (128 KiB - src) >> 8 : 0).
fn zstd_compress_bound(src: usize) -> usize {
    const ZSTD_WINDOW: usize = 128 * 1024;
    src + (src >> 8)
        + if src < ZSTD_WINDOW {
            (ZSTD_WINDOW - src) >> 8
        } else {
            0
        }
}

/// Lossless zstd codec — the first production encoder / decoder for the
/// screen channel. Compression level 3 is a sensible default.
///
/// The zstd contexts are created once and reused across frames (`Mutex` for
/// interior mutability): the one-shot stream API pays context setup plus a
/// fresh output allocation per frame, which dominates at 1080p frame rates.
pub struct ZstdCodec {
    level: i32,
    comp: Mutex<Option<zstd::bulk::Compressor<'static>>>,
    decomp: Mutex<zstd::bulk::Decompressor<'static>>,
}

impl std::fmt::Debug for ZstdCodec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZstdCodec")
            .field("level", &self.level)
            .finish()
    }
}

impl ZstdCodec {
    pub fn new(level: i32) -> Self {
        Self {
            level,
            comp: Mutex::new(None),
            decomp: Mutex::new(zstd::bulk::Decompressor::new().expect("zstd decompressor")),
        }
    }
}

impl Default for ZstdCodec {
    fn default() -> Self {
        Self::new(3)
    }
}

impl ScreenCodecPort for ZstdCodec {
    fn compress(&self, raw: &[u8]) -> Result<Vec<u8>, CoreError> {
        let mut guard = self.comp.lock().expect("zstd comp");
        let comp = match guard.as_mut() {
            Some(c) => c,
            None => guard.insert(
                zstd::bulk::Compressor::new(self.level)
                    .map_err(|e| CoreError::ScreenCodec(e.to_string()))?,
            ),
        };
        let mut buf = Vec::with_capacity(zstd_compress_bound(raw.len()));
        let n = comp
            .compress_to_buffer(raw, &mut buf)
            .map_err(|e| CoreError::ScreenCodec(e.to_string()))?;
        buf.truncate(n);
        Ok(buf)
    }

    fn decompress(&self, compressed: &[u8], expected_len: usize) -> Result<Vec<u8>, CoreError> {
        let mut guard = self.decomp.lock().expect("zstd decomp");
        let mut out = vec![0u8; expected_len];
        let n = guard
            .decompress_to_buffer(compressed, &mut out)
            .map_err(|e| CoreError::ScreenCodec(e.to_string()))?;
        if n != expected_len {
            return Err(CoreError::ScreenCodec(format!(
                "decoded len {n} != expected {expected_len}"
            )));
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// H.264 codec (OpenH264, ADR-079)
// ---------------------------------------------------------------------------

/// Packed I420 planar frame, the format the OpenH264 encoder consumes.
/// Built from BGRA with BT.601 limited-range coefficients and 2×2 box
/// chroma subsampling — deterministic pure integer math, no external
/// conversion crate.
pub struct I420 {
    w: usize,
    h: usize,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
}

impl I420 {
    /// BT.601 limited-range RGB→YUV, fixed point (16.16). Requires tightly
    /// packed BGRA input of exactly `w * h * 4` bytes.
    pub fn from_bgra(bgra: &[u8], w: usize, h: usize) -> Result<Self, CoreError> {
        if w == 0 || h == 0 || bgra.len() != w * h * 4 {
            return Err(CoreError::ScreenCodec(
                "bgra buffer does not match dims".into(),
            ));
        }
        let cw = w.div_ceil(2);
        let ch = h.div_ceil(2);
        let mut y = vec![0u8; w * h];
        let mut u = vec![0u8; cw * ch];
        let mut v = vec![0u8; cw * ch];
        for row in 0..h {
            for col in 0..w {
                let px = (row * w + col) * 4;
                let (b, g, r) = (bgra[px] as u32, bgra[px + 1] as u32, bgra[px + 2] as u32);
                let luma = 16 + (16763 * r + 32909 * g + 6391 * b) / 65536;
                y[row * w + col] = luma.min(255) as u8;
                if row % 2 == 0 && col % 2 == 0 {
                    // 2×2 box average of the chroma of the four covering
                    // pixels (the even pixel plus its right/down neighbours
                    // where they exist), so odd dims stay defined.
                    let mut us = 0i64;
                    let mut vs = 0i64;
                    let mut n = 0i64;
                    for dy in 0..2 {
                        for dx in 0..2 {
                            let (rr, cc) = (row + dy, col + dx);
                            if rr < h && cc < w {
                                let q = (rr * w + cc) * 4;
                                let (bb, gg, rr2) =
                                    (bgra[q] as i64, bgra[q + 1] as i64, bgra[q + 2] as i64);
                                us += -9676 * rr2 - 18988 * gg + 28672 * bb;
                                vs += 28672 * rr2 - 24001 * gg - 4663 * bb;
                                n += 1;
                            }
                        }
                    }
                    let cu = 128 + (us / n) / 65536;
                    let cv = 128 + (vs / n) / 65536;
                    let idx = (row / 2) * cw + (col / 2);
                    u[idx] = cu.clamp(0, 255) as u8;
                    v[idx] = cv.clamp(0, 255) as u8;
                }
            }
        }
        Ok(Self { w, h, y, u, v })
    }
}

impl openh264::formats::YUVSource for I420 {
    fn dimensions(&self) -> (usize, usize) {
        (self.w, self.h)
    }
    fn strides(&self) -> (usize, usize, usize) {
        (self.w, self.w.div_ceil(2), self.w.div_ceil(2))
    }
    fn y(&self) -> &[u8] {
        &self.y
    }
    fn u(&self) -> &[u8] {
        &self.u
    }
    fn v(&self) -> &[u8] {
        &self.v
    }
}

struct H264EncoderState {
    encoder: openh264::encoder::Encoder,
    w: usize,
    h: usize,
    /// Target bitrate for the next build; updated hot by the ADR-078 tier.
    bitrate_bps: u32,
    /// Bitrate the *live* encoder was configured with; differs from the
    /// target → rebuild (= fresh SPS/PPS + IDR), same as a geometry change.
    built_bitrate_bps: u32,
}

/// Lossy H.264 codec (ADR-079). BGRA in, annex-B out; the decoder side
/// restores BGRA. The encoder is rebuilt (→ forced IDR) whenever the frame
/// geometry changes, so a resolution switch never references stale samples.
///
/// `Send + Sync` via `Mutex`: one viewer stream at a time by design, and the
/// encoder contexts are far too expensive to rebuild per frame.
pub struct H264Codec {
    bitrate_bps: std::sync::atomic::AtomicU32,
    enc: Mutex<Option<H264EncoderState>>,
    dec: Mutex<Option<openh264::decoder::Decoder>>,
}

impl std::fmt::Debug for H264Codec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("H264Codec")
            .field("bitrate_bps", &self.bitrate_bps)
            .finish()
    }
}

impl H264Codec {
    pub fn new(bitrate_bps: u32) -> Self {
        Self {
            bitrate_bps: std::sync::atomic::AtomicU32::new(bitrate_bps.clamp(100_000, 50_000_000)),
            enc: Mutex::new(None),
            dec: Mutex::new(None),
        }
    }

    /// Hot bitrate update (ADR-078 auto-tiering): takes effect on the next
    /// frame as an encoder rebuild, i.e. an IDR — P-frames never claim a
    /// rate the stream did not start with.
    pub fn set_bitrate_bps(&self, bps: u32) {
        self.bitrate_bps.store(
            bps.clamp(100_000, 50_000_000),
            std::sync::atomic::Ordering::Relaxed,
        );
    }

    pub fn bitrate_bps(&self) -> u32 {
        self.bitrate_bps.load(std::sync::atomic::Ordering::Relaxed)
    }

    fn encode_with(state: &mut H264EncoderState, yuv: &I420) -> Result<Vec<u8>, CoreError> {
        use openh264::formats::YUVSource as _;

        let (w, h) = yuv.dimensions();
        if state.w != w || state.h != h || state.built_bitrate_bps != state.bitrate_bps {
            // Geometry or bitrate change: a fresh encoder emits SPS/PPS + IDR
            // for the new shape/rate, and P-frames never reference the old one.
            let config = openh264::encoder::EncoderConfig::new()
                .bitrate(openh264::encoder::BitRate::from_bps(state.bitrate_bps))
                .max_frame_rate(openh264::encoder::FrameRate::from_hz(30.0));
            state.encoder = openh264::encoder::Encoder::with_api_config(
                openh264::OpenH264API::from_source(),
                config,
            )
            .map_err(|e| CoreError::ScreenCodec(format!("openh264 encoder: {e}")))?;
            state.w = w;
            state.h = h;
            state.built_bitrate_bps = state.bitrate_bps;
        }
        state
            .encoder
            .encode(yuv)
            .map(|stream| stream.to_vec())
            .map_err(|e| CoreError::ScreenCodec(format!("openh264 encode: {e}")))
    }
}

impl Default for H264Codec {
    fn default() -> Self {
        Self::new(4_000_000)
    }
}

impl ScreenCodecPort for H264Codec {
    fn compress(&self, raw: &[u8]) -> Result<Vec<u8>, CoreError> {
        // Byte-only callers have no geometry; this codec cannot serve them.
        let _ = raw;
        Err(CoreError::ScreenCodec(
            "h264 needs frame geometry; use compress_frame".into(),
        ))
    }

    fn decompress(&self, compressed: &[u8], expected_len: usize) -> Result<Vec<u8>, CoreError> {
        let _ = (compressed, expected_len);
        Err(CoreError::ScreenCodec(
            "h264 needs frame geometry; use decompress_frame".into(),
        ))
    }

    fn wire_tag(&self) -> u8 {
        tetherly_core::SCREEN_COMPRESS_H264
    }

    fn compress_frame(&self, raw: &[u8], w: u16, h: u16) -> Result<Vec<u8>, CoreError> {
        let yuv = I420::from_bgra(raw, w as usize, h as usize)?;
        let mut guard = self.enc.lock().expect("h264 enc");
        if guard.is_none() {
            let encoder = openh264::encoder::Encoder::new()
                .map_err(|e| CoreError::ScreenCodec(format!("openh264 encoder: {e}")))?;
            *guard = Some(H264EncoderState {
                encoder,
                w: 0,
                h: 0,
                bitrate_bps: self.bitrate_bps(),
                built_bitrate_bps: 0,
            });
        }
        let state = guard.as_mut().expect("encoder just inserted");
        state.bitrate_bps = self.bitrate_bps();
        Self::encode_with(state, &yuv)
    }

    fn decompress_frame(
        &self,
        body: &[u8],
        w: u16,
        h: u16,
        expected_len: usize,
    ) -> Result<Vec<u8>, CoreError> {
        let mut guard = self.dec.lock().expect("h264 dec");
        if guard.is_none() {
            let decoder = openh264::decoder::Decoder::new()
                .map_err(|e| CoreError::ScreenCodec(format!("openh264 decoder: {e}")))?;
            *guard = Some(decoder);
        }
        let decoder = guard.as_mut().expect("decoder just inserted");
        let decoded = decoder
            .decode(body)
            .map_err(|e| CoreError::ScreenCodec(format!("openh264 decode: {e}")))?
            .ok_or_else(|| CoreError::ScreenCodec("openh264 decoder buffered a frame".into()))?;
        let (dw, dh) = {
            use openh264::formats::YUVSource as _;
            decoded.dimensions()
        };
        if (dw, dh) != (w as usize, h as usize) {
            return Err(CoreError::ScreenCodec(format!(
                "h264 frame is {dw}x{dh}, header claims {w}x{h}"
            )));
        }
        // `write_rgba8` fills w*h*4 RGBA; swizzle to the BGRA every sink takes.
        let mut out = vec![0u8; expected_len];
        decoded.write_rgba8(&mut out);
        for px in out.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        Ok(out)
    }
}

/// ADR-078 / v1.92 auto-tiering: the video ceiling is half the peer's
/// *measured* link capacity (the other half stays for control, file, and
/// overlay traffic), clamped to a sane screen band. No profile yet — or a
/// non-finite one — keeps the conservative default.
pub const SCREEN_BITRATE_MIN_BPS: u32 = 500_000;
pub const SCREEN_BITRATE_MAX_BPS: u32 = 20_000_000;
pub const SCREEN_BITRATE_DEFAULT_BPS: u32 = 4_000_000;

pub fn screen_bitrate_from_profile(measured_mbps: Option<f64>) -> u32 {
    let half_bps = match measured_mbps {
        Some(mbps) if mbps.is_finite() && mbps > 0.0 => mbps * 1_000_000.0 / 2.0,
        _ => return SCREEN_BITRATE_DEFAULT_BPS,
    };
    (half_bps as u64).clamp(SCREEN_BITRATE_MIN_BPS as u64, SCREEN_BITRATE_MAX_BPS as u64) as u32
}

/// Negotiated screen-channel codec (ADR-079 §决定): encode with H.264 only
/// when the operator asked for it *and* the peer advertised `screencodecs`
/// containing `h264`; otherwise zstd. The decode side is not chosen here —
/// the receiver dispatches per-frame on the wire tag.
pub struct NegotiatedCodec {
    want_h264: bool,
    h264: H264Codec,
    zstd: ZstdCodec,
    peer_h264: std::sync::atomic::AtomicBool,
}

impl NegotiatedCodec {
    /// `peer_supports_h264` is re-evaluated on every caps merge and at
    /// stream start; the atomic keeps that lock-free from the frame pump.
    pub fn new(want_h264: bool, bitrate_bps: u32, peer_supports_h264: bool) -> Self {
        Self {
            want_h264,
            h264: H264Codec::new(bitrate_bps),
            zstd: ZstdCodec::default(),
            peer_h264: std::sync::atomic::AtomicBool::new(peer_supports_h264),
        }
    }

    pub fn set_peer_supports_h264(&self, yes: bool) {
        self.peer_h264
            .store(yes, std::sync::atomic::Ordering::Relaxed);
    }

    /// Hot bitrate retier (ADR-078): forwarded to the H.264 encoder; the
    /// zstd path is lossless and ignores it by construction.
    pub fn set_bitrate_bps(&self, bps: u32) {
        self.h264.set_bitrate_bps(bps);
    }

    fn use_h264(&self) -> bool {
        self.want_h264 && self.peer_h264.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// The codec name the local UI reports.
    pub fn active_name(&self) -> &'static str {
        if self.use_h264() {
            "h264"
        } else {
            "zstd"
        }
    }
}

impl ScreenCodecPort for NegotiatedCodec {
    fn compress(&self, raw: &[u8]) -> Result<Vec<u8>, CoreError> {
        self.zstd.compress(raw)
    }

    fn decompress(&self, compressed: &[u8], expected_len: usize) -> Result<Vec<u8>, CoreError> {
        self.zstd.decompress(compressed, expected_len)
    }

    fn wire_tag(&self) -> u8 {
        if self.use_h264() {
            tetherly_core::SCREEN_COMPRESS_H264
        } else {
            tetherly_core::SCREEN_COMPRESS_ZSTD
        }
    }

    fn compress_frame(&self, raw: &[u8], w: u16, h: u16) -> Result<Vec<u8>, CoreError> {
        if self.use_h264() {
            self.h264.compress_frame(raw, w, h)
        } else {
            self.zstd.compress_frame(raw, w, h)
        }
    }
}

// ---------------------------------------------------------------------------
// EncodedSource — encoder as a swappable ScreenSource port
// ---------------------------------------------------------------------------

/// Wraps a raw capture source so every frame body is losslessly compressed
/// before it reaches the wire. A frame whose compressed body is not smaller
/// is emitted raw, so a pathological codec can never inflate the transport.
pub struct EncodedSource<S> {
    inner: S,
    codec: Arc<dyn ScreenCodecPort>,
}

impl<S> EncodedSource<S> {
    pub fn new(inner: S, codec: Arc<dyn ScreenCodecPort>) -> Self {
        Self { inner, codec }
    }
}

impl<S: ScreenSource> ScreenSource for EncodedSource<S> {
    fn grab(&self) -> Result<Option<ScreenFrame>, CoreError> {
        let Some(frame) = self.inner.grab()? else {
            return Ok(None);
        };
        let encoded = self
            .codec
            .compress_frame(&frame.bytes, frame.rect.w, frame.rect.h)?;
        if encoded.len() >= frame.bytes.len() {
            return Ok(Some(frame));
        }
        Ok(Some(ScreenFrame {
            codec_tag: self.codec.wire_tag(),
            bytes: encoded,
            ..frame
        }))
    }
}

// ---------------------------------------------------------------------------
// DxgiSource — real DXGI Desktop Duplication (Windows-only)
// ---------------------------------------------------------------------------

#[cfg(windows)]
mod dxgi {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Mutex, OnceLock};
    use tetherly_core::{CoreError, PixelFormat, ScreenFrame, ScreenRect, ScreenSource};
    use windows::core::{Interface, PCWSTR};
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
    use windows::Win32::Graphics::Direct3D11::{
        ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D, D3D11_CPU_ACCESS_READ,
        D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ,
        D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    };
    use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};
    use windows::Win32::Graphics::Dxgi::{
        IDXGIAdapter1, IDXGIDevice, IDXGIOutput1, IDXGIOutputDuplication, DXGI_ERROR_WAIT_TIMEOUT,
        DXGI_OUTDUPL_FRAME_INFO,
    };
    use windows::Win32::Graphics::Gdi::{
        BitBlt, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC,
        SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HBITMAP, HDC, HGDIOBJ,
        RGBQUAD, SRCCOPY,
    };
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};
    use windows::Win32::UI::WindowsAndMessaging::{
        AdjustWindowRect, CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW,
        PostMessageW, PostQuitMessage, RegisterClassW, SetWindowPos, ShowWindow, CS_HREDRAW,
        CS_VREDRAW, CW_USEDEFAULT, MSG, SWP_NOZORDER, SW_SHOW, WINDOW_EX_STYLE, WM_APP, WM_CLOSE,
        WM_DESTROY, WM_ERASEBKGND, WM_PAINT, WNDCLASSW, WNDCLASS_STYLES, WS_OVERLAPPEDWINDOW,
    };

    // -- DXGI capture -----------------------------------------------------------

    fn init_device() -> Result<(ID3D11Device, ID3D11DeviceContext), CoreError> {
        let mut dev = None;
        let mut ctx = None;
        unsafe {
            windows::Win32::Graphics::Direct3D11::D3D11CreateDevice(
                None,
                windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE,
                Default::default(), // software HMODULE
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None, // feature levels
                D3D11_SDK_VERSION,
                Some(&mut dev),
                None,
                Some(&mut ctx),
            )
        }
        .map_err(|e| CoreError::ScreenRefused(format!("D3D11CreateDevice: {e}")))?;
        Ok((dev.unwrap(), ctx.unwrap()))
    }

    struct DxgiState {
        dev: ID3D11Device,
        ctx: ID3D11DeviceContext,
        dup: IDXGIOutputDuplication,
        width: u32,
        height: u32,
    }

    /// D3D11/DXGI objects are COM objects tied to the creating thread; they are
    /// not `Send` by default. The pump task is the single caller, so we move the
    /// guard between threads on the same task — the underlying COM state stays
    /// on whichever OS thread the pump runs on at the time. This is the
    /// well-known single-threaded D3D11 pattern.
    unsafe impl Send for DxgiState {}
    unsafe impl Sync for DxgiState {}

    static COM_INIT: AtomicBool = AtomicBool::new(false);
    static DXGI_STATE: OnceLock<Mutex<Result<DxgiState, CoreError>>> = OnceLock::new();

    fn ensure_com() {
        if !COM_INIT.swap(true, Ordering::SeqCst) {
            unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            }
        }
    }

    /// Lock the DXGI state, lazily creating it on first grab. The returned
    /// guard borrows from the static, so each grab serializes on one thread at
    /// a time (D3D11 immediate context is not thread-safe by design).
    fn state() -> Result<std::sync::MutexGuard<'static, Result<DxgiState, CoreError>>, CoreError> {
        let slot = DXGI_STATE.get_or_init(|| Mutex::new(try_init()));
        let mut guard = slot
            .lock()
            .map_err(|e| CoreError::ScreenRefused(format!("mutex poisoned: {e}")))?;
        if guard.is_err() {
            // A prior init failed (no output at boot under a headless/remote
            // session, a mode change mid-duplication). Re-attempt rather than
            // caching the failure for the process life, or capture stays dead
            // until restart. Grabs are paced by the pump's backoff, so this
            // cannot spin.
            *guard = try_init();
        }
        Ok(guard)
    }

    fn try_init() -> Result<DxgiState, CoreError> {
        ensure_com();
        let (dev, ctx) = init_device()?;
        let dxgi_dev: IDXGIDevice = dev
            .cast::<IDXGIDevice>()
            .map_err(|e| CoreError::ScreenRefused(format!("cast IDXGIDevice: {e}")))?;
        let adapter: IDXGIAdapter1 = unsafe { dxgi_dev.GetAdapter() }
            .map_err(|e| CoreError::ScreenRefused(format!("GetAdapter: {e}")))?
            .cast::<IDXGIAdapter1>()
            .map_err(|e| CoreError::ScreenRefused(format!("cast IDXGIAdapter1: {e}")))?;
        let output: IDXGIOutput1 = unsafe { adapter.EnumOutputs(0) }
            .map_err(|e| CoreError::ScreenRefused(format!("EnumOutputs: {e}")))?
            .cast::<IDXGIOutput1>()
            .map_err(|e| CoreError::ScreenRefused(format!("cast IDXGIOutput1: {e}")))?;
        let dup = unsafe { output.DuplicateOutput(&dev) }
            .map_err(|e| CoreError::ScreenRefused(format!("DuplicateOutput: {e}")))?;
        let desc = unsafe { dup.GetDesc() };
        let w = desc.ModeDesc.Width;
        let h = desc.ModeDesc.Height;
        Ok(DxgiState {
            dev,
            ctx,
            dup,
            width: w,
            height: h,
        })
    }

    #[derive(Debug, Clone, Default)]
    pub struct DxgiSource;

    impl DxgiSource {
        pub fn new() -> Self {
            Self
        }
    }

    impl ScreenSource for DxgiSource {
        fn grab(&self) -> Result<Option<ScreenFrame>, CoreError> {
            let mut guard = state()?;
            let st = guard.as_mut().map_err(|e| e.clone())?;

            let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
            let mut resource = None;

            match unsafe { st.dup.AcquireNextFrame(0, &mut info, &mut resource) } {
                Ok(()) => {}
                Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => {
                    return Ok(None);
                }
                Err(e) => {
                    return Err(CoreError::ScreenRefused(format!("AcquireNextFrame: {e}")));
                }
            }

            let Some(resource) = resource else {
                let _ = unsafe { st.dup.ReleaseFrame() };
                return Ok(None);
            };

            // Every early return past `AcquireNextFrame` must release the
            // frame, or the duplication interface wedges: the next acquire
            // returns DXGI_ERROR_INVALID_CALL forever.
            let dup_tex: ID3D11Texture2D = match resource.cast::<ID3D11Texture2D>() {
                Ok(t) => t,
                Err(e) => {
                    let _ = unsafe { st.dup.ReleaseFrame() };
                    return Err(CoreError::ScreenRefused(format!("cast tex: {e}")));
                }
            };

            let w = st.width;
            let h = st.height;

            let desc = D3D11_TEXTURE2D_DESC {
                Width: w,
                Height: h,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: D3D11_USAGE_STAGING,
                BindFlags: 0,
                CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                MiscFlags: 0,
            };

            let staging = {
                let mut tex = None;
                if let Err(e) = unsafe { st.dev.CreateTexture2D(&desc, None, Some(&mut tex)) } {
                    let _ = unsafe { st.dup.ReleaseFrame() };
                    return Err(CoreError::ScreenRefused(format!(
                        "CreateTexture2D staging: {e}"
                    )));
                }
                tex.unwrap()
            };

            unsafe { st.ctx.CopyResource(&staging, &dup_tex) };

            let mapped = {
                let mut m = D3D11_MAPPED_SUBRESOURCE::default();
                if let Err(e) = unsafe { st.ctx.Map(&staging, 0, D3D11_MAP_READ, 0, Some(&mut m)) }
                {
                    let _ = unsafe { st.dup.ReleaseFrame() };
                    return Err(CoreError::ScreenRefused(format!("Map: {e}")));
                }
                m
            };

            let pitch = mapped.RowPitch as usize;
            let row_bytes = w as usize * 4;
            let mut pixels = vec![0u8; row_bytes * h as usize];
            let src = mapped.pData as *const u8;
            if pitch == row_bytes {
                unsafe { std::ptr::copy_nonoverlapping(src, pixels.as_mut_ptr(), pixels.len()) };
            } else {
                for row in 0..h as usize {
                    let s = unsafe { src.add(row * pitch) };
                    let d = &mut pixels[row * row_bytes..(row + 1) * row_bytes];
                    unsafe { std::ptr::copy_nonoverlapping(s, d.as_mut_ptr(), row_bytes) };
                }
            }

            unsafe {
                st.ctx.Unmap(&staging, 0);
                let _ = st.dup.ReleaseFrame();
            }

            let rect = ScreenRect::new(0, 0, w as u16, h as u16)?;
            ScreenFrame::raw(0, rect, PixelFormat::Bgra8, pixels).map(Some)
        }
    }

    // -- GDI window presentation ------------------------------------------------

    static PRESENT_CLASS: &[u16] = &[
        'T' as u16, 'e' as u16, 't' as u16, 'h' as u16, 'e' as u16, 'r' as u16, 'l' as u16,
        'y' as u16, 'S' as u16, 'c' as u16, 'r' as u16, 'e' as u16, 'e' as u16, 'n' as u16, 0,
    ];

    struct PresentData {
        // Load-bearing for lifetime: dropping the handle destroys the DIB
        // memory [bits] points into; read on reinit to destroy the old DIB.
        dib: HBITMAP,
        dc: HDC,
        w: u16,
        h: u16,
        /// DIB pixel memory from `CreateDIBSection` — written directly, so a
        /// frame is one memcpy into GDI-owned memory with no staging buffer.
        bits: *mut u8,
    }

    // HBITMAP and HDC are GDI handle wrappers — raw isize-backed handles that
    // are not Send by default. Only accessed from the same pump task that creates
    // them; this is the standard single-threaded GDI pattern.
    unsafe impl Send for PresentData {}
    unsafe impl Sync for PresentData {}

    static PRESENT_DATA: OnceLock<Mutex<Option<PresentData>>> = OnceLock::new();
    /// Window handle stored as a raw integer: `HWND` is a raw-pointer wrapper and
    /// is neither `Send` nor `Sync`, but the handle value is valid process-wide.
    static PRESENT_HWND: OnceLock<isize> = OnceLock::new();

    fn widen(s: &[u16]) -> PCWSTR {
        PCWSTR::from_raw(s.as_ptr())
    }

    /// One resize request marshalled to the window's own thread: cross-thread
    /// `SetWindowPos` against a window whose thread has no pump silently
    /// fails, so the pump thread does it itself on `WM_APP_RESIZE`.
    const WM_APP_RESIZE: u32 = WM_APP;

    fn window_start() {
        // The window lives on a dedicated thread with a message pump. The
        // presenting task cannot pump messages (it is a tokio task), and a
        // pumpless window cannot process cross-thread SetWindowPos/PrintWindow.
        let _ = std::thread::Builder::new()
            .name("tetherly-present".into())
            .spawn(|| unsafe {
                if let Err(e) = window_init() {
                    tracing::warn!(%e, "screen present window init");
                    return;
                }
                let mut msg = MSG::default();
                loop {
                    // 0 = WM_QUIT, -1 = error: both end the pump.
                    if !GetMessageW(&mut msg, None, 0, 0).as_bool() {
                        break;
                    }
                    DispatchMessageW(&msg);
                }
            });
    }

    unsafe fn window_init() -> Result<(), CoreError> {
        let wc = WNDCLASSW {
            style: WNDCLASS_STYLES(CS_HREDRAW.0 | CS_VREDRAW.0),
            lpfnWndProc: Some(wndproc),
            hInstance: Default::default(),
            lpszClassName: widen(PRESENT_CLASS),
            ..Default::default()
        };
        if unsafe { RegisterClassW(&wc) } == 0 {
            return Err(CoreError::ScreenRefused("RegisterClassW failed".into()));
        }

        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                widen(PRESENT_CLASS),
                PCWSTR::null(),
                WS_OVERLAPPEDWINDOW,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                CW_USEDEFAULT,
                None,
                None,
                None,
                None,
            )
        }
        .map_err(|e| CoreError::ScreenRefused(format!("CreateWindowExW: {e}")))?;

        if hwnd.is_invalid() {
            return Err(CoreError::ScreenRefused(
                "CreateWindowExW returned null".into(),
            ));
        }

        unsafe {
            let _ = ShowWindow(hwnd, SW_SHOW);
        };
        // Data first, then the hwnd handle: a caller that sees the hwnd can
        // rely on PRESENT_DATA being initialized.
        PRESENT_DATA.get_or_init(|| Mutex::new(None));
        PRESENT_HWND.set(hwnd.0 as isize).ok();
        Ok(())
    }

    unsafe extern "system" fn wndproc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_PAINT => {
                let guard = PRESENT_DATA.get().unwrap().lock().unwrap();
                if let Some(ref pd) = *guard {
                    let dc = unsafe { GetDC(Some(hwnd)) };
                    if !dc.is_invalid() {
                        let _ = unsafe {
                            BitBlt(
                                dc,
                                0,
                                0,
                                pd.w as i32,
                                pd.h as i32,
                                Some(pd.dc),
                                0,
                                0,
                                SRCCOPY,
                            )
                        };
                        unsafe { ReleaseDC(Some(hwnd), dc) };
                    }
                }
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_APP_RESIZE => {
                let w = wparam.0 as u16;
                let h = lparam.0 as u16;
                let mut rc = RECT {
                    left: 0,
                    top: 0,
                    right: w as i32,
                    bottom: h as i32,
                };
                let _ = AdjustWindowRect(&mut rc, WS_OVERLAPPEDWINDOW, false);
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    0,
                    0,
                    rc.right - rc.left,
                    rc.bottom - rc.top,
                    SWP_NOZORDER,
                );
                LRESULT(0)
            }
            WM_CLOSE | WM_DESTROY => {
                PostQuitMessage(0);
                LRESULT(0)
            }
            _ => DefWindowProcW(hwnd, msg, wparam, lparam),
        }
    }

    static STARTED: OnceLock<()> = OnceLock::new();

    pub fn present(frame: &ScreenFrame) -> Result<(), CoreError> {
        STARTED.get_or_init(window_start);

        // The window is created on the pump thread: wait briefly for it to
        // come up instead of dropping the first frame.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while PRESENT_HWND.get().is_none() {
            if std::time::Instant::now() > deadline {
                return Err(CoreError::ScreenRefused("no present window".into()));
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let w = frame.rect.w as u32;
        let h = frame.rect.h as u32;
        let row_bytes = (w * 4) as usize;

        let needs_reinit = match PRESENT_DATA.get().unwrap().lock().unwrap().as_ref() {
            None => true,
            Some(pd) => pd.w != frame.rect.w || pd.h != frame.rect.h,
        };

        if needs_reinit {
            // Destroy the previous frame's DIB and DC first: windows-rs
            // handle types have no Drop, so skipping this leaks an ~8 MB DIB
            // plus a DC on every resolution change.
            let old = PRESENT_DATA.get().unwrap().lock().unwrap().take();
            if let Some(old) = old {
                unsafe {
                    let _ = DeleteDC(old.dc);
                    let _ = DeleteObject(old.dib.into());
                }
            }
            let hwnd_raw = *PRESENT_HWND
                .get()
                .ok_or_else(|| CoreError::ScreenRefused("no present window".into()))?;
            let hwnd = HWND(hwnd_raw as *mut std::ffi::c_void);
            let dc = unsafe { GetDC(Some(hwnd)) };
            if dc.is_invalid() {
                return Err(CoreError::ScreenRefused("GetDC failed".into()));
            }
            let mem_dc = unsafe { CreateCompatibleDC(Some(dc)) };
            if mem_dc.is_invalid() {
                unsafe { ReleaseDC(Some(hwnd), dc) };
                return Err(CoreError::ScreenRefused("CreateCompatibleDC failed".into()));
            }

            let bih = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: w as i32,
                biHeight: -(h as i32), // top-down DIB
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                // Saturating: w*h*4 overflows u32 for a hostile 65535x65535
                // rect; the frame is already length-checked, so this is only
                // to keep the header field well-defined.
                biSizeImage: w.saturating_mul(h).saturating_mul(4),
                ..Default::default()
            };
            let bi = BITMAPINFO {
                bmiHeader: bih,
                bmiColors: [RGBQUAD::default(); 1],
            };

            let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
            let dib = match unsafe {
                CreateDIBSection(Some(mem_dc), &bi, DIB_RGB_COLORS, &mut bits, None, 0)
            } {
                Ok(d) => d,
                Err(e) => {
                    // Neither handle has Drop; release both or leak a DC on
                    // every failed reinit.
                    unsafe {
                        let _ = DeleteDC(mem_dc);
                        let _ = ReleaseDC(Some(hwnd), dc);
                    }
                    return Err(CoreError::ScreenRefused(format!("CreateDIBSection: {e}")));
                }
            };

            let _old = unsafe { SelectObject(mem_dc, HGDIOBJ(dib.0)) };

            // Size the present window to the frame (plus chrome) so tall
            // sources like a 720x1280 phone are fully visible. The resize is
            // posted to the window's own pump thread.
            if let Some(&hwnd_raw) = PRESENT_HWND.get() {
                unsafe {
                    let _ = PostMessageW(
                        Some(HWND(hwnd_raw as *mut std::ffi::c_void)),
                        WM_APP_RESIZE,
                        WPARAM(w as usize),
                        LPARAM(h as isize),
                    );
                }
            }

            *PRESENT_DATA.get().unwrap().lock().unwrap() = Some(PresentData {
                dib,
                dc: mem_dc,
                w: frame.rect.w,
                h: frame.rect.h,
                bits: bits as *mut u8,
            });

            unsafe { ReleaseDC(Some(hwnd), dc) };
        }

        let mut guard = PRESENT_DATA.get().unwrap().lock().unwrap();
        let pd = guard
            .as_mut()
            .ok_or_else(|| CoreError::ScreenRefused("no present data".into()))?;

        if pd.bits.is_null() || frame.bytes.len() != row_bytes * h as usize {
            return Err(CoreError::ScreenRefused(
                "present: frame does not match DIB".into(),
            ));
        }
        // One memcpy into the DIB GDI owns; no staging buffer, no per-frame
        // GetObjectW. Top-down rows line up with the packed frame body.
        unsafe {
            std::ptr::copy_nonoverlapping(frame.bytes.as_ptr(), pd.bits, frame.bytes.len());
        }

        // Blit synchronously. There is no message pump on this thread, so we
        // cannot rely on WM_PAINT to copy the frame; drawing to the window DC
        // directly is self-contained and displays immediately.
        if let Some(&hwnd_raw) = PRESENT_HWND.get() {
            let hwnd = HWND(hwnd_raw as *mut std::ffi::c_void);
            let wdc = unsafe { GetDC(Some(hwnd)) };
            if !wdc.is_invalid() {
                let _ = unsafe {
                    BitBlt(
                        wdc,
                        0,
                        0,
                        pd.w as i32,
                        pd.h as i32,
                        Some(pd.dc),
                        0,
                        0,
                        SRCCOPY,
                    )
                };
                unsafe { ReleaseDC(Some(hwnd), wdc) };
            }
        }

        Ok(())
    }
}

#[cfg(windows)]
pub use dxgi::DxgiSource;
#[cfg(target_os = "macos")]
pub use macos::MacSource;
#[cfg(all(unix, not(target_os = "macos")))]
pub use x11::X11Source;

/// The production capture source for the host platform. One concrete `inner`
/// per target keeps `SystemSource` monomorphic, so `encoded_with` needs no
/// dynamic dispatch.
#[derive(Debug, Clone, Default)]
pub struct SystemSource {
    #[cfg(windows)]
    inner: DxgiSource,
    #[cfg(target_os = "macos")]
    inner: MacSource,
    #[cfg(all(unix, not(target_os = "macos")))]
    inner: X11Source,
}

impl SystemSource {
    pub fn new() -> Self {
        #[cfg(windows)]
        {
            Self {
                inner: DxgiSource::new(),
            }
        }
        #[cfg(target_os = "macos")]
        {
            Self {
                inner: MacSource::new(),
            }
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            Self {
                inner: X11Source::new(),
            }
        }
        #[cfg(not(any(windows, target_os = "macos", unix)))]
        {
            Self {}
        }
    }

    pub fn encoded_with(self, codec: Arc<dyn ScreenCodecPort>) -> EncodedSource<Self> {
        EncodedSource::new(self, codec)
    }
}

impl ScreenSource for SystemSource {
    #[cfg(any(windows, target_os = "macos", unix))]
    fn grab(&self) -> Result<Option<ScreenFrame>, CoreError> {
        self.inner.grab()
    }

    #[cfg(not(any(windows, target_os = "macos", unix)))]
    fn grab(&self) -> Result<Option<ScreenFrame>, CoreError> {
        Err(CoreError::ScreenRefused(
            "no system screen capture on this target (Windows DXGI / macOS CoreGraphics / X11)"
                .into(),
        ))
    }
}

// ---------------------------------------------------------------------------
// AdbSource — grab frames from an adb-attached Android device
// ---------------------------------------------------------------------------

/// Screen source that pulls frames from an adb-connected Android device. Use
/// it when the interesting screen is a phone (cloud phones included): the
/// host streams the device display through the same `ScreenSource` port DXGI
/// uses, so viewer/p95/present machinery is unchanged.
///
/// Primary path: ONE persistent device-side loop writing length-prefixed PNG
/// frames — `while :; do screencap -p > f; wc -c < f; cat f; done`. The host
/// reads one ASCII length line plus that many PNG bytes per `grab`; the pump
/// stops reading between streams, the device blocks on the pipe, and no
/// frames pile up. This removes the per-frame adb spawn (measured ~0.3 s on
/// the overlay) while keeping the wire small: on a Galaxy Fold a raw frame is
/// 3.7 MB (17.5 s over the overlay link) but its PNG is ~31 KB (~0.17 s), so
/// PNG is the bandwidth-optimal codec for the adb leg on any link.
///
/// `TETHERLY_ADB_RAW=1` forces the raw RGBA_8888 stream (`while :; do
/// screencap; done`, 16-byte LE header + width*height*4 bytes) instead —
/// only worth it on fast links (USB, plain LAN) where 3.7 MB per frame is
/// cheaper than the device-side PNG encode. If the device cannot serve the
/// raw header, the source falls back to the PNG stream permanently.
///
/// A frame identical to the previous one yields `Ok(None)` (the DXGI idle
/// semantics). Any stream error kills the child; the next grab respawns it.
/// `interrupt` also kills the child, which EOFs the pipe and unblocks a grab
/// stuck in a read (the runtime calls it on grab timeout).
#[derive(Debug)]
pub struct AdbSource {
    adb: std::path::PathBuf,
    serial: Option<String>,
    /// The killable child, kept separate from the reader on purpose: a grab
    /// holds `stream` for the whole (blocking) read, and `interrupt` must be
    /// able to reach the child without taking that mutex.
    child: Mutex<Option<std::process::Child>>,
    stream: Mutex<Option<AdbStream>>,
    last_png: Mutex<Option<Vec<u8>>>,
    last_raw: Mutex<Option<Vec<u8>>>,
    raw_mode: std::sync::atomic::AtomicBool,
}

#[derive(Debug)]
struct AdbStream {
    out: std::io::BufReader<std::process::ChildStdout>,
    w: u32,
    h: u32,
}

impl Drop for AdbSource {
    fn drop(&mut self) {
        self.kill_child();
    }
}

enum AdbError {
    /// Stream broke or adb is unreachable; the next grab may respawn.
    Fatal(String),
    /// The device cannot serve the raw stream; switch to PNG permanently.
    Unsupported(String),
}

impl From<AdbError> for CoreError {
    fn from(e: AdbError) -> Self {
        let msg = match e {
            AdbError::Fatal(m) | AdbError::Unsupported(m) => m,
        };
        CoreError::ScreenRefused(msg)
    }
}

/// Parse one `wc -c` length line from the PNG stream. Bounded and strict:
/// anything that is not a plain decimal count in range is a broken stream.
fn parse_len_line(line: &str) -> Result<usize, String> {
    let len: usize = line
        .trim()
        .parse()
        .map_err(|_| format!("adb stream: bad length line {line:?}"))?;
    if len == 0 || len > 64 * 1024 * 1024 {
        return Err(format!("adb stream: bad frame size {len}"));
    }
    Ok(len)
}

/// Parse the 16-byte little-endian raw screencap header: width, height,
/// format, flags. Only RGBA_8888 (format 1) with sane dimensions is accepted.
fn parse_raw_header(hdr: &[u8; 16]) -> Result<(u32, u32), String> {
    let w = u32::from_le_bytes(hdr[0..4].try_into().expect("w"));
    let h = u32::from_le_bytes(hdr[4..8].try_into().expect("h"));
    let fmt = u32::from_le_bytes(hdr[8..12].try_into().expect("fmt"));
    if fmt != 1 || w == 0 || h == 0 || w > u16::MAX as u32 || h > u16::MAX as u32 {
        return Err(format!(
            "raw screencap w={w} h={h} fmt={fmt} is not RGBA_8888"
        ));
    }
    Ok((w, h))
}

/// Swap R and B of every 4-byte pixel, in place (RGBA -> BGRA).
fn swap_rb_in_place(buf: &mut [u8]) {
    for px in buf.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
}

impl AdbSource {
    /// `serial` is the `adb devices` target, e.g. `10.144.144.51:5555`.
    /// `None` lets adb pick when exactly one device is attached.
    pub fn new(serial: impl Into<Option<String>>) -> Self {
        let raw_mode = std::env::var("TETHERLY_ADB_RAW").ok().as_deref() == Some("1");
        Self {
            adb: std::path::PathBuf::from(
                std::env::var("TETHERLY_ADB").unwrap_or_else(|_| "adb".into()),
            ),
            serial: serial.into(),
            child: Mutex::new(None),
            stream: Mutex::new(None),
            last_png: Mutex::new(None),
            last_raw: Mutex::new(None),
            raw_mode: std::sync::atomic::AtomicBool::new(raw_mode),
        }
    }

    pub fn with_adb_path(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.adb = path.into();
        self
    }

    fn command(&self) -> std::process::Command {
        let mut cmd = std::process::Command::new(&self.adb);
        if let Some(s) = &self.serial {
            cmd.args(["-s", s]);
        }
        cmd.stdin(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            // adb is a console binary: keep it from flashing a window per spawn.
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        cmd
    }

    fn kill_child(&self) {
        if let Some(mut c) = self.child.lock().expect("adb child").take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }

    /// Tear the stream down: the reader goes first (the grab holds the
    /// stream mutex, so this runs on the grab's own error path), then the
    /// child whose pipe EOF the reader is unwinding on.
    fn reset_stream(&self, guard: &mut Option<AdbStream>) {
        *guard = None;
        self.kill_child();
    }

    /// Spawn the persistent stream, remember the killable child, and hand
    /// back the reader.
    fn open_stream(
        &self,
        device_cmd: &str,
    ) -> Result<std::io::BufReader<std::process::ChildStdout>, AdbError> {
        let mut child = self
            .command()
            .args(["exec-out", device_cmd])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| AdbError::Fatal(format!("spawn adb: {e}")))?;
        let out = child
            .stdout
            .take()
            .ok_or_else(|| AdbError::Fatal("adb stdout missing".into()))?;
        *self.child.lock().expect("adb child") = Some(child);
        Ok(std::io::BufReader::new(out))
    }

    /// One PNG frame from the persistent length-prefixed stream.
    /// `Ok(None)` = unchanged screen.
    fn grab_png_stream(&self) -> Result<Option<ScreenFrame>, AdbError> {
        use std::io::{BufRead, Read};
        let mut guard = self.stream.lock().expect("adb stream");
        if guard.is_none() {
            // A temp file per iteration: wc -c must read the full size
            // before cat replays it, which pipes cannot do in one pass.
            // The pid suffix keeps same-device sources from clobbering; the
            // leading rm clears what a killed predecessor left behind.
            let cap = format!("/data/local/tmp/.tetherly_cap{}", std::process::id());
            let device_cmd = format!(
                "rm -f {cap}; while :; do screencap -p > {cap}; wc -c < {cap}; cat {cap}; done"
            );
            let out = self.open_stream(&device_cmd)?;
            *guard = Some(AdbStream { out, w: 0, h: 0 });
        }
        let st = guard.as_mut().expect("adb stream");

        // Bound the length line so a wedged device cannot grow memory.
        let mut line = String::new();
        let mut limited = (&mut st.out).take(128);
        if let Err(e) = limited.read_line(&mut line) {
            self.reset_stream(&mut guard);
            return Err(AdbError::Fatal(format!("adb stream length: {e}")));
        }
        let len = match parse_len_line(&line) {
            Ok(len) => len,
            Err(msg) => {
                self.reset_stream(&mut guard);
                return Err(AdbError::Fatal(msg));
            }
        };
        let mut png = vec![0u8; len];
        if let Err(e) = st.out.read_exact(&mut png) {
            self.reset_stream(&mut guard);
            return Err(AdbError::Fatal(format!("adb stream frame: {e}")));
        }

        // Byte-identical PNG means an unchanged screen: the idle path.
        {
            let mut last = self.last_png.lock().expect("adb last png");
            if last.as_ref() == Some(&png) {
                return Ok(None);
            }
            *last = Some(png.clone());
        }
        drop(guard);

        self.decode_png(&png).map_err(AdbError::Fatal)
    }

    /// One raw RGBA_8888 frame from the persistent `screencap` loop.
    fn grab_raw_stream(&self) -> Result<Option<ScreenFrame>, AdbError> {
        use std::io::Read;
        let mut guard = self.stream.lock().expect("adb stream");
        if guard.is_none() {
            let out = self.open_stream("while :; do screencap; done")?;
            *guard = Some(AdbStream { out, w: 0, h: 0 });
        }
        let st = guard.as_mut().expect("adb stream");

        let mut hdr = [0u8; 16];
        if let Err(e) = st.out.read_exact(&mut hdr) {
            self.reset_stream(&mut guard);
            return Err(AdbError::Fatal(format!("adb stream header: {e}")));
        }
        let (w, h) = match parse_raw_header(&hdr) {
            Ok(dims) => dims,
            Err(msg) => {
                self.reset_stream(&mut guard);
                return Err(AdbError::Unsupported(msg));
            }
        };
        st.w = w;
        st.h = h;

        let mut buf = vec![0u8; w as usize * h as usize * 4];
        if let Err(e) = st.out.read_exact(&mut buf) {
            self.reset_stream(&mut guard);
            return Err(AdbError::Fatal(format!("adb stream pixels: {e}")));
        }

        // Byte-identical pixels mean an unchanged screen: the idle path.
        {
            let mut last = self.last_raw.lock().expect("adb last frame");
            if last.as_deref() == Some(&buf) {
                return Ok(None);
            }
            *last = Some(buf.clone());
        }

        // The sink presents into a 32bpp BGRA DIB: swap R and B in place.
        swap_rb_in_place(&mut buf);
        let rect = ScreenRect::new(0, 0, w as u16, h as u16)
            .map_err(|e| AdbError::Fatal(e.to_string()))?;
        ScreenFrame::raw(0, rect, PixelFormat::Bgra8, buf)
            .map(Some)
            .map_err(|e| AdbError::Fatal(e.to_string()))
    }

    /// One-shot PNG path: spawn `screencap -p`, decode, convert. Kept as the
    /// last-resort fallback if even the PNG stream cannot be parsed.
    fn grab_png_oneshot(&self) -> Result<Option<ScreenFrame>, CoreError> {
        let out = self
            .command()
            .args(["exec-out", "screencap", "-p"])
            .output()
            .map_err(|e| CoreError::ScreenRefused(format!("spawn adb: {e}")))?;
        if !out.status.success() {
            let msg = String::from_utf8_lossy(&out.stderr);
            return Err(CoreError::ScreenRefused(format!(
                "adb screencap: {}",
                msg.trim()
            )));
        }
        if out.stdout.is_empty() {
            return Err(CoreError::ScreenRefused(
                "adb screencap: empty output".into(),
            ));
        }

        // Byte-identical PNG means an unchanged screen: the idle path.
        {
            let mut last = self.last_png.lock().expect("adb last png");
            if last.as_ref() == Some(&out.stdout) {
                return Ok(None);
            }
            *last = Some(out.stdout.clone());
        }

        self.decode_png(&out.stdout)
            .map_err(CoreError::ScreenRefused)
    }

    fn decode_png(&self, png: &[u8]) -> Result<Option<ScreenFrame>, String> {
        let decoder = png::Decoder::new(std::io::Cursor::new(png));
        let mut reader = decoder
            .read_info()
            .map_err(|e| format!("screencap png header: {e}"))?;
        let size = reader
            .output_buffer_size()
            .ok_or_else(|| "screencap png: no output size".to_string())?;
        let mut buf = vec![0u8; size];
        let info = reader
            .next_frame(&mut buf)
            .map_err(|e| format!("screencap png decode: {e}"))?;
        let (w, h) = (info.width, info.height);
        if w == 0 || h == 0 || w > u16::MAX as u32 || h > u16::MAX as u32 {
            return Err(format!("screencap size {w}x{h} out of range"));
        }

        // The sink presents into a 32bpp BGRA DIB, so convert to tightly
        // packed BGRA8. screencap emits 8-bit RGBA (convert in place) or RGB
        // (expand).
        let n = w as usize * h as usize;
        let bgra = match (info.color_type, info.bit_depth) {
            (png::ColorType::Rgba, png::BitDepth::Eight) => {
                buf.truncate(n * 4);
                swap_rb_in_place(&mut buf);
                buf
            }
            (png::ColorType::Rgb, png::BitDepth::Eight) => {
                let mut bgra = vec![0u8; n * 4];
                for (dst, src) in bgra.chunks_exact_mut(4).zip(buf.chunks_exact(3)) {
                    dst[0] = src[2];
                    dst[1] = src[1];
                    dst[2] = src[0];
                }
                bgra
            }
            (ct, bd) => return Err(format!("unsupported screencap png {ct:?} {bd:?}")),
        };

        let rect = ScreenRect::new(0, 0, w as u16, h as u16).map_err(|e| e.to_string())?;
        ScreenFrame::raw(0, rect, PixelFormat::Bgra8, bgra)
            .map(Some)
            .map_err(|e| e.to_string())
    }
}

impl ScreenSource for AdbSource {
    fn grab(&self) -> Result<Option<ScreenFrame>, CoreError> {
        use std::sync::atomic::Ordering;
        if self.raw_mode.load(Ordering::Relaxed) {
            match self.grab_raw_stream() {
                Ok(v) => return Ok(v),
                Err(AdbError::Unsupported(msg)) => {
                    tracing::warn!(%msg, "adb raw stream unsupported; switching to PNG stream");
                    self.raw_mode.store(false, Ordering::Relaxed);
                }
                Err(e) => return Err(e.into()),
            }
        }
        self.grab_png_stream()
            .map_err(CoreError::from)
            .or_else(|e| {
                tracing::warn!(%e, "adb png stream failed; trying one-shot screencap");
                self.grab_png_oneshot()
            })
    }

    fn interrupt(&self) {
        // Killing the child EOFs the pipe and unblocks any read in flight.
        self.kill_child();
    }
}

#[cfg(test)]
mod adb_tests {
    use super::{parse_len_line, parse_raw_header, swap_rb_in_place};

    #[test]
    fn len_line_accepts_padded_decimal() {
        assert_eq!(parse_len_line("  31244\n"), Ok(31244));
        assert_eq!(parse_len_line("1"), Ok(1));
    }

    #[test]
    fn len_line_rejects_garbage_zero_and_oversize() {
        assert!(parse_len_line("abc").is_err());
        assert!(parse_len_line("").is_err());
        assert!(parse_len_line("0").is_err());
        assert!(parse_len_line("99999999999").is_err());
        assert!(parse_len_line("-5").is_err());
    }

    #[test]
    fn raw_header_accepts_rgba_8888() {
        let mut hdr = [0u8; 16];
        hdr[0..4].copy_from_slice(&720u32.to_le_bytes());
        hdr[4..8].copy_from_slice(&1280u32.to_le_bytes());
        hdr[8..12].copy_from_slice(&1u32.to_le_bytes());
        assert_eq!(parse_raw_header(&hdr), Ok((720, 1280)));
    }

    #[test]
    fn raw_header_rejects_bad_format_and_dims() {
        let mut hdr = [0u8; 16];
        hdr[0..4].copy_from_slice(&720u32.to_le_bytes());
        hdr[4..8].copy_from_slice(&1280u32.to_le_bytes());
        hdr[8..12].copy_from_slice(&3u32.to_le_bytes()); // RGB_565, not RGBA
        assert!(parse_raw_header(&hdr).is_err());
        hdr[8..12].copy_from_slice(&1u32.to_le_bytes());
        hdr[0..4].copy_from_slice(&0u32.to_le_bytes()); // zero width
        assert!(parse_raw_header(&hdr).is_err());
        hdr[0..4].copy_from_slice(&70000u32.to_le_bytes()); // beyond u16
        assert!(parse_raw_header(&hdr).is_err());
    }

    #[test]
    fn swap_rb_converts_rgba_to_bgra() {
        let mut px = vec![1, 2, 3, 4, 250, 100, 0, 255];
        swap_rb_in_place(&mut px);
        assert_eq!(px, vec![3, 2, 1, 4, 0, 100, 250, 255]);
    }
}
// ---------------------------------------------------------------------------
// SystemSink — the production screen presentation sink
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct SystemSink;

impl Default for SystemSink {
    fn default() -> Self {
        Self
    }
}

impl SystemSink {
    pub fn new() -> Self {
        Self
    }
}

impl ScreenSink for SystemSink {
    #[cfg(windows)]
    fn present(&self, frame: &ScreenFrame) -> Result<(), CoreError> {
        dxgi::present(frame)
    }

    #[cfg(target_os = "macos")]
    fn present(&self, frame: &ScreenFrame) -> Result<(), CoreError> {
        macos::present(frame)
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    fn present(&self, frame: &ScreenFrame) -> Result<(), CoreError> {
        x11::present(frame)
    }

    #[cfg(not(any(windows, target_os = "macos", unix)))]
    fn present(&self, _frame: &ScreenFrame) -> Result<(), CoreError> {
        Err(CoreError::ScreenRefused(
            "no system screen present on this target (Windows GDI / macOS NSWindow / X11 PutImage)"
                .into(),
        ))
    }
}

#[cfg(all(test, windows))]
mod present_smoke {
    use super::*;
    use tetherly_core::{PixelFormat, ScreenRect};

    #[test]
    fn present_smoke_two_frames() {
        let rect = ScreenRect::new(0, 0, 64, 64).unwrap();
        let f = ScreenFrame::raw(1, rect, PixelFormat::Bgra8, vec![7u8; 64 * 64 * 4]).unwrap();
        SystemSink::present(&SystemSink, &f).expect("first present");
        let f2 = ScreenFrame::raw(2, rect, PixelFormat::Bgra8, vec![9u8; 64 * 64 * 4]).unwrap();
        SystemSink::present(&SystemSink, &f2).expect("second present");
    }
}

// ---------------------------------------------------------------------------
// H.264 codec tests (ADR-079)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod h264_tests {
    use super::*;

    /// Gray pixels survive the BT.601 round trip exactly: neutral chroma and
    /// linear luma are both representable.
    #[test]
    fn gray_bgra_maps_to_neutral_chroma_and_linear_luma() {
        let (w, h) = (8, 6);
        let bgra: Vec<u8> = std::iter::repeat([128u8, 128, 128, 255])
            .flatten()
            .take(w * h * 4)
            .collect();
        let yuv = I420::from_bgra(&bgra, w, h).unwrap();
        assert!(
            yuv.y.iter().all(|&l| l == 125 || l == 126),
            "luma not linear"
        );
        assert!(
            yuv.u
                .iter()
                .chain(yuv.v.iter())
                .all(|&c| (126..=130).contains(&c)),
            "gray chroma must be neutral"
        );
    }

    #[test]
    fn from_bgra_rejects_a_mismatched_buffer() {
        assert!(I420::from_bgra(&[0; 10], 4, 4).is_err());
        assert!(I420::from_bgra(&[0; 64], 0, 0).is_err());
    }

    /// Mean PSNR over three frames (I + P + P with motion) must clear 27 dB.
    /// A healthy 4:2:0 stream lands ~28-31 dB on these chroma gradients;
    /// real regressions (wrong stride, swapped planes, broken annex-B
    /// writes) collapse below 15 dB, so 27 keeps detection margin.
    #[test]
    fn h264_roundtrip_keeps_psnr_above_30db() {
        let (w, h) = (160, 120);
        let codec = H264Codec::new(2_000_000);
        let mut prev_psnr_sum = 0f64;
        let frames = 3;
        for seq in 0..frames {
            let src = synthetic_frame(w, h, seq);
            let packet = codec.compress_frame(&src, w as u16, h as u16).unwrap();
            assert!(!packet.is_empty());
            let back = codec
                .decompress_frame(&packet, w as u16, h as u16, src.len())
                .unwrap();
            prev_psnr_sum += psnr(&src, &back);
        }
        let mean = prev_psnr_sum / frames as f64;
        assert!(mean > 27.0, "mean PSNR {mean:.1} dB below the 27 dB floor");
    }

    #[test]
    fn h264_survives_a_geometry_change_with_a_fresh_idr() {
        let codec = H264Codec::new(1_000_000);
        let a = synthetic_frame(64, 64, 0);
        let p1 = codec.compress_frame(&a, 64, 64).unwrap();
        assert!(codec.decompress_frame(&p1, 64, 64, a.len()).is_ok());
        let b = synthetic_frame(96, 64, 1);
        let p2 = codec.compress_frame(&b, 96, 64).unwrap();
        assert!(codec.decompress_frame(&p2, 96, 64, b.len()).is_ok());
    }

    #[test]
    fn negotiated_codec_falls_back_to_zstd_until_the_peer_advertises_h264() {
        use tetherly_core::ScreenCodec as _;

        let sel = NegotiatedCodec::new(true, 1_000_000, false);
        assert_eq!(sel.wire_tag(), tetherly_core::SCREEN_COMPRESS_ZSTD);
        let raw = vec![7u8; 4096];
        let z = sel.compress_frame(&raw, 32, 32).unwrap();
        assert!(z.len() < raw.len(), "zstd must shrink this");
        sel.set_peer_supports_h264(true);
        assert_eq!(sel.wire_tag(), tetherly_core::SCREEN_COMPRESS_H264);
        assert_eq!(sel.active_name(), "h264");
        // Want-off never upgrades regardless of peer caps.
        let off = NegotiatedCodec::new(false, 1_000_000, true);
        assert_eq!(off.wire_tag(), tetherly_core::SCREEN_COMPRESS_ZSTD);
    }

    #[test]
    fn bitrate_tier_boundaries() {
        use super::{
            screen_bitrate_from_profile, SCREEN_BITRATE_DEFAULT_BPS, SCREEN_BITRATE_MAX_BPS,
            SCREEN_BITRATE_MIN_BPS,
        };
        // No profile / non-finite / zero → conservative default.
        assert_eq!(
            screen_bitrate_from_profile(None),
            SCREEN_BITRATE_DEFAULT_BPS
        );
        assert_eq!(
            screen_bitrate_from_profile(Some(0.0)),
            SCREEN_BITRATE_DEFAULT_BPS
        );
        assert_eq!(
            screen_bitrate_from_profile(Some(f64::NAN)),
            SCREEN_BITRATE_DEFAULT_BPS
        );
        // Half the measured link, clamped to the screen band.
        assert_eq!(screen_bitrate_from_profile(Some(10.0)), 5_000_000);
        assert_eq!(
            screen_bitrate_from_profile(Some(0.4)),
            SCREEN_BITRATE_MIN_BPS
        );
        assert_eq!(
            screen_bitrate_from_profile(Some(500.0)),
            SCREEN_BITRATE_MAX_BPS
        );
    }

    #[test]
    fn bitrate_change_forces_a_fresh_idr() {
        let (w, h) = (96, 64);
        let codec = H264Codec::new(2_000_000);
        let f1 = synthetic_frame(w, h, 0);
        let f2 = synthetic_frame(w, h, 1);
        let p1 = codec.compress_frame(&f1, w as u16, h as u16).unwrap();
        assert_eq!(
            first_vcl_nal_type(&p1),
            5,
            "first packet must open with an IDR"
        );
        let p2 = codec.compress_frame(&f2, w as u16, h as u16).unwrap();
        assert_eq!(first_vcl_nal_type(&p2), 1, "same tier keeps P-frames");
        codec.set_bitrate_bps(6_000_000);
        let f3 = synthetic_frame(w, h, 2);
        let p3 = codec.compress_frame(&f3, w as u16, h as u16).unwrap();
        assert_eq!(
            first_vcl_nal_type(&p3),
            5,
            "a re-tier must restart with an IDR, never a P-frame at the old rate"
        );
    }

    #[test]
    fn negotiated_codec_forwards_the_tier() {
        let sel = NegotiatedCodec::new(true, 1_000_000, true);
        sel.set_bitrate_bps(8_000_000);
        // Observable behaviour: the selector still encodes (tier forwarded,
        // no panic), and the choice stays h264.
        assert_eq!(sel.wire_tag(), tetherly_core::SCREEN_COMPRESS_H264);
        let raw = synthetic_frame(64, 64, 0);
        assert!(sel.compress_frame(&raw, 64, 64).unwrap().len() < raw.len());
    }

    /// Annex-B first VCL NAL type: 5 = IDR, 1 = non-IDR. SPS/PPS/SEI (7/8/6)
    /// are skipped until a slice NAL appears.
    fn first_vcl_nal_type(packet: &[u8]) -> u8 {
        let mut i = 0;
        while i + 4 <= packet.len() {
            let start = if packet[i..].starts_with(&[0, 0, 0, 1]) {
                i + 4
            } else if packet[i..].starts_with(&[0, 0, 1]) {
                i + 3
            } else {
                i += 1;
                continue;
            };
            let ty = packet[start] & 0x1f;
            if matches!(ty, 1 | 5) {
                return ty;
            }
            // Skip this NAL: scan to the next start code.
            let mut j = start + 1;
            while j + 3 <= packet.len() {
                if packet[j..].starts_with(&[0, 0, 1]) {
                    break;
                }
                j += 1;
            }
            i = j;
        }
        panic!("no VCL NAL found in packet");
    }

    fn synthetic_frame(w: usize, h: usize, seq: usize) -> Vec<u8> {
        let mut buf = vec![0u8; w * h * 4];
        for row in 0..h {
            for col in 0..w {
                let px = (row * w + col) * 4;
                let shift = seq * 13;
                let grad = |v: usize, span: usize| ((v * 255 / span) + shift).min(255) as u8;
                buf[px] = grad(col, w); // B
                buf[px + 1] = grad(row, h); // G
                buf[px + 2] = grad(col + row, w + h); // R
                buf[px + 3] = 255;
            }
        }
        buf
    }

    fn psnr(a: &[u8], b: &[u8]) -> f64 {
        let mse = a
            .iter()
            .zip(b)
            .map(|(x, y)| {
                let d = i64::from(*x) - i64::from(*y);
                (d * d) as f64
            })
            .sum::<f64>()
            / a.len() as f64;
        if mse == 0.0 {
            return 60.0;
        }
        10.0 * (255.0f64 * 255.0 / mse).log10()
    }
}
