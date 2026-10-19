// SPDX-License-Identifier: Apache-2.0 OR MIT
//! ANCS over Bluetooth LE — the computer side of Phase 4.
//!
//! `tetherly-core::ancs` owns the protocol and the ingress state machine; this
//! module owns the *stack*, the part that actually talks to the phone's GATT
//! server. The ingress drives it through the `AncsTransport` port, which stays
//! synchronous and hardware-free. Inbound traffic comes back over an
//! [`AncsInbound`] channel that the node pumps into the ingress
//! (`Node::spawn_ancs_pump`), so the real stack can keep its own callback
//! thread instead of dragging async BLE into `tetherly-core`.
//!
//! Windows is the implemented stack (WinRT Bluetooth LE GATT, §9.4). macOS
//! (CoreBluetooth) and Linux (BlueZ) are not written yet, so [`open`] honestly
//! returns `Ok(None)` there rather than pretending ANCS works.
//!
//! **Runtime against a real paired iPhone is Manual-required.** The code here is
//! compile-verified; nothing in CI can pair a phone, so no test claims otherwise.

use std::sync::Arc;

use tetherly_core::{AncsTransport, CoreError};

// The stack body lives in its own file, the shape `screen/macos.rs` and
// `screen/x11.rs` use: it imports only absolute paths so it can be read and
// compiled for its target on its own. `ancs.rs` stays the assembly point.
#[cfg(windows)]
#[path = "ancs/win_gatt.rs"]
mod win_gatt;

#[cfg(target_os = "macos")]
#[path = "ancs/macos.rs"]
mod macos;

/// One inbound ANCS event, already copied off the stack's callback thread.
///
/// `tetherly-core` deliberately knows nothing about these: the node translates
/// each one into the matching `AncsIngress` feed point, which is why the
/// transport port needs no inbound method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AncsInbound {
    /// The GATT session is up and all three ANCS characteristics resolved. The
    /// node may now subscribe (Data Source first — `AncsIngress::connect` fixes
    /// that order).
    Ready,
    /// Notification Source value changed; raw bytes for `NotificationEvent`.
    NotificationSource(Vec<u8>),
    /// Data Source value changed; raw bytes, fragments included.
    DataSource(Vec<u8>),
    /// The peripheral went away; the node schedules the backoff reconnect.
    Disconnected,
}

/// A live transport plus the channel its inbound events arrive on.
pub type AncsLink = (
    Arc<dyn AncsTransport>,
    tokio::sync::mpsc::UnboundedReceiver<AncsInbound>,
);

/// Open the platform ANCS BLE transport for the paired iPhone at `address`,
/// written the way Windows Bluetooth settings shows it (`11:22:33:44:55:66`)
/// or macOS CoreBluetooth shows it (`9F82A2E6-B9C0-4E90-B8E5-3A21F5A67C43`).
///
/// Returns `Ok(Some(..))` where a stack exists, `Ok(None)` on targets whose
/// stack is not implemented yet (Linux BlueZ) so the caller can say so honestly,
/// and `Err` only for a malformed address.
pub fn open(address: &str) -> Result<Option<AncsLink>, CoreError> {
    #[cfg(windows)]
    {
        win_gatt::open(address).map(Some)
    }
    #[cfg(target_os = "macos")]
    {
        macos::open(address).map(Some)
    }
    #[cfg(all(not(windows), not(target_os = "macos")))]
    {
        let _ = address;
        Ok(None)
    }
}
