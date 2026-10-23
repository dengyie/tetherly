// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Wire-format helpers for binary payload fields.
//!
//! Inner frames are JSON-encoded inside a hard Noise transport budget
//! (`INNER_PAYLOAD_MAX` = 65507 bytes). A raw `Vec<u8>` field serializes as a
//! JSON array of numbers (~3.8x inflation), which silently blows that budget
//! for chunks near 64 KiB. Base64 inflates only 4/3 (~33KB for a 24 KiB-ish
//! payload), so binary chunk fields ride the wire as base64 strings and the
//! chunk-size caps are set so the fully encoded frame always fits.

use base64::Engine as _;
use serde::{Deserialize, Deserializer, Serializer};

/// serde `with` module for owned `Vec<u8>` fields: base64 string on the wire.
pub mod base64_bytes {
    use super::*;

    pub fn serialize<S: Serializer>(data: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error> {
        let encoded = base64::engine::general_purpose::STANDARD.encode(data.as_slice());
        serializer.serialize_str(&encoded)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<u8>, D::Error> {
        let encoded = String::deserialize(deserializer)?;
        base64::engine::general_purpose::STANDARD
            .decode(encoded.as_bytes())
            .map_err(serde::de::Error::custom)
    }
}

/// serde `with` module for borrowed `&[u8]` fields (streaming zero-copy encode).
pub mod base64_bytes_ref {
    use super::*;

    pub fn serialize<S: Serializer>(data: &&[u8], serializer: S) -> Result<S::Ok, S::Error> {
        let encoded = base64::engine::general_purpose::STANDARD.encode(*data);
        serializer.serialize_str(&encoded)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(_deserializer: D) -> Result<&'de [u8], D::Error> {
        // Borrowed decoding is impossible for base64 (output is longer than a
        // slice of the input); owned decoding is provided by `base64_bytes`.
        Err(serde::de::Error::custom(
            "base64_bytes_ref is encode-only; use base64_bytes for deserialization",
        ))
    }
}

/// Wire budget arithmetic shared by chunked stream frames: the fully encoded
/// inner-frame payload (JSON wrapper + base64 body) must stay below
/// [`crate::frame::INNER_PAYLOAD_MAX`].
pub fn base64_len(len: usize) -> usize {
    len.div_ceil(3) * 4
}

/// Largest binary chunk that survives base64 wire encoding plus the JSON
/// wrapper inside the Noise inner-frame budget. Shared cap for the camera,
/// microphone, and benchmark chunked stream frames.
pub const MAX_WIRE_CHUNK_BYTES: usize = 32_768;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::INNER_PAYLOAD_MAX;

    #[test]
    fn base64_len_matches_engine() {
        for len in [0usize, 1, 2, 3, 4, 1024, 32767, 32768, 32769] {
            let buf = vec![0xABu8; len];
            let encoded = base64::engine::general_purpose::STANDARD.encode(&buf);
            assert_eq!(base64_len(len), encoded.len(), "len={len}");
        }
    }

    #[test]
    fn max_chunk_fits_inner_frame_budget() {
        // Worst-case JSON wrapper for the chunked stream frames (camera/mic/bench):
        // session_id(<=64) + frame_seq/chunk_seq/total_chunks/is_muted/rms_db/
        // payload_bytes/timestamp_ms fields + quotes and commas.
        const JSON_WRAPPER_UPPER_BOUND: usize = 320;
        let encoded = base64_len(32768);
        assert!(
            encoded + JSON_WRAPPER_UPPER_BOUND <= INNER_PAYLOAD_MAX,
            "encoded {encoded} + wrapper must fit {INNER_PAYLOAD_MAX}"
        );
        // And the old 65536 cap must be proven to NOT fit, guarding regression.
        assert!(base64_len(65536) + 96 > INNER_PAYLOAD_MAX);
    }
}
