// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::error::NetError;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const MAX_OUTER: usize = 256 * 1024 + 32;

/// Post-handshake wire tag: the body is a Noise transport ciphertext of one
/// inner frame.
pub const TAG_DATA: u8 = 0x01;
/// Post-handshake wire tag: the body is one Noise handshake message of an
/// in-band rekey. Both peers run the same framing, so the tag needs no
/// negotiation.
pub const TAG_REKEY: u8 = 0x02;

pub async fn write_len_prefixed<W: AsyncWrite + Unpin>(
    w: &mut W,
    payload: &[u8],
) -> Result<(), NetError> {
    if payload.len() > MAX_OUTER {
        return Err(NetError::TooLarge);
    }
    w.write_all(&(payload.len() as u32).to_be_bytes()).await?;
    w.write_all(payload).await?;
    w.flush().await?;
    Ok(())
}

pub async fn read_len_prefixed<R: AsyncRead + Unpin>(r: &mut R) -> Result<Vec<u8>, NetError> {
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_OUTER {
        return Err(NetError::TooLarge);
    }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await?;
    Ok(buf)
}

/// Write `tag | u32 len | body`. The single tag byte lets the reader tell a
/// transport frame from a rekey handshake message without decrypting first (a
/// decrypt failure cannot be distinguished from a handshake message).
pub async fn write_tagged<W: AsyncWrite + Unpin>(
    w: &mut W,
    tag: u8,
    body: &[u8],
) -> Result<(), NetError> {
    if body.len() > MAX_OUTER {
        return Err(NetError::TooLarge);
    }
    w.write_all(&[tag]).await?;
    w.write_all(&(body.len() as u32).to_be_bytes()).await?;
    w.write_all(body).await?;
    w.flush().await?;
    Ok(())
}

/// Read one tagged message written by [`write_tagged`].
pub async fn read_tagged<R: AsyncRead + Unpin>(r: &mut R) -> Result<(u8, Vec<u8>), NetError> {
    let mut tag = [0u8; 1];
    r.read_exact(&mut tag).await?;
    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > MAX_OUTER {
        return Err(NetError::TooLarge);
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).await?;
    Ok((tag[0], body))
}
