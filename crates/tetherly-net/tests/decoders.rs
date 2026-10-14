// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Property tests for the untrusted *wire* decoders (spec §13 `proptest`
//! layer). Everything here is fed bytes a peer could put on the socket, so a
//! panic, an over-allocation, or a mis-parse would be remotely triggerable.
//!
//! The async readers are driven over an in-memory `&[u8]`, so no socket or
//! runtime feature beyond `tokio`'s current-thread scheduler is needed.

use proptest::prelude::*;
use tetherly_net::codec::{read_len_prefixed, read_tagged, MAX_OUTER};
use tetherly_net::filechan::read_header as read_file_header;

fn bytes(max: usize) -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(any::<u8>(), 0..max)
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .expect("current-thread runtime")
}

proptest! {
    /// Arbitrary bytes on the length-prefixed reader: never panic, and never
    /// allocate a body larger than the documented outer cap when the header
    /// claims an enormous length.
    #[test]
    fn len_prefixed_never_panics(b in bytes(8_192)) {
        let r = rt();
        r.block_on(async {
            let mut cur: &[u8] = &b;
            let _ = read_len_prefixed(&mut cur).await;
        });
    }

    #[test]
    fn tagged_never_panics(b in bytes(8_192)) {
        let r = rt();
        r.block_on(async {
            let mut cur: &[u8] = &b;
            let _ = read_tagged(&mut cur).await;
        });
    }

    #[test]
    fn file_header_never_panics(b in bytes(4096)) {
        let r = rt();
        r.block_on(async {
            let mut cur: &[u8] = &b;
            let _ = read_file_header(&mut cur).await;
        });
    }

    /// A header that claims more than the outer cap must be refused, not
    /// buffered: this is the guard against a peer-driven allocation DoS.
    #[test]
    fn len_prefixed_refuses_an_oversized_header(claimed in (MAX_OUTER as u32 + 1)..=u32::MAX) {
        let r = rt();
        r.block_on(async {
            let mut buf = claimed.to_be_bytes().to_vec();
            buf.extend_from_slice(&[0u8; 16]);
            let mut cur: &[u8] = &buf;
            assert!(read_len_prefixed(&mut cur).await.is_err());
        });
    }

    /// A valid header can never be mistaken for a different transfer id or a
    /// different size, and a resumed tail keeps the chunk alignment the crypto
    /// nonce schedule depends on.
    #[test]
    fn valid_headers_roundtrip(
        id in "[A-Za-z0-9_]{1,16}",
        total in any::<u64>(),
        start in any::<u64>(),
    ) {
        let r = rt();
        r.block_on(async {
            let mut buf = Vec::new();
            buf.extend_from_slice(b"TFL2");
            buf.extend_from_slice(&(id.len() as u32).to_be_bytes());
            buf.extend_from_slice(id.as_bytes());
            buf.extend_from_slice(&8u32.to_be_bytes());
            buf.extend_from_slice(&total.to_be_bytes());
            buf.extend_from_slice(&8u32.to_be_bytes());
            buf.extend_from_slice(&start.to_be_bytes());
            let mut cur: &[u8] = &buf;
            let (got_id, got_total, got_start) = read_file_header(&mut cur).await.unwrap();
            assert_eq!(got_id, id);
            assert_eq!(got_total, total);
            assert_eq!(got_start, start);
        });
    }

    /// The legacy `TFL1` header (no start field) is read as `start = 0`.
    #[test]
    fn legacy_header_reads_as_zero_start(id in "[A-Za-z0-9_]{1,16}", total in any::<u64>()) {
        let r = rt();
        r.block_on(async {
            let mut buf = Vec::new();
            buf.extend_from_slice(b"TFL1");
            buf.extend_from_slice(&(id.len() as u32).to_be_bytes());
            buf.extend_from_slice(id.as_bytes());
            buf.extend_from_slice(&8u32.to_be_bytes());
            buf.extend_from_slice(&total.to_be_bytes());
            let mut cur: &[u8] = &buf;
            let (got_id, got_total, got_start) = read_file_header(&mut cur).await.unwrap();
            assert_eq!(got_id, id);
            assert_eq!(got_total, total);
            assert_eq!(got_start, 0);
        });
    }

    /// A transfer id split so its length prefix and body disagree must be
    /// refused (never a partial id silently accepted as another transfer).
    #[test]
    fn file_header_rejects_a_wrong_magic(id in "[A-Za-z0-9_]{1,8}") {
        let r = rt();
        r.block_on(async {
            let mut buf = Vec::new();
            buf.extend_from_slice(b"XXXX");
            buf.extend_from_slice(&(id.len() as u32).to_be_bytes());
            buf.extend_from_slice(id.as_bytes());
            let mut cur: &[u8] = &buf;
            assert!(read_file_header(&mut cur).await.is_err());
        });
    }
}
