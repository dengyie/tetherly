// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Windows ANCS over WinRT Bluetooth LE. Self-contained: it imports only
//! absolute paths, the shape `screen/macos.rs` and `screen/x11.rs` use.
//!
//! One dedicated thread owns every WinRT interface and its own MTA apartment.
//! The `AncsTransport` front end only moves commands across a channel and waits
//! for the acknowledgement, so the port stays `Send + Sync` and the node never
//! touches a COM interface directly.
//!
//! The §9.4 hard constraints land like this:
//! - "subscribe Data Source before Notification Source" is the *ingress*'s job
//!   (`AncsIngress::connect` orders the two calls); this file performs whatever
//!   it is told, in order.
//! - "never write the Control Point from a value-changed thread": the callbacks
//!   here only forward bytes; every write arrives as a command on the driver
//!   thread.
//! - "Control Point writes are serial": the ingress keeps at most one request in
//!   flight and `write_control_point` blocks until the stack reports `Success`.
//!
//! Runtime against a physically paired iPhone is Manual-required.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::Duration;

use tetherly_core::ancs::{
    ancs_backoff_delay, uuid_bytes_le, UUID_ANCS_SERVICE, UUID_CONTROL_POINT, UUID_DATA_SOURCE,
    UUID_NOTIFICATION_SOURCE,
};
use tetherly_core::{AncsTransport, CoreError};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};
use windows::core::{Error as WinError, IInspectable, GUID};
use windows::Devices::Bluetooth::GenericAttributeProfile::{
    GattCharacteristic, GattCharacteristicProperties,
    GattClientCharacteristicConfigurationDescriptorValue, GattCommunicationStatus,
    GattDeviceService, GattValueChangedEventArgs,
};
use windows::Devices::Bluetooth::{BluetoothConnectionStatus, BluetoothLEDevice};
use windows::Foundation::Collections::IVectorView;
use windows::Foundation::TypedEventHandler;
use windows::Storage::Streams::{DataReader, DataWriter, IBuffer};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

use crate::ancs::AncsInbound;

/// How long the front end waits for the driver to acknowledge one command. A
/// wedged GATT stack must not park the node's ANCS pump forever; the operation is
/// reported as failed instead of silently pretended (the shape `tetherly-net`
/// uses for a stalled session write).
const ACK_TIMEOUT: Duration = Duration::from_secs(10);

/// The driver polls its command channel in slices, so a peripheral disconnect is
/// noticed promptly without a second thread watching the connection.
const SERVE_SLICE: Duration = Duration::from_millis(250);

// ------------------------------------------------------------- front end

/// A command for the driver thread, with the slot it answers on.
enum Cmd {
    SubscribeData(Sender<Result<(), CoreError>>),
    SubscribeNotification(Sender<Result<(), CoreError>>),
    Write(Vec<u8>, Sender<Result<(), CoreError>>),
    /// The front end is gone; stop for good. Unacknowledged by design.
    Stop,
}

/// Front end for the driver thread. `Send + Sync` by construction: it holds a
/// `Sender` and nothing else.
pub struct WinAncsTransport {
    cmd: Sender<Cmd>,
}

impl WinAncsTransport {
    fn call(
        &self,
        build: impl FnOnce(Sender<Result<(), CoreError>>) -> Cmd,
        dead: CoreError,
    ) -> Result<(), CoreError> {
        let (ack, answer) = channel();
        if self.cmd.send(build(ack)).is_err() {
            // The driver thread is gone (device unplugged, process shutting
            // down): report the failure rather than a success we cannot prove.
            return Err(dead);
        }
        match answer.recv_timeout(ACK_TIMEOUT) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => {
                tracing::warn!("ancs: the BLE stack did not answer in time");
                Err(dead)
            }
            Err(RecvTimeoutError::Disconnected) => Err(dead),
        }
    }
}

impl AncsTransport for WinAncsTransport {
    fn subscribe_data_source(&self) -> Result<(), CoreError> {
        self.call(Cmd::SubscribeData, CoreError::AncsSubscribe)
    }

    fn subscribe_notification_source(&self) -> Result<(), CoreError> {
        self.call(Cmd::SubscribeNotification, CoreError::AncsSubscribe)
    }

    fn write_control_point(&self, bytes: &[u8]) -> Result<(), CoreError> {
        let owned = bytes.to_vec();
        self.call(|ack| Cmd::Write(owned, ack), CoreError::AncsWrite)
    }
}

impl Drop for WinAncsTransport {
    fn drop(&mut self) {
        let _ = self.cmd.send(Cmd::Stop);
    }
}

/// Spawn the driver thread and hand back the transport plus its inbound channel.
pub fn open(
    address: &str,
) -> Result<(Arc<dyn AncsTransport>, UnboundedReceiver<AncsInbound>), CoreError> {
    let address = parse_address(address)?;
    let (cmd, cmd_rx) = channel();
    let (in_tx, in_rx) = unbounded_channel();
    std::thread::Builder::new()
        .name("tetherly-ancs".into())
        .spawn(move || driver(address, in_tx, cmd_rx))
        .map_err(|_| CoreError::AncsFrame)?;
    Ok((Arc::new(WinAncsTransport { cmd }), in_rx))
}

/// Parse Windows Bluetooth notation (`11:22:33:44:55:66`, `:` or `-` separated,
/// or bare hex) into the 48-bit address WinRT wants, most significant octet
/// first. Exits as soon as the input cannot be an address.
fn parse_address(text: &str) -> Result<u64, CoreError> {
    let mut value = 0u64;
    let mut digits = 0usize;
    for byte in text.bytes() {
        if byte == b':' || byte == b'-' {
            continue;
        }
        let nibble = (byte as char).to_digit(16).ok_or(CoreError::AncsFrame)? as u64;
        value = value
            .checked_mul(16)
            .and_then(|v| v.checked_add(nibble))
            .ok_or(CoreError::AncsFrame)?;
        digits += 1;
        if digits > 12 {
            return Err(CoreError::AncsFrame);
        }
    }
    if digits != 12 {
        return Err(CoreError::AncsFrame);
    }
    Ok(value)
}

/// Canonical text UUID to the WinRT `GUID`. `uuid_bytes_le` already produces the
/// GUID in-memory layout (least-significant field first), so reading it back as
/// a little-endian integer recovers the canonical 128-bit value.
fn guid(canonical: &str) -> Result<GUID, String> {
    let bytes = uuid_bytes_le(canonical).ok_or_else(|| format!("malformed UUID {canonical}"))?;
    Ok(GUID::from_u128(u128::from_le_bytes(bytes)))
}

// --------------------------------------------------------------- driver

/// Owns the WinRT objects for the life of one process: establish a session,
/// serve commands until the phone goes away, then back off and try again.
fn driver(address: u64, out: UnboundedSender<AncsInbound>, cmd: Receiver<Cmd>) {
    // WinRT activation needs an initialized apartment on this thread. MTA is the
    // right model: the two callbacks are invoked by the Bluetooth stack, never by
    // a UI pump. `S_FALSE` (already initialized) is not an error.
    let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if hr.is_err() {
        tracing::error!(error = %WinError::from(hr), "ancs: COM apartment unavailable; BLE stays off");
        return;
    }

    let mut attempt = 0u32;
    loop {
        match Session::establish(address, &out) {
            Ok(session) => {
                attempt = 0;
                if out.send(AncsInbound::Ready).is_err() {
                    // The node dropped its pump; nothing left to feed.
                    return;
                }
                if !session.serve(&cmd) {
                    return;
                }
                let _ = out.send(AncsInbound::Disconnected);
            }
            Err(reason) => {
                tracing::warn!(%reason, "ancs: GATT session unavailable");
            }
        }
        std::thread::sleep(Duration::from_millis(ancs_backoff_delay(attempt)));
        attempt = attempt.saturating_add(1);
    }
}

/// One established GATT session. Dropping it detaches the callbacks.
struct Session {
    device: BluetoothLEDevice,
    notification_source: GattCharacteristic,
    data_source: GattCharacteristic,
    control_point: GattCharacteristic,
    /// Kept alive so the characteristics stay resolvable for the session's life.
    _service: GattDeviceService,
    disconnected: Arc<AtomicBool>,
    notification_token: i64,
    data_token: i64,
    status_token: i64,
}

impl Drop for Session {
    fn drop(&mut self) {
        // Detach before the interfaces go away: a registration left behind would
        // leak, and the stack could still call a handler after the front end that
        // owns its channel is gone.
        let _ = self
            .notification_source
            .RemoveValueChanged(self.notification_token);
        let _ = self.data_source.RemoveValueChanged(self.data_token);
        let _ = self.device.RemoveConnectionStatusChanged(self.status_token);
    }
}

impl Session {
    fn establish(address: u64, out: &UnboundedSender<AncsInbound>) -> Result<Self, String> {
        // Resolves only for a device Windows already knows (paired); an absent or
        // unpaired address fails here, which is the honest outcome.
        let device = BluetoothLEDevice::FromBluetoothAddressAsync(address)
            .and_then(|op| op.get())
            .map_err(fail("FromBluetoothAddressAsync"))?;

        let services = device
            .GetGattServicesForUuidAsync(guid(UUID_ANCS_SERVICE)?)
            .and_then(|op| op.get())
            .map_err(fail("GetGattServicesForUuidAsync"))?;
        let status = services.Status().map_err(fail("Services::Status"))?;
        if status != GattCommunicationStatus::Success {
            return Err(format!("ANCS service query returned {status:?}"));
        }
        let found = services.Services().map_err(fail("Services::Services"))?;
        if found.Size().map_err(fail("IVectorView::Size"))? == 0 {
            return Err("device exposes no ANCS service (paired iPhone?)".into());
        }
        let service = found.GetAt(0).map_err(fail("IVectorView::GetAt"))?;

        let chars = service
            .GetCharacteristicsAsync()
            .and_then(|op| op.get())
            .map_err(fail("GetCharacteristicsAsync"))?;
        let chars = chars
            .Characteristics()
            .map_err(fail("Characteristics::Characteristics"))?;

        // ANCS is only usable if each characteristic carries the property the
        // protocol needs; a missing one is reported, not papered over.
        let notification_source = find(
            &chars,
            UUID_NOTIFICATION_SOURCE,
            GattCharacteristicProperties::Notify,
        )?;
        let data_source = find(
            &chars,
            UUID_DATA_SOURCE,
            GattCharacteristicProperties::Notify,
        )?;
        let control_point = find(
            &chars,
            UUID_CONTROL_POINT,
            GattCharacteristicProperties::Write,
        )?;

        let disconnected = Arc::new(AtomicBool::new(false));
        let flag = disconnected.clone();
        let status_token = device
            .ConnectionStatusChanged(&TypedEventHandler::<BluetoothLEDevice, IInspectable>::new(
                move |sender: &Option<BluetoothLEDevice>, _| {
                    let gone = sender
                        .as_ref()
                        .and_then(|device| device.ConnectionStatus().ok())
                        .map(|s| s == BluetoothConnectionStatus::Disconnected)
                        .unwrap_or(false);
                    if gone {
                        flag.store(true, Ordering::SeqCst);
                    }
                    Ok(())
                },
            ))
            .map_err(fail("ConnectionStatusChanged"))?;

        let notification_token = watch(
            &notification_source,
            out.clone(),
            AncsInbound::NotificationSource,
        )
        .map_err(fail("NotificationSource::ValueChanged"))?;
        let data_token = watch(&data_source, out.clone(), AncsInbound::DataSource)
            .map_err(fail("DataSource::ValueChanged"))?;

        Ok(Self {
            device,
            notification_source,
            data_source,
            control_point,
            _service: service,
            disconnected,
            notification_token,
            data_token,
            status_token,
        })
    }

    /// Serve commands until the phone goes away or the front end is dropped.
    /// Returns `false` when the driver thread should stop for good.
    fn serve(&self, cmd: &Receiver<Cmd>) -> bool {
        loop {
            if self.disconnected.load(Ordering::SeqCst) {
                tracing::info!("ancs: peripheral disconnected");
                return true;
            }
            match cmd.recv_timeout(SERVE_SLICE) {
                Ok(Cmd::SubscribeData(ack)) => {
                    let _ = ack.send(subscribe(&self.data_source));
                }
                Ok(Cmd::SubscribeNotification(ack)) => {
                    let _ = ack.send(subscribe(&self.notification_source));
                }
                Ok(Cmd::Write(bytes, ack)) => {
                    let _ = ack.send(write_control_point(&self.control_point, &bytes));
                }
                Ok(Cmd::Stop) | Err(RecvTimeoutError::Disconnected) => return false,
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
    }
}

/// Register a value-changed handler that forwards the bytes to `out`. The
/// callback never writes the Control Point — it only copies a value across.
fn watch(
    characteristic: &GattCharacteristic,
    out: UnboundedSender<AncsInbound>,
    wrap: fn(Vec<u8>) -> AncsInbound,
) -> windows::core::Result<i64> {
    characteristic.ValueChanged(&TypedEventHandler::<
        GattCharacteristic,
        GattValueChangedEventArgs,
    >::new(
        move |_, args: &Option<GattValueChangedEventArgs>| {
            let value = args
                .as_ref()
                .and_then(|args| args.CharacteristicValue().ok())
                .and_then(|buffer| buffer_bytes(&buffer).ok());
            if let Some(bytes) = value {
                let _ = out.send(wrap(bytes));
            }
            Ok(())
        },
    ))
}

/// Find the ANCS characteristic with `uuid` and check it can do `required`.
fn find(
    characteristics: &IVectorView<GattCharacteristic>,
    uuid: &str,
    required: GattCharacteristicProperties,
) -> Result<GattCharacteristic, String> {
    let want = guid(uuid)?;
    let size = characteristics.Size().map_err(fail("IVectorView::Size"))?;
    for index in 0..size {
        let candidate = characteristics
            .GetAt(index)
            .map_err(fail("IVectorView::GetAt"))?;
        if candidate.Uuid().map_err(fail("Uuid"))? != want {
            continue;
        }
        let properties = candidate
            .CharacteristicProperties()
            .map_err(fail("CharacteristicProperties"))?;
        if !properties.contains(required) {
            return Err(format!(
                "characteristic {uuid} is missing a required property"
            ));
        }
        return Ok(candidate);
    }
    Err(format!("this device has no {uuid} characteristic"))
}

/// Enable notifications: write `Notify` to the characteristic's client
/// configuration descriptor (CCCD) and require the stack to confirm it.
fn subscribe(characteristic: &GattCharacteristic) -> Result<(), CoreError> {
    let status = characteristic
        .WriteClientCharacteristicConfigurationDescriptorAsync(
            GattClientCharacteristicConfigurationDescriptorValue::Notify,
        )
        .and_then(|op| op.get())
        .map_err(|error| {
            tracing::warn!(%error, "ancs: enabling notifications failed");
            CoreError::AncsSubscribe
        })?;
    if status != GattCommunicationStatus::Success {
        tracing::warn!(
            ?status,
            "ancs: the phone refused the notification subscription"
        );
        return Err(CoreError::AncsSubscribe);
    }
    Ok(())
}

/// One Control Point write. Blocks until the stack reports the result, which is
/// what makes "one request in flight" true rather than merely assumed.
fn write_control_point(characteristic: &GattCharacteristic, bytes: &[u8]) -> Result<(), CoreError> {
    let writer = DataWriter::new().map_err(control_point_failed)?;
    writer.WriteBytes(bytes).map_err(control_point_failed)?;
    let buffer = writer.DetachBuffer().map_err(control_point_failed)?;
    let result = characteristic
        .WriteValueWithResultAsync(&buffer)
        .and_then(|op| op.get())
        .map_err(control_point_failed)?;
    let status = result.Status().map_err(control_point_failed)?;
    if status != GattCommunicationStatus::Success {
        tracing::warn!(?status, "ancs: the phone refused a control point write");
        return Err(CoreError::AncsWrite);
    }
    Ok(())
}

fn control_point_failed(error: WinError) -> CoreError {
    tracing::warn!(%error, "ancs: control point write failed");
    CoreError::AncsWrite
}

/// Copy a GATT value out of its `IBuffer`. `DataReader` handles the buffer's
/// length prefix, so this is a plain byte copy.
fn buffer_bytes(buffer: &IBuffer) -> windows::core::Result<Vec<u8>> {
    let length = buffer.Length()? as usize;
    let reader = DataReader::FromBuffer(buffer)?;
    let mut bytes = vec![0u8; length];
    if length != 0 {
        reader.ReadBytes(&mut bytes)?;
    }
    Ok(bytes)
}

fn fail(what: &'static str) -> impl Fn(WinError) -> String {
    move |error| format!("{what}: {error}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_address_notation_parses() {
        assert_eq!(
            parse_address("11:22:33:44:55:66").unwrap(),
            0x1122_3344_5566
        );
        assert_eq!(
            parse_address("11-22-33-44-55-66").unwrap(),
            0x1122_3344_5566
        );
        assert_eq!(parse_address("aabbccddeeff").unwrap(), 0xAABB_CCDD_EEFF);
        // The most significant octet is the first one written, as Windows shows it.
        assert_eq!(
            parse_address("AA:BB:CC:DD:EE:FF").unwrap(),
            0xAABB_CCDD_EEFF
        );
    }

    #[test]
    fn malformed_addresses_are_refused() {
        for bad in [
            "",
            "11:22:33:44:55",       // too short
            "11:22:33:44:55:66:77", // too long
            "11:22:33:44:55:6g",    // not hex
            "0x112233445566",       // prefix is not part of the notation
            "11:22:33:44:55:66:zz", // trailing garbage
        ] {
            assert!(parse_address(bad).is_err(), "{bad} should be refused");
        }
    }

    #[test]
    fn ancs_uuids_decode_to_their_published_guids() {
        // The little-endian decode has to agree with the canonical text form,
        // field by field; a wrong mapping would silently query the wrong service.
        assert!(
            guid(UUID_ANCS_SERVICE).unwrap()
                == GUID::from_u128(0x7905_F431_B5CE_4E99_A40F_4B1E_122D_00D0)
        );
        assert!(
            guid(UUID_NOTIFICATION_SOURCE).unwrap()
                == GUID::from_u128(0x9FBF_120D_6301_42D9_8C58_25E6_99A2_1DBD)
        );
        assert!(
            guid(UUID_CONTROL_POINT).unwrap()
                == GUID::from_u128(0x69D1_D8F3_45E1_49A8_9821_9BBD_FDAA_D9D9)
        );
        assert!(
            guid(UUID_DATA_SOURCE).unwrap()
                == GUID::from_u128(0x22EA_C6E9_24D6_4BB5_BE44_B36A_CE7C_7BFB)
        );
    }

    #[test]
    fn a_malformed_uuid_is_reported() {
        assert!(guid("not-a-uuid").is_err());
    }
}
