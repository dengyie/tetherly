// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Screen hot-path micro-benchmark: zstd, fragmentation, reassembly.
//! Run: cargo run -p tetherly-node --example bench_screen [--release]

use std::time::Instant;
use tetherly_core::{PixelFormat, ScreenAssembler, ScreenFrame, ScreenRect};

fn synthetic_frame(w: u16, h: u16) -> ScreenFrame {
    // Gradient + block noise: a realistic mix of flat areas and detail.
    let mut px = vec![0u8; w as usize * h as usize * 4];
    for y in 0..h as usize {
        for x in 0..w as usize {
            let i = (y * w as usize + x) * 4;
            px[i] = (x % 256) as u8;
            px[i + 1] = (y % 256) as u8;
            px[i + 2] = ((x / 8) ^ (y / 8)) as u8;
            px[i + 3] = 255;
        }
    }
    ScreenFrame::raw(
        1,
        ScreenRect::new(0, 0, w, h).unwrap(),
        PixelFormat::Bgra8,
        px,
    )
    .unwrap()
}

fn main() {
    let (w, h) = (1920u16, 1080u16);
    let f = synthetic_frame(w, h);
    println!("frame {w}x{h} = {} B raw", f.bytes.len());

    for level in [1i32, 3, 9] {
        let t = Instant::now();
        let enc = zstd::stream::encode_all(f.bytes.as_slice(), level).unwrap();
        let dt_enc = t.elapsed();
        let t = Instant::now();
        let dec = zstd::stream::decode_all(&enc[..]).unwrap();
        let dt_dec = t.elapsed();
        assert_eq!(dec.len(), f.bytes.len());
        println!(
            "zstd stream L{level}: enc {dt_enc:?} -> {} B ({:.0}x) | dec {dt_dec:?}",
            enc.len(),
            f.bytes.len() as f64 / enc.len() as f64
        );
    }

    // Persistent bulk compressor (reused context) vs stream (new ctx per call).
    let mut comp = zstd::bulk::Compressor::new(3).unwrap();
    let mut buf = Vec::with_capacity(f.bytes.len());
    let mut dbuf = vec![0u8; f.bytes.len()];
    // Warm once to size the buffer, then reuse it across frames.
    let mut n = comp.compress_to_buffer(&f.bytes, &mut buf).unwrap();
    buf.truncate(n);
    let t = Instant::now();
    for _ in 0..10 {
        buf.clear();
        n = comp.compress_to_buffer(&f.bytes, &mut buf).unwrap();
        buf.truncate(n);
    }
    println!(
        "zstd bulk L3 reused x10: {:?}/frame -> {n} B",
        t.elapsed() / 10
    );
    let mut decomp = zstd::bulk::Decompressor::new().unwrap();
    let t = Instant::now();
    for _ in 0..10 {
        decomp.decompress_to_buffer(&buf, &mut dbuf).unwrap();
    }
    println!("zstd bulk dec reused x10: {:?}/frame", t.elapsed() / 10);

    // Fragmentation + reassembly on the compressed frame (wire-realistic).
    let enc = zstd::stream::encode_all(f.bytes.as_slice(), 3).unwrap();
    let fc = ScreenFrame {
        codec_tag: tetherly_core::SCREEN_COMPRESS_ZSTD,
        bytes: enc,
        ..f.clone()
    };
    let frames = fc.to_frames().unwrap();
    let t = Instant::now();
    for _ in 0..10 {
        let _ = fc.to_frames().unwrap();
    }
    println!(
        "to_frames x10: {:?}/frame ({} fragments)",
        t.elapsed() / 10,
        frames.len()
    );
    let t = Instant::now();
    for _ in 0..10 {
        let mut asm = ScreenAssembler::new();
        let mut out = None;
        for fr in &frames {
            out = asm.push(fr).unwrap();
        }
        assert!(out.is_some());
    }
    println!(
        "assembler roundtrip x10 (incl new): {:?}/frame",
        t.elapsed() / 10
    );
}
