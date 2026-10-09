// SPDX-License-Identifier: Apache-2.0 OR MIT
//! File data channel on TCP 45718. Control-plane offer/accept stays on 45717.
//! Chunks are ChaCha20-Poly1305 under HKDF(file_token). Never write until the
//! FileHub has Accepted.

use crate::codec::{read_len_prefixed, write_len_prefixed};
use crate::error::NetError;
use tetherly_core::hex_sha256;
use tetherly_crypto::{aead_open, aead_seal, counter_nonce, file_token};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const FILE_MAGIC: [u8; 4] = *b"TFL1";
pub const CHUNK: usize = 64 * 1024;

pub fn token_for(handshake_hash: &[u8; 32], transfer_id: &str) -> [u8; 32] {
    file_token(handshake_hash, transfer_id.as_bytes())
}

pub async fn send_bytes<W: AsyncWrite + Unpin>(
    w: &mut W,
    token: &[u8; 32],
    transfer_id: &str,
    data: &[u8],
) -> Result<[u8; 32], NetError> {
    w.write_all(&FILE_MAGIC).await?;
    write_len_prefixed(w, transfer_id.as_bytes()).await?;
    write_len_prefixed(w, &(data.len() as u64).to_be_bytes()).await?;
    let digest = hex_sha256(data);
    let mut offset = 0usize;
    let mut counter = 1u64;
    let aad = transfer_id.as_bytes();
    while offset < data.len() {
        let end = (offset + CHUNK).min(data.len());
        let nonce = counter_nonce(counter);
        let ct = aead_seal(token, &nonce, aad, &data[offset..end])?;
        write_len_prefixed(w, &ct).await?;
        counter = counter.saturating_add(1);
        offset = end;
    }
    w.flush().await?;
    parse_sha256_hex(&digest)
}

pub async fn recv_bytes<R: AsyncRead + Unpin>(
    r: &mut R,
    token: &[u8; 32],
    expected_id: &str,
    max_size: u64,
) -> Result<(Vec<u8>, [u8; 32]), NetError> {
    let mut magic = [0u8; 4];
    r.read_exact(&mut magic).await?;
    if magic != FILE_MAGIC {
        return Err(NetError::Handshake);
    }
    let id = read_len_prefixed(r).await?;
    let id = String::from_utf8(id).map_err(|_| NetError::Handshake)?;
    if id != expected_id {
        return Err(NetError::Handshake);
    }
    let size_bytes = read_len_prefixed(r).await?;
    if size_bytes.len() != 8 {
        return Err(NetError::Handshake);
    }
    let size = u64::from_be_bytes(size_bytes.try_into().map_err(|_| NetError::Handshake)?);
    if size > max_size {
        return Err(NetError::TooLarge);
    }
    let mut out = Vec::with_capacity(size as usize);
    let mut counter = 1u64;
    let aad = expected_id.as_bytes();
    while (out.len() as u64) < size {
        let ct = read_len_prefixed(r).await?;
        let nonce = counter_nonce(counter);
        let pt = aead_open(token, &nonce, aad, &ct)?;
        out.extend_from_slice(&pt);
        counter = counter.saturating_add(1);
        if out.len() as u64 > size {
            return Err(NetError::TooLarge);
        }
    }
    if out.len() as u64 != size {
        return Err(NetError::Handshake);
    }
    let digest = parse_sha256_hex(&hex_sha256(&out))?;
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
}
