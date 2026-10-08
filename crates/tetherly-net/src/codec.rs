// SPDX-License-Identifier: Apache-2.0 OR MIT

use crate::error::NetError;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

pub const MAX_OUTER: usize = 256 * 1024 + 32;

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
