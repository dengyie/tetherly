// SPDX-License-Identifier: Apache-2.0 OR MIT
//! File data channel on TCP 45718. Control-plane offer/accept stays on 45717.
//! Chunks are ChaCha20-Poly1305 under HKDF(file_token). Never write until the
//! FileHub has Accepted.
//!
//! Wire (magic `TFL2`): `TFL2` | id(len-prefixed) | size u64be | start u64be |
//! then ChaCha20-Poly1305 chunks. Chunk *i* carries plaintext bytes
//! `[start + i*CHUNK, start + (i+1)*CHUNK)` and uses nonce counter `i+1`; the
//! AAD binds `id || chan_offset_le` so a chunk cannot be replayed at a
//! different channel position (resume continuity). `start` must be a multiple
//! of `CHUNK` so a resumed stream still maps to the same nonces/AAD as the
//! original, and a partial file is truncatable back to `start`.
//!
//! `TFL1` (the pre-resume format) is still accepted on receive: it is treated
//! as `TFL2` with `start = 0`.

use crate::codec::{read_len_prefixed, write_len_prefixed};
use crate::error::NetError;
use tetherly_core::hex_sha256;
use tetherly_crypto::{aead_open, aead_seal, counter_nonce, file_token};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const FILE_MAGIC: [u8; 4] = *b"TFL1";
pub const FILE_MAGIC_V2: [u8; 4] = *b"TFL2";
pub const CHUNK: usize = 64 * 1024;

pub fn token_for(handshake_hash: &[u8; 32], transfer_id: &str) -> [u8; 32] {
    file_token(handshake_hash, transfer_id.as_bytes())
}

/// `id || chan_offset_le` — the per-chunk AAD. Binds each chunk to its exact
/// channel position so a chunk cannot be relocated across a resume boundary.
fn aad_for(id: &str, chan_offset: u64) -> Vec<u8> {
    let mut aad = Vec::with_capacity(id.len() + 8);
    aad.extend_from_slice(id.as_bytes());
    aad.extend_from_slice(&chan_offset.to_le_bytes());
    aad
}

/// Send `data[start..]` as encrypted chunks, where `start` is the absolute
/// offset of `data[0]` within the whole file. `start` must be `CHUNK`-aligned.
pub async fn send_chunks_from<W: AsyncWrite + Unpin>(
    w: &mut W,
    token: &[u8; 32],
    transfer_id: &str,
    total: u64,
    start: u64,
    data: &[u8],
) -> Result<(), NetError> {
    debug_assert_eq!(
        start % CHUNK as u64,
        0,
        "resume start must be chunk-aligned"
    );
    w.write_all(&FILE_MAGIC_V2).await?;
    write_len_prefixed(w, transfer_id.as_bytes()).await?;
    write_len_prefixed(w, &total.to_be_bytes()).await?;
    write_len_prefixed(w, &start.to_be_bytes()).await?;
    let mut off = 0usize;
    while off < data.len() {
        let end = (off + CHUNK).min(data.len());
        let chan_offset = start + off as u64;
        let counter = chan_offset / CHUNK as u64 + 1;
        let nonce = counter_nonce(counter);
        let aad = aad_for(transfer_id, chan_offset);
        let ct = aead_seal(token, &nonce, &aad, &data[off..end])?;
        write_len_prefixed(w, &ct).await?;
        off = end;
    }
    w.flush().await?;
    Ok(())
}

/// If `path` exists, truncate it back to a chunk boundary and return the
/// resumable offset. Any trailing partial chunk from an interrupted write is
/// discarded (the sender re-sends that whole chunk). `Ok(0)` when absent.
pub fn resumable_offset(path: &std::path::Path, chunk: u64) -> std::io::Result<u64> {
    match std::fs::metadata(path) {
        Ok(m) => {
            let aligned = m.len() - (m.len() % chunk);
            if aligned != m.len() {
                let f = std::fs::OpenOptions::new().write(true).open(path)?;
                f.set_len(aligned)?;
            }
            Ok(aligned)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(e),
    }
}

/// Read a file-channel header (`TFL1` or `TFL2`) and return
/// `(id, total, start)`. Consumes exactly the header bytes.
pub async fn read_header<R: AsyncRead + Unpin>(r: &mut R) -> Result<(String, u64, u64), NetError> {
    let mut magic = [0u8; 4];
    r.read_exact(&mut magic).await?;
    if magic == FILE_MAGIC_V2 {
        let id = read_len_prefixed(r).await?;
        let id = String::from_utf8(id).map_err(|_| NetError::Handshake)?;
        let total = read_u64(r).await?;
        let start = read_u64(r).await?;
        Ok((id, total, start))
    } else if magic == FILE_MAGIC {
        let id = read_len_prefixed(r).await?;
        let id = String::from_utf8(id).map_err(|_| NetError::Handshake)?;
        let total = read_u64(r).await?;
        Ok((id, total, 0))
    } else {
        Err(NetError::Handshake)
    }
}

async fn read_u64<R: AsyncRead + Unpin>(r: &mut R) -> Result<u64, NetError> {
    let b = read_len_prefixed(r).await?;
    if b.len() != 8 {
        return Err(NetError::Handshake);
    }
    Ok(u64::from_be_bytes(
        b.try_into().map_err(|_| NetError::Handshake)?,
    ))
}

/// Receive chunks appended into `out` (which must already hold exactly `start`
/// bytes for a resumed transfer). Every chunk is decrypted, appended, then the
/// callback persists `(out, offset_after_chunk)` so an interrupt leaves a
/// durable chunk-aligned partial.
pub async fn recv_chunks_into<R, F>(
    r: &mut R,
    token: &[u8; 32],
    expected_id: &str,
    max_size: u64,
    out: &mut Vec<u8>,
    on_chunk: F,
) -> Result<[u8; 32], NetError>
where
    R: AsyncRead + Unpin,
    F: FnMut(&[u8], u64) -> Result<(), NetError>,
{
    let (id, total, start) = read_header(r).await?;
    if id != expected_id {
        return Err(NetError::Handshake);
    }
    recv_chunks_after_header(r, token, expected_id, total, start, max_size, out, on_chunk).await
}

/// Like [`recv_chunks_into`] but for a caller that has already consumed the
/// header (the node must read the id to resolve the token before it can
/// decrypt). `total`/`start` come from [`read_header`].
#[allow(clippy::too_many_arguments)]
pub async fn recv_chunks_after_header<R, F>(
    r: &mut R,
    token: &[u8; 32],
    expected_id: &str,
    total: u64,
    start: u64,
    max_size: u64,
    out: &mut Vec<u8>,
    mut on_chunk: F,
) -> Result<[u8; 32], NetError>
where
    R: AsyncRead + Unpin,
    F: FnMut(&[u8], u64) -> Result<(), NetError>,
{
    if total > max_size {
        return Err(NetError::TooLarge);
    }
    if start % CHUNK as u64 != 0 || start as usize != out.len() || out.len() as u64 > total {
        return Err(NetError::Handshake);
    }
    while (out.len() as u64) < total {
        let ct = read_len_prefixed(r).await?;
        let chan_offset = out.len() as u64;
        let counter = chan_offset / CHUNK as u64 + 1;
        let nonce = counter_nonce(counter);
        let aad = aad_for(expected_id, chan_offset);
        let pt = aead_open(token, &nonce, &aad, &ct)?;
        out.extend_from_slice(&pt);
        if out.len() as u64 > total {
            return Err(NetError::TooLarge);
        }
        on_chunk(out, out.len() as u64)?;
    }
    if out.len() as u64 != total {
        return Err(NetError::Handshake);
    }
    parse_sha256_hex(&hex_sha256(out))
}

/// Non-resumable whole-buffer send (start 0). Kept for callers/tests that do
/// not resume.
pub async fn send_bytes<W: AsyncWrite + Unpin>(
    w: &mut W,
    token: &[u8; 32],
    transfer_id: &str,
    data: &[u8],
) -> Result<[u8; 32], NetError> {
    send_chunks_from(w, token, transfer_id, data.len() as u64, 0, data).await?;
    parse_sha256_hex(&hex_sha256(data))
}

/// Whole-buffer receive from start 0, verifying the digest. Kept for callers
/// that do not persist partials.
pub async fn recv_bytes<R: AsyncRead + Unpin>(
    r: &mut R,
    token: &[u8; 32],
    expected_id: &str,
    max_size: u64,
) -> Result<(Vec<u8>, [u8; 32]), NetError> {
    let mut out = Vec::new();
    let digest = recv_chunks_into(r, token, expected_id, max_size, &mut out, |_, _| Ok(())).await?;
    Ok((out, digest))
}

fn parse_sha256_hex(hex: &str) -> Result<[u8; 32], NetError> {
    if hex.len() != 64 {
        return Err(NetError::File("sha256 length".into()));
    }
    let mut out = [0u8; 32];
    for i in 0..32 {
        let byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16)
            .map_err(|_| NetError::File("sha256 hex".into()))?;
        out[i] = byte;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    #[tokio::test]
    async fn file_bytes_roundtrip() {
        let token = [3u8; 32];
        let payload = vec![7u8; 100_000];
        let (mut a, mut b) = duplex(256 * 1024);
        let send = send_bytes(&mut a, &token, "tr_x", &payload);
        let recv = recv_bytes(&mut b, &token, "tr_x", 2 * 1024 * 1024);
        let (sent, recvd) = tokio::join!(send, recv);
        let sh = sent.unwrap();
        let (bytes, rh) = recvd.unwrap();
        assert_eq!(bytes, payload);
        assert_eq!(sh, rh);
    }

    #[tokio::test]
    async fn resume_sends_only_the_tail_from_a_chunk_boundary() {
        let token = [9u8; 32];
        let total = 3 * CHUNK + 1234;
        let full: Vec<u8> = (0..total).map(|i| (i % 251) as u8).collect();
        // sink writes into a duplex and reads the header + tail chunks.
        let (mut w, mut r) = duplex(8 * CHUNK);
        let start = CHUNK as u64;
        let tail = full[start as usize..].to_vec();
        let send = send_chunks_from(&mut w, &token, "tr_r", total as u64, start, &tail);
        // Preload the receiver with the first `start` bytes.
        let mut party = full[..start as usize].to_vec();
        let recv = recv_chunks_into(&mut r, &token, "tr_r", u64::MAX, &mut party, |_, _| Ok(()));
        let (s, rv) = tokio::join!(send, recv);
        s.unwrap();
        let digest = rv.unwrap();
        assert_eq!(party.len() as u64, total as u64);
        assert_eq!(party, full);
        assert_eq!(digest, parse_sha256_hex(&hex_sha256(&full)).unwrap());
    }

    #[tokio::test]
    async fn a_chunk_cannot_be_relocated_across_resume_positions() {
        let token = [1u8; 32];
        // Build a stream that claims start=CHUNK but whose single chunk was
        // sealed for channel offset 0 -> AAD mismatch must be rejected.
        let (mut w, mut r) = duplex(64 * 1024);
        let bad = tetherly_crypto::aead_seal(&token, &counter_nonce(1), &aad_for("tr_a", 0), b"xy")
            .unwrap();
        tokio::spawn(async move {
            w.write_all(&FILE_MAGIC_V2).await.unwrap();
            write_len_prefixed(&mut w, b"tr_a").await.unwrap();
            write_len_prefixed(&mut w, &(CHUNK as u64 + 2).to_be_bytes())
                .await
                .unwrap();
            write_len_prefixed(&mut w, &(CHUNK as u64).to_be_bytes())
                .await
                .unwrap();
            write_len_prefixed(&mut w, &bad).await.unwrap();
        });
        let mut out = vec![0u8; CHUNK];
        let err = recv_chunks_into(&mut r, &token, "tr_a", u64::MAX, &mut out, |_, _| Ok(()))
            .await
            .unwrap_err();
        assert!(matches!(err, NetError::Crypto(_)));
    }

    #[test]
    fn resumable_offset_truncates_partial_tail() {
        let dir = std::env::temp_dir().join(format!("tfl_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("x.part");
        std::fs::write(&p, vec![0u8; CHUNK + 5]).unwrap();
        let off = resumable_offset(&p, CHUNK as u64).unwrap();
        assert_eq!(off, CHUNK as u64);
        assert_eq!(std::fs::metadata(&p).unwrap().len(), CHUNK as u64);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn truncated_stream_is_an_error_not_a_silent_short_file() {
        // A body that ends before `total` bytes must fail: the caller sha256s
        // only on Ok, so a short read must never look like success.
        let token = [5u8; 32];
        let total = CHUNK as u64 + 10;
        let (mut w, mut r) = duplex(8 * CHUNK);
        // Write a valid header promising `total` bytes, then close with no chunks.
        tokio::spawn(async move {
            w.write_all(&FILE_MAGIC_V2).await.unwrap();
            write_len_prefixed(&mut w, b"tr_t").await.unwrap();
            write_len_prefixed(&mut w, &total.to_be_bytes())
                .await
                .unwrap();
            write_len_prefixed(&mut w, &0u64.to_be_bytes())
                .await
                .unwrap();
            // drop w -> EOF
        });
        let mut out = Vec::new();
        let err = recv_chunks_into(&mut r, &token, "tr_t", u64::MAX, &mut out, |_, _| Ok(()))
            .await
            .unwrap_err();
        assert!(matches!(err, NetError::Io(_)), "{err:?}");
    }

    #[tokio::test]
    async fn resume_start_must_match_the_preloaded_length() {
        // Claims start=0 (so out must be empty) but the caller preloaded bytes:
        // the guard must reject rather than append at the wrong channel offset.
        let token = [6u8; 32];
        let (mut w, mut r) = duplex(8 * CHUNK);
        let send = send_chunks_from(&mut w, &token, "tr_m", CHUNK as u64 * 2, 0, &[0u8; 16]);
        let mut out = vec![9u8; 32]; // not a valid TFL2 start=0 receiver state
        let recv = recv_chunks_into(&mut r, &token, "tr_m", u64::MAX, &mut out, |_, _| Ok(()));
        let (s, rv) = tokio::join!(send, recv);
        s.unwrap();
        assert!(matches!(rv, Err(NetError::Handshake)), "{rv:?}");
    }

    #[tokio::test]
    async fn tfl1_legacy_header_is_read_as_start_zero() {
        // The pre-resume format must still decode (treated as start=0) so an old
        // sender is not silently dropped.
        let token = [8u8; 32];
        let payload = vec![3u8; 100];
        let (mut w, mut r) = duplex(64 * 1024);
        let p = payload.clone();
        tokio::spawn(async move {
            w.write_all(&FILE_MAGIC).await.unwrap();
            write_len_prefixed(&mut w, b"tr_legacy").await.unwrap();
            write_len_prefixed(&mut w, &(p.len() as u64).to_be_bytes())
                .await
                .unwrap();
            let ct = aead_seal(&token, &counter_nonce(1), &aad_for("tr_legacy", 0), &p).unwrap();
            write_len_prefixed(&mut w, &ct).await.unwrap();
        });
        let mut out = Vec::new();
        recv_chunks_into(&mut r, &token, "tr_legacy", u64::MAX, &mut out, |_, _| {
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(out, payload);
    }
}
