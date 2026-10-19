// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Linux ANCS over BlueZ D-Bus. Self-contained: it imports only
//! absolute paths, matching `screen/x11.rs` and `cred/linux.rs`.
//!
//! One dedicated background thread owns the D-Bus system connection and the
//! Tokio runtime driving the BlueZ GATT client. The `AncsTransport` front end
//! only moves commands across a channel and waits for the acknowledgement, so
//! the port stays `Send + Sync` and the node never touches D-Bus interfaces directly.
//!
//! The §9.4 hard constraints land like this:
//! - "subscribe Data Source before Notification Source" is the *ingress*'s job
//!   (`AncsIngress::connect` orders the two calls); this file performs whatever
//!   it is told, in order.
//! - "never write the Control Point from a value-changed thread": signal listener
//!   callbacks here only forward bytes into [`AncsInbound`]; every write arrives
//!   as a command on the driver thread.
//! - "Control Point writes are serial": the ingress keeps at most one request in
//!   flight and `write_control_point` blocks until the stack reports write confirmation.
//!
//! Runtime against a physically paired iPhone is Manual-required.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::Duration;

use tetherly_core::ancs::{
    ancs_backoff_delay, parse_ble_address, BleAddress, UUID_CONTROL_POINT, UUID_DATA_SOURCE,
    UUID_NOTIFICATION_SOURCE,
};
use tetherly_core::{AncsTransport, CoreError};
use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};
use zbus::export::futures_util::TryStreamExt;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zbus::Connection;

use crate::ancs::{AncsInbound, AncsLink};

/// How long the front end waits for the driver to acknowledge one command. A
/// wedged GATT stack must not park the node's ANCS pump forever; the operation is
/// reported as failed instead of silently pretended.
const ACK_TIMEOUT: Duration = Duration::from_secs(10);

/// The driver checks for stop signals periodically so shutdowns are prompt.
const SERVE_SLICE: Duration = Duration::from_millis(250);

const BLUEZ_SERVICE: &str = "org.bluez";
const DBUS_PROPERTIES_INTERFACE: &str = "org.freedesktop.DBus.Properties";
const DBUS_OBJECT_MANAGER_INTERFACE: &str = "org.freedesktop.DBus.ObjectManager";
const BLUEZ_DEVICE_INTERFACE: &str = "org.bluez.Device1";
const BLUEZ_GATT_CHARACTERISTIC_INTERFACE: &str = "org.bluez.GattCharacteristic1";

// ------------------------------------------------------------- front end

/// A command for the driver thread, with the slot it answers on.
enum Cmd {
    SubscribeData(Sender<Result<(), CoreError>>),
    SubscribeNotification(Sender<Result<(), CoreError>>),
    Write(Vec<u8>, Sender<Result<(), CoreError>>),
    Stop,
}

/// Front end for the driver thread. `Send + Sync` by construction: it holds a
/// `Sender` and nothing else.
pub struct BluezAncsTransport {
    cmd: Sender<Cmd>,
}

impl BluezAncsTransport {
    fn call(
        &self,
        build: impl FnOnce(Sender<Result<(), CoreError>>) -> Cmd,
        dead: CoreError,
    ) -> Result<(), CoreError> {
        let (ack, answer) = channel();
        if self.cmd.send(build(ack)).is_err() {
            return Err(dead);
        }
        match answer.recv_timeout(ACK_TIMEOUT) {
            Ok(result) => result,
            Err(RecvTimeoutError::Timeout) => {
                tracing::warn!("ancs: the BlueZ stack did not answer in time");
                Err(dead)
            }
            Err(RecvTimeoutError::Disconnected) => Err(dead),
        }
    }
}

impl AncsTransport for BluezAncsTransport {
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

impl Drop for BluezAncsTransport {
    fn drop(&mut self) {
        let _ = self.cmd.send(Cmd::Stop);
    }
}

// ------------------------------------------------------------- driver

/// Formats a 48-bit MAC address into a standard BlueZ device path string.
/// E.g. `[0x11, 0x22, 0x33, 0x44, 0x55, 0x66]` -> `"/org/bluez/hci0/dev_11_22_33_44_55_66"`.
pub fn dev_path_from_mac(mac: &[u8; 6]) -> String {
    format!(
        "/org/bluez/hci0/dev_{:02X}_{:02X}_{:02X}_{:02X}_{:02X}_{:02X}",
        mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]
    )
}

/// Resolves characteristic paths for ANCS service.
#[derive(Debug, Clone, Default)]
struct ResolvedAncsPaths {
    device_path: String,
    notification_source: Option<String>,
    data_source: Option<String>,
    control_point: Option<String>,
}

/// Look up managed objects under BlueZ to discover ANCS service and characteristics.
async fn resolve_ancs_paths(
    conn: &Connection,
    target_addr: &BleAddress,
) -> Result<ResolvedAncsPaths, CoreError> {
    let reply = conn
        .call_method(
            Some(BLUEZ_SERVICE),
            "/",
            Some(DBUS_OBJECT_MANAGER_INTERFACE),
            "GetManagedObjects",
            &(),
        )
        .await
        .map_err(|e| {
            tracing::warn!(%e, "bluez: failed to call GetManagedObjects");
            CoreError::AncsFrame
        })?;

    let managed_objects: HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>> =
        reply.body().deserialize().map_err(|e| {
            tracing::warn!(%e, "bluez: failed to deserialize GetManagedObjects response");
            CoreError::AncsFrame
        })?;

    // Determine target device path
    let target_dev_str = match target_addr {
        BleAddress::Mac(mac) => dev_path_from_mac(mac),
        BleAddress::Uuid(uuid) => {
            let canonical = format!(
                "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
                uuid[0], uuid[1], uuid[2], uuid[3], uuid[4], uuid[5], uuid[6], uuid[7],
                uuid[8], uuid[9], uuid[10], uuid[11], uuid[12], uuid[13], uuid[14], uuid[15]
            );
            // Search for matching device by UUID or name
            let mut matched = String::new();
            for (path, ifaces) in &managed_objects {
                if let Some(dev_props) = ifaces.get(BLUEZ_DEVICE_INTERFACE) {
                    if let Some(addr_val) = dev_props.get("Address") {
                        if let Ok(addr_str) = addr_val.downcast_ref::<zbus::zvariant::Str>() {
                            if addr_str.as_str().eq_ignore_ascii_case(&canonical) {
                                matched = path.as_str().to_string();
                                break;
                            }
                        }
                    }
                }
            }
            if matched.is_empty() {
                return Err(CoreError::AncsFrame);
            }
            matched
        }
    };

    let mut resolved = ResolvedAncsPaths {
        device_path: target_dev_str.clone(),
        notification_source: None,
        data_source: None,
        control_point: None,
    };

    // Find characteristics under target device path
    for (path, ifaces) in &managed_objects {
        let path_str = path.as_str();
        if !path_str.starts_with(&target_dev_str) {
            continue;
        }

        if let Some(char_props) = ifaces.get(BLUEZ_GATT_CHARACTERISTIC_INTERFACE) {
            if let Some(uuid_val) = char_props.get("UUID") {
                if let Ok(uuid_str) = uuid_val.downcast_ref::<zbus::zvariant::Str>() {
                    if uuid_str
                        .as_str()
                        .eq_ignore_ascii_case(UUID_NOTIFICATION_SOURCE)
                    {
                        resolved.notification_source = Some(path_str.to_string());
                    } else if uuid_str.as_str().eq_ignore_ascii_case(UUID_DATA_SOURCE) {
                        resolved.data_source = Some(path_str.to_string());
                    } else if uuid_str.as_str().eq_ignore_ascii_case(UUID_CONTROL_POINT) {
                        resolved.control_point = Some(path_str.to_string());
                    }
                }
            }
        }
    }

    if resolved.notification_source.is_some()
        && resolved.data_source.is_some()
        && resolved.control_point.is_some()
    {
        Ok(resolved)
    } else {
        tracing::debug!(?resolved, "bluez: ANCS characteristics not yet resolved");
        Err(CoreError::AncsFrame)
    }
}

/// Extracts a raw byte vector from a D-Bus `ay` (array of bytes) variant property.
fn extract_bytes_from_value(val: &OwnedValue) -> Option<Vec<u8>> {
    if let Value::Array(arr) = &**val {
        let mut bytes = Vec::with_capacity(arr.len());
        for elem in arr.inner() {
            if let Value::U8(b) = elem {
                bytes.push(*b);
            } else {
                return None;
            }
        }
        Some(bytes)
    } else {
        None
    }
}

/// Drives the BlueZ GATT connection and characteristic commands.
async fn run_session(
    conn: &Connection,
    target_addr: &BleAddress,
    inbound_tx: &UnboundedSender<AncsInbound>,
    cmd_rx: &Receiver<Cmd>,
    stopped: &AtomicBool,
) -> Result<(), CoreError> {
    let resolved = resolve_ancs_paths(conn, target_addr).await?;
    tracing::info!(
        device = %resolved.device_path,
        "bluez: ANCS service and characteristics resolved successfully"
    );

    // Notify inbound pump that transport is ready
    let _ = inbound_tx.send(AncsInbound::Ready);

    let notif_path = resolved.notification_source.unwrap();
    let data_path = resolved.data_source.unwrap();
    let cp_path = resolved.control_point.unwrap();

    // Set up D-Bus signal receiver for characteristic value updates
    let mut stream = zbus::MessageStream::from(conn);

    loop {
        if stopped.load(Ordering::Relaxed) {
            break;
        }

        // Drain pending transport commands
        match cmd_rx.recv_timeout(SERVE_SLICE) {
            Ok(Cmd::SubscribeData(ack)) => {
                let res = conn
                    .call_method(
                        Some(BLUEZ_SERVICE),
                        data_path.as_str(),
                        Some(BLUEZ_GATT_CHARACTERISTIC_INTERFACE),
                        "StartNotify",
                        &(),
                    )
                    .await
                    .map(|_| ())
                    .map_err(|e| {
                        tracing::warn!(%e, "bluez: failed to StartNotify on Data Source");
                        CoreError::AncsSubscribe
                    });
                let _ = ack.send(res);
            }
            Ok(Cmd::SubscribeNotification(ack)) => {
                let res = conn
                    .call_method(
                        Some(BLUEZ_SERVICE),
                        notif_path.as_str(),
                        Some(BLUEZ_GATT_CHARACTERISTIC_INTERFACE),
                        "StartNotify",
                        &(),
                    )
                    .await
                    .map(|_| ())
                    .map_err(|e| {
                        tracing::warn!(%e, "bluez: failed to StartNotify on Notification Source");
                        CoreError::AncsSubscribe
                    });
                let _ = ack.send(res);
            }
            Ok(Cmd::Write(bytes, ack)) => {
                let mut options = HashMap::new();
                options.insert("type", Value::from("request"));
                let res = conn
                    .call_method(
                        Some(BLUEZ_SERVICE),
                        cp_path.as_str(),
                        Some(BLUEZ_GATT_CHARACTERISTIC_INTERFACE),
                        "WriteValue",
                        &(bytes, options),
                    )
                    .await
                    .map(|_| ())
                    .map_err(|e| {
                        tracing::warn!(%e, "bluez: failed to WriteValue to Control Point");
                        CoreError::AncsWrite
                    });
                let _ = ack.send(res);
            }
            Ok(Cmd::Stop) => {
                stopped.store(true, Ordering::Relaxed);
                break;
            }
            Err(RecvTimeoutError::Timeout) => {
                // Slice timeout; fall through to poll D-Bus message stream
            }
            Err(RecvTimeoutError::Disconnected) => {
                stopped.store(true, Ordering::Relaxed);
                break;
            }
        }

        // Poll any received D-Bus signals non-blockingly
        while let Ok(Ok(Some(msg))) =
            tokio::time::timeout(Duration::from_millis(5), stream.try_next()).await
        {
            let header = msg.header();
            if header.interface().map(|i| i.as_str()) == Some(DBUS_PROPERTIES_INTERFACE)
                && header.member().map(|m| m.as_str()) == Some("PropertiesChanged")
            {
                let path = header.path().map(|p| p.as_str()).unwrap_or("");
                if let Ok((iface, changed, _invalidated)) =
                    msg.body()
                        .deserialize::<(String, HashMap<String, OwnedValue>, Vec<String>)>()
                {
                    if iface == BLUEZ_DEVICE_INTERFACE && path == resolved.device_path {
                        if let Some(conn_val) = changed.get("Connected") {
                            if let Ok(is_connected) = conn_val.downcast_ref::<bool>() {
                                if !is_connected {
                                    tracing::warn!("bluez: peripheral disconnected");
                                    let _ = inbound_tx.send(AncsInbound::Disconnected);
                                    return Err(CoreError::AncsFrame);
                                }
                            }
                        }
                    } else if iface == BLUEZ_GATT_CHARACTERISTIC_INTERFACE {
                        if let Some(val) = changed.get("Value") {
                            if let Some(bytes) = extract_bytes_from_value(val) {
                                if path == notif_path {
                                    let _ = inbound_tx.send(AncsInbound::NotificationSource(bytes));
                                } else if path == data_path {
                                    let _ = inbound_tx.send(AncsInbound::DataSource(bytes));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Spawns the BlueZ driver thread with its own single-threaded Tokio runtime.
pub fn open(address: &str) -> Result<AncsLink, CoreError> {
    let ble_addr = parse_ble_address(address)?;
    let (cmd_tx, cmd_rx) = channel();
    let (inbound_tx, inbound_rx) = unbounded_channel();
    let stopped = Arc::new(AtomicBool::new(false));
    let stopped_for_thread = stopped.clone();

    std::thread::Builder::new()
        .name("tetherly-ancs-bluez".into())
        .spawn(move || {
            let rt = match tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            {
                Ok(rt) => rt,
                Err(e) => {
                    tracing::error!(%e, "bluez: failed to create driver runtime");
                    return;
                }
            };

            rt.block_on(async move {
                let mut attempt = 0;
                while !stopped_for_thread.load(Ordering::Relaxed) {
                    tracing::debug!(attempt, "bluez: connecting to system D-Bus");
                    let conn = match Connection::system().await {
                        Ok(c) => c,
                        Err(e) => {
                            tracing::warn!(%e, "bluez: cannot connect to system D-Bus");
                            let delay = ancs_backoff_delay(attempt);
                            tokio::time::sleep(Duration::from_millis(delay)).await;
                            attempt = attempt.saturating_add(1);
                            continue;
                        }
                    };

                    match run_session(&conn, &ble_addr, &inbound_tx, &cmd_rx, &stopped_for_thread)
                        .await
                    {
                        Ok(()) => {
                            attempt = 0;
                        }
                        Err(e) => {
                            tracing::warn!(%e, "bluez: session exited, backing off");
                            let delay = ancs_backoff_delay(attempt);
                            tokio::time::sleep(Duration::from_millis(delay)).await;
                            attempt = attempt.saturating_add(1);
                        }
                    }
                }
            });
        })
        .map_err(|_| CoreError::AncsFrame)?;

    let transport = Arc::new(BluezAncsTransport { cmd: cmd_tx });
    Ok((transport, inbound_rx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_address_formats_to_bluez_device_path() {
        let mac = [0x11, 0x22, 0x33, 0x44, 0x55, 0x66];
        let path = dev_path_from_mac(&mac);
        assert_eq!(path, "/org/bluez/hci0/dev_11_22_33_44_55_66");
    }

    #[test]
    fn transport_command_channel_handles_disconnect() {
        let (tx, rx) = channel();
        let transport = BluezAncsTransport { cmd: tx };
        drop(rx);
        assert!(matches!(
            transport.subscribe_data_source(),
            Err(CoreError::AncsSubscribe)
        ));
        assert!(matches!(
            transport.subscribe_notification_source(),
            Err(CoreError::AncsSubscribe)
        ));
        assert!(matches!(
            transport.write_control_point(b"test"),
            Err(CoreError::AncsWrite)
        ));
    }

    #[test]
    fn transport_command_acknowledgement() {
        let (tx, rx) = channel();
        let transport = BluezAncsTransport { cmd: tx };

        // Test subscribe_data_source ACK
        std::thread::spawn(move || match rx.recv() {
            Ok(Cmd::SubscribeData(ack)) => {
                let _ = ack.send(Ok(()));
            }
            _ => panic!("unexpected command"),
        });
        assert!(transport.subscribe_data_source().is_ok());

        // Test subscribe_notification_source ACK
        let (tx2, rx2) = channel();
        let transport2 = BluezAncsTransport { cmd: tx2 };
        std::thread::spawn(move || match rx2.recv() {
            Ok(Cmd::SubscribeNotification(ack)) => {
                let _ = ack.send(Ok(()));
            }
            _ => panic!("unexpected command"),
        });
        assert!(transport2.subscribe_notification_source().is_ok());

        // Test write_control_point ACK
        let (tx3, rx3) = channel();
        let transport3 = BluezAncsTransport { cmd: tx3 };
        std::thread::spawn(move || match rx3.recv() {
            Ok(Cmd::Write(bytes, ack)) => {
                assert_eq!(bytes, b"command");
                let _ = ack.send(Ok(()));
            }
            _ => panic!("unexpected command"),
        });
        assert!(transport3.write_control_point(b"command").is_ok());
    }

    #[test]
    fn extract_bytes_from_value_handles_byte_array() {
        use zbus::zvariant::Array;

        // Valid byte array
        let raw = vec![0x10u8, 0x20, 0x30, 0x40];
        let mut arr = Array::new(zbus::zvariant::Signature::from_static_str_unchecked("y"));
        for b in &raw {
            arr.append(Value::U8(*b)).unwrap();
        }
        let owned_val = OwnedValue::try_from(Value::Array(arr)).unwrap();
        let extracted = extract_bytes_from_value(&owned_val);
        assert_eq!(extracted, Some(raw));

        // Non-array value
        let non_arr = OwnedValue::from(true);
        assert_eq!(extract_bytes_from_value(&non_arr), None);
    }

    #[test]
    fn open_rejects_invalid_ble_address() {
        let res = open("not:a:valid:address");
        assert!(matches!(res, Err(CoreError::AncsFrame)));
    }
}
