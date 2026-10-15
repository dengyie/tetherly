// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The Noise transport's plaintext ceiling is a hard, measured number.
//!
//! `snow`'s `TransportState::write_message` refuses a plaintext above
//! `65535 - 16` (the 65535-byte Noise message cap minus the AEAD tag) with
//! `Error::Input` — surfaced here as `CryptoError::Noise("input error")`.
//! The inner frame's own 12-byte header is part of that plaintext, so a
//! caller that budgets only against `INNER_PAYLOAD_MAX` will overflow the
//! transport by 12 bytes. This test pins the exact ceiling so no layer can
//! drift above it again: the whole point is that `encrypt` (not `snow`)
//! reports the failure, and the number is measured from a real handshake.

use tetherly_core::hello::Hello;
use tetherly_crypto::identity::Identity;
use tetherly_crypto::noise::{prologue, NoiseHandshake, NoiseTransport};

/// Complete a local IK handshake between two fresh identities and return the
/// initiator's transport. Mirrors `session.rs`'s handshake shape.
fn initiator_transport() -> (NoiseTransport, Hello, Hello) {
    let a = Identity::from_secrets([1u8; 32], [2u8; 32]);
    let b = Identity::from_secrets([3u8; 32], [4u8; 32]);
    let hello_a = a.hello("a", "test", &["control"]);
    let hello_b = b.hello("b", "test", &["control"]);
    let pro = prologue(&hello_a, &hello_b).expect("prologue");

    let mut hi = NoiseHandshake::initiator(&a, b.n_pk(), &pro).expect("initiator");
    let mut hr = NoiseHandshake::responder(&b, &pro).expect("responder");
    let m1 = hi.write_message(b"").expect("m1");
    hr.read_message(&m1).expect("read m1");
    let m2 = hr.write_message(b"").expect("m2");
    hi.read_message(&m2).expect("read m2");
    let transport = hi.into_transport().expect("transport");
    (transport, hello_a, hello_b)
}

/// The measured ceiling: a plaintext of 65535-16 = 65519 bytes is the largest
/// the transport will accept; one more byte is refused.
#[test]
fn transport_plaintext_ceiling_is_65519_bytes() {
    const CEILING: usize = 65535 - 16;
    let (mut t, _, _) = initiator_transport();

    assert!(
        t.encrypt(&vec![0u8; CEILING]).is_ok(),
        "the transport must accept a plaintext of exactly {CEILING} bytes"
    );
    assert!(
        t.encrypt(&vec![0u8; CEILING + 1]).is_err(),
        "the transport must refuse a plaintext of {} bytes (one over the cap)",
        CEILING + 1
    );
}

/// A fully-sized inner frame — 12-byte header plus `INNER_PAYLOAD_MAX` payload —
/// must now fit inside the transport ceiling exactly. This is the invariant
/// the screen fragmentation depends on: `to_frames` sizes every fragment so its
/// payload is at most `INNER_PAYLOAD_MAX`, and that plus the header plus the tag
/// must be encryptable. If `INNER_PAYLOAD_MAX` ever drifts back above the
/// ceiling, this fails and no fragment could be sent.
#[test]
fn a_max_payload_inner_frame_exactly_fits_the_transport_ceiling() {
    const CEILING: usize = 65535 - 16;
    const INNER_HEADER: usize = 12;
    let inner_plain_len = INNER_HEADER + tetherly_core::frame::INNER_PAYLOAD_MAX;
    assert!(
        inner_plain_len <= CEILING,
        "inner frame plaintext {inner_plain_len} must not exceed the {CEILING}-byte ceiling"
    );

    let (mut t, _, _) = initiator_transport();
    assert!(
        t.encrypt(&vec![0u8; inner_plain_len]).is_ok(),
        "a maximum inner frame must be encryptable in one transport message"
    );
    assert!(
        t.encrypt(&vec![0u8; inner_plain_len + 1]).is_err(),
        "one byte over the maximum inner frame must be refused"
    );
}
