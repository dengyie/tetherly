// SPDX-License-Identifier: Apache-2.0 OR MIT
//! macOS ANCS over CoreBluetooth. Self-contained: it imports only
//! absolute paths, the shape `screen/macos.rs` and `ancs/win_gatt.rs` use.
//!
//! One dedicated thread owns the CoreBluetooth manager, the serial dispatch
//! queue, and the delegate callbacks. The `AncsTransport` front end only moves
//! commands across a channel and waits for the acknowledgement, so the port
//! stays `Send + Sync` and the node never touches an Objective-C interface directly.
//!
//! The §9.4 hard constraints land like this:
//! - "subscribe Data Source before Notification Source" is the *ingress*'s job
//!   (`AncsIngress::connect` orders the two calls); this file performs whatever
//!   it is told, in order.
//! - "never write the Control Point from a value-changed thread": the callbacks
//!   here only forward bytes into [`AncsInbound`]; every write arrives as a
//!   command on the driver thread.
//! - "Control Point writes are serial": the ingress keeps at most one request in
//!   flight and `write_control_point` blocks until the stack reports write confirmation.
//!
//! Runtime against a physically paired iPhone is Manual-required.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use dispatch2::{DispatchQueue, DispatchQueueAttr};
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, AnyThread, DefinedClass};
use objc2_core_bluetooth::{
    CBCentralManager, CBCentralManagerDelegate, CBCharacteristic, CBCharacteristicWriteType,
    CBManagerState, CBPeripheral, CBPeripheralDelegate, CBPeripheralState, CBService, CBUUID,
};
use objc2_foundation::{NSArray, NSData, NSError, NSString, NSUUID};
use tetherly_core::ancs::{
    ancs_backoff_delay, BleAddress, UUID_ANCS_SERVICE, UUID_CONTROL_POINT, UUID_DATA_SOURCE,
    UUID_NOTIFICATION_SOURCE,
};
use tetherly_core::{AncsTransport, CoreError};
use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use crate::ancs::AncsInbound;

/// How long the front end waits for the driver to acknowledge one command. A
/// wedged GATT stack must not park the node's ANCS pump forever; the operation is
/// reported as failed instead of silently pretended.
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
pub struct MacAncsTransport {
    cmd: Sender<Cmd>,
}

impl MacAncsTransport {
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
                tracing::warn!("ancs: the CoreBluetooth stack did not answer in time");
                Err(dead)
            }
            Err(RecvTimeoutError::Disconnected) => Err(dead),
        }
    }
}

impl AncsTransport for MacAncsTransport {
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

impl Drop for MacAncsTransport {
    fn drop(&mut self) {
        let _ = self.cmd.send(Cmd::Stop);
    }
}

/// Spawn the driver thread and hand back the transport plus its inbound channel.
pub fn open(
    address: &str,
) -> Result<(Arc<dyn AncsTransport>, UnboundedReceiver<AncsInbound>), CoreError> {
    let ble_addr = tetherly_core::ancs::parse_ble_address(address)?;
    let (cmd, cmd_rx) = channel();
    let (in_tx, in_rx) = unbounded_channel();
    std::thread::Builder::new()
        .name("tetherly-ancs-cb".into())
        .spawn(move || driver(ble_addr, in_tx, cmd_rx))
        .map_err(|_| CoreError::AncsFrame)?;
    Ok((Arc::new(MacAncsTransport { cmd }), in_rx))
}

// --------------------------------------------------------------- delegate

struct SharedState {
    out: Mutex<Option<UnboundedSender<AncsInbound>>>,
    manager_state: Mutex<CBManagerState>,
    state_cond: Condvar,
    connected_peripheral: Mutex<Option<Retained<CBPeripheral>>>,
    connect_failed: AtomicBool,
    services_discovered: AtomicBool,
    chars_discovered: AtomicBool,
    notification_source: Mutex<Option<Retained<CBCharacteristic>>>,
    data_source: Mutex<Option<Retained<CBCharacteristic>>>,
    control_point: Mutex<Option<Retained<CBCharacteristic>>>,
    disconnected: AtomicBool,
    write_ack: Mutex<Option<Sender<Result<(), CoreError>>>>,
    notif_ack: Mutex<Option<Sender<Result<(), CoreError>>>>,
    data_ack: Mutex<Option<Sender<Result<(), CoreError>>>>,
}

impl Default for SharedState {
    fn default() -> Self {
        Self {
            out: Mutex::new(None),
            manager_state: Mutex::new(CBManagerState::Unknown),
            state_cond: Condvar::new(),
            connected_peripheral: Mutex::new(None),
            connect_failed: AtomicBool::new(false),
            services_discovered: AtomicBool::new(false),
            chars_discovered: AtomicBool::new(false),
            notification_source: Mutex::new(None),
            data_source: Mutex::new(None),
            control_point: Mutex::new(None),
            disconnected: AtomicBool::new(false),
            write_ack: Mutex::new(None),
            notif_ack: Mutex::new(None),
            data_ack: Mutex::new(None),
        }
    }
}

// SAFETY: All mutating fields are protected by Mutex or AtomicBool. CoreBluetooth
// objects are referenced by serial queue and accessed across the driver thread.
unsafe impl Send for SharedState {}
unsafe impl Sync for SharedState {}

define_class!(
    #[unsafe(super = NSObject)]
    #[name = "TetherlyAncsDelegate"]
    #[ivars = Arc<SharedState>]
    struct AncsDelegate;

    unsafe impl NSObjectProtocol for AncsDelegate {}

    unsafe impl CBCentralManagerDelegate for AncsDelegate {
        #[unsafe(method(centralManagerDidUpdateState:))]
        fn did_update_state(&self, central: &CBCentralManager) {
            let state = unsafe { central.state() };
            let mut guard = self.ivars().manager_state.lock().unwrap();
            *guard = state;
            self.ivars().state_cond.notify_all();
        }

        #[unsafe(method(centralManager:didConnectPeripheral:))]
        fn did_connect_peripheral(
            &self,
            _central: &CBCentralManager,
            peripheral: &CBPeripheral,
        ) {
            let mut guard = self.ivars().connected_peripheral.lock().unwrap();
            *guard = Some(Retained::from(peripheral));
            self.ivars().disconnected.store(false, Ordering::Release);
            self.ivars().state_cond.notify_all();
        }

        #[unsafe(method(centralManager:didFailToConnectPeripheral:error:))]
        fn did_fail_to_connect_peripheral(
            &self,
            _central: &CBCentralManager,
            _peripheral: &CBPeripheral,
            error: Option<&NSError>,
        ) {
            if let Some(err) = error {
                tracing::warn!(%err, "ancs: failed to connect peripheral");
            }
            self.ivars().connect_failed.store(true, Ordering::Release);
            self.ivars().state_cond.notify_all();
        }

        #[unsafe(method(centralManager:didDisconnectPeripheral:error:))]
        fn did_disconnect_peripheral(
            &self,
            _central: &CBCentralManager,
            _peripheral: &CBPeripheral,
            error: Option<&NSError>,
        ) {
            if let Some(err) = error {
                tracing::warn!(%err, "ancs: peripheral disconnected with error");
            }
            self.ivars().disconnected.store(true, Ordering::Release);
            if let Some(ref out) = *self.ivars().out.lock().unwrap() {
                let _ = out.send(AncsInbound::Disconnected);
            }
            self.ivars().state_cond.notify_all();
        }
    }

    unsafe impl CBPeripheralDelegate for AncsDelegate {
        #[unsafe(method(peripheral:didDiscoverServices:))]
        fn did_discover_services(
            &self,
            _peripheral: &CBPeripheral,
            error: Option<&NSError>,
        ) {
            if let Some(err) = error {
                tracing::warn!(%err, "ancs: error discovering services");
            }
            self.ivars().services_discovered.store(true, Ordering::Release);
            self.ivars().state_cond.notify_all();
        }

        #[unsafe(method(peripheral:didDiscoverCharacteristicsForService:error:))]
        fn did_discover_characteristics(
            &self,
            _peripheral: &CBPeripheral,
            service: &CBService,
            error: Option<&NSError>,
        ) {
            if let Some(err) = error {
                tracing::warn!(%err, "ancs: error discovering characteristics");
            }
            if let Some(chars) = unsafe { service.characteristics() } {
                for ch in chars.iter() {
                    let uuid_str = unsafe { ch.UUID().UUIDString() }.to_string();
                    if uuid_str.eq_ignore_ascii_case(UUID_NOTIFICATION_SOURCE) {
                        *self.ivars().notification_source.lock().unwrap() = Some(ch.clone());
                    } else if uuid_str.eq_ignore_ascii_case(UUID_DATA_SOURCE) {
                        *self.ivars().data_source.lock().unwrap() = Some(ch.clone());
                    } else if uuid_str.eq_ignore_ascii_case(UUID_CONTROL_POINT) {
                        *self.ivars().control_point.lock().unwrap() = Some(ch.clone());
                    }
                }
            }
            self.ivars().chars_discovered.store(true, Ordering::Release);
            self.ivars().state_cond.notify_all();
        }

        #[unsafe(method(peripheral:didUpdateValueForCharacteristic:error:))]
        fn did_update_value(
            &self,
            _peripheral: &CBPeripheral,
            characteristic: &CBCharacteristic,
            error: Option<&NSError>,
        ) {
            if let Some(err) = error {
                tracing::warn!(%err, "ancs: didUpdateValue error");
                return;
            }
            let uuid_str = unsafe { characteristic.UUID().UUIDString() }.to_string();
            if let Some(data) = unsafe { characteristic.value() } {
                let bytes = data.to_vec();
                if let Some(ref out) = *self.ivars().out.lock().unwrap() {
                    if uuid_str.eq_ignore_ascii_case(UUID_NOTIFICATION_SOURCE) {
                        let _ = out.send(AncsInbound::NotificationSource(bytes));
                    } else if uuid_str.eq_ignore_ascii_case(UUID_DATA_SOURCE) {
                        let _ = out.send(AncsInbound::DataSource(bytes));
                    }
                }
            }
        }

        #[unsafe(method(peripheral:didWriteValueForCharacteristic:error:))]
        fn did_write_value(
            &self,
            _peripheral: &CBPeripheral,
            _characteristic: &CBCharacteristic,
            error: Option<&NSError>,
        ) {
            let res = match error {
                Some(err) => {
                    tracing::warn!(%err, "ancs: control point write failed");
                    Err(CoreError::AncsWrite)
                }
                None => Ok(()),
            };
            if let Some(ack) = self.ivars().write_ack.lock().unwrap().take() {
                let _ = ack.send(res);
            }
        }

        #[unsafe(method(peripheral:didUpdateNotificationStateForCharacteristic:error:))]
        fn did_update_notification_state(
            &self,
            _peripheral: &CBPeripheral,
            characteristic: &CBCharacteristic,
            error: Option<&NSError>,
        ) {
            let uuid_str = unsafe { characteristic.UUID().UUIDString() }.to_string();
            let res = match error {
                Some(err) => {
                    tracing::warn!(%err, "ancs: subscription update failed");
                    Err(CoreError::AncsSubscribe)
                }
                None => Ok(()),
            };
            if uuid_str.eq_ignore_ascii_case(UUID_NOTIFICATION_SOURCE) {
                if let Some(ack) = self.ivars().notif_ack.lock().unwrap().take() {
                    let _ = ack.send(res);
                }
            } else if uuid_str.eq_ignore_ascii_case(UUID_DATA_SOURCE) {
                if let Some(ack) = self.ivars().data_ack.lock().unwrap().take() {
                    let _ = ack.send(res);
                }
            }
        }
    }
);

impl AncsDelegate {
    fn new(shared: Arc<SharedState>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(shared);
        unsafe { msg_send![super(this), init] }
    }
}

// --------------------------------------------------------------- driver

/// Owns the CoreBluetooth objects for the life of one process: establish a session,
/// serve commands until the phone goes away, then back off and try again.
fn driver(address: BleAddress, out: UnboundedSender<AncsInbound>, cmd: Receiver<Cmd>) {
    let canonical_uuid = match address {
        BleAddress::Uuid(_) => address.to_canonical_string(),
        BleAddress::Mac(_) => {
            tracing::error!(
                "ancs: macOS CoreBluetooth requires 128-bit UUID, not 48-bit MAC address"
            );
            return;
        }
    };

    let mut attempt = 0u32;
    loop {
        match Session::establish(&canonical_uuid, &out) {
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

/// One established GATT session.
struct Session {
    central: Retained<CBCentralManager>,
    peripheral: Retained<CBPeripheral>,
    notification_source: Retained<CBCharacteristic>,
    data_source: Retained<CBCharacteristic>,
    control_point: Retained<CBCharacteristic>,
    shared: Arc<SharedState>,
}

impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            self.central.cancelPeripheralConnection(&self.peripheral);
        }
    }
}

impl Session {
    fn establish(canonical_uuid: &str, out: &UnboundedSender<AncsInbound>) -> Result<Self, String> {
        let shared = Arc::new(SharedState::default());
        *shared.out.lock().unwrap() = Some(out.clone());

        let queue = DispatchQueue::new("tetherly.ancs.queue", DispatchQueueAttr::SERIAL);
        let delegate = AncsDelegate::new(shared.clone());
        let central = unsafe {
            CBCentralManager::initWithDelegate_queue(
                CBCentralManager::alloc(),
                Some(ProtocolObject::from_ref(&*delegate)),
                Some(&queue),
            )
        };

        // 1. Wait for Bluetooth to power on
        let start = std::time::Instant::now();
        let timeout = Duration::from_secs(5);
        loop {
            let state = *shared.manager_state.lock().unwrap();
            if state == CBManagerState::PoweredOn {
                break;
            }
            if state == CBManagerState::Unauthorized {
                return Err("Bluetooth permission unauthorized".into());
            }
            if state == CBManagerState::Unsupported {
                return Err("Bluetooth unsupported on this host".into());
            }
            if start.elapsed() >= timeout {
                return Err(format!(
                    "Bluetooth not powered on in time (state {state:?})"
                ));
            }
            let guard = shared.manager_state.lock().unwrap();
            let _ = shared
                .state_cond
                .wait_timeout(guard, Duration::from_millis(100));
        }

        // 2. Locate peripheral by NSUUID
        let nsuuid =
            NSUUID::initWithUUIDString(NSUUID::alloc(), &NSString::from_str(canonical_uuid))
                .ok_or_else(|| format!("malformed UUID {canonical_uuid}"))?;

        let retrieved = unsafe {
            central.retrievePeripheralsWithIdentifiers(&NSArray::from_retained_slice(&[nsuuid]))
        };
        let peripheral = if let Some(p) = retrieved.firstObject() {
            p
        } else {
            // Check connected peripherals with ANCS service
            let ancs_uuid =
                unsafe { CBUUID::UUIDWithString(&NSString::from_str(UUID_ANCS_SERVICE)) };
            let connected = unsafe {
                central.retrieveConnectedPeripheralsWithServices(&NSArray::from_retained_slice(&[
                    ancs_uuid,
                ]))
            };
            connected
                .firstObject()
                .ok_or_else(|| format!("peripheral {canonical_uuid} not found or not connected"))?
        };

        // 3. Connect peripheral
        unsafe {
            central.connectPeripheral_options(&peripheral, None);
        }

        let start = std::time::Instant::now();
        loop {
            if shared.connected_peripheral.lock().unwrap().is_some() {
                break;
            }
            if shared.connect_failed.load(Ordering::Acquire) {
                return Err("failed to connect peripheral".into());
            }
            if start.elapsed() >= timeout {
                return Err("connection timed out".into());
            }
            let guard = shared.connected_peripheral.lock().unwrap();
            let _ = shared
                .state_cond
                .wait_timeout(guard, Duration::from_millis(100));
        }

        // 4. Discover ANCS service and characteristics
        unsafe {
            peripheral.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            let ancs_uuid = CBUUID::UUIDWithString(&NSString::from_str(UUID_ANCS_SERVICE));
            peripheral.discoverServices(Some(&NSArray::from_retained_slice(&[ancs_uuid])));
        }

        let start = std::time::Instant::now();
        loop {
            if shared.services_discovered.load(Ordering::Acquire) {
                break;
            }
            if start.elapsed() >= timeout {
                return Err("service discovery timed out".into());
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        let ancs_service = unsafe {
            let services = peripheral
                .services()
                .ok_or_else(|| "no services found on peripheral".to_string())?;
            let mut target = None;
            for svc in services.iter() {
                let uuid_str = svc.UUID().UUIDString().to_string();
                if uuid_str.eq_ignore_ascii_case(UUID_ANCS_SERVICE) {
                    target = Some(svc.clone());
                    break;
                }
            }
            target.ok_or_else(|| "ANCS service not found on peripheral".to_string())?
        };

        unsafe {
            peripheral.discoverCharacteristics_forService(None, &ancs_service);
        }

        let start = std::time::Instant::now();
        loop {
            if shared.chars_discovered.load(Ordering::Acquire) {
                break;
            }
            if start.elapsed() >= timeout {
                return Err("characteristic discovery timed out".into());
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        let notification_source = shared
            .notification_source
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| "ANCS Notification Source characteristic missing".to_string())?;
        let data_source = shared
            .data_source
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| "ANCS Data Source characteristic missing".to_string())?;
        let control_point = shared
            .control_point
            .lock()
            .unwrap()
            .clone()
            .ok_or_else(|| "ANCS Control Point characteristic missing".to_string())?;

        Ok(Self {
            central,
            peripheral,
            notification_source,
            data_source,
            control_point,
            shared,
        })
    }

    /// Serve commands from the node until the peripheral disconnects or a Stop
    /// command arrives. Returns true to trigger a reconnect, false on clean exit.
    fn serve(&self, cmd: &Receiver<Cmd>) -> bool {
        loop {
            if self.shared.disconnected.load(Ordering::Acquire) {
                return true;
            }
            let state = unsafe { self.peripheral.state() };
            if state != CBPeripheralState::Connected {
                return true;
            }

            match cmd.recv_timeout(SERVE_SLICE) {
                Ok(Cmd::SubscribeData(ack)) => {
                    *self.shared.data_ack.lock().unwrap() = Some(ack);
                    unsafe {
                        self.peripheral
                            .setNotifyValue_forCharacteristic(true, &self.data_source);
                    }
                }
                Ok(Cmd::SubscribeNotification(ack)) => {
                    *self.shared.notif_ack.lock().unwrap() = Some(ack);
                    unsafe {
                        self.peripheral
                            .setNotifyValue_forCharacteristic(true, &self.notification_source);
                    }
                }
                Ok(Cmd::Write(bytes, ack)) => {
                    *self.shared.write_ack.lock().unwrap() = Some(ack);
                    let data = NSData::with_bytes(&bytes);
                    unsafe {
                        self.peripheral.writeValue_forCharacteristic_type(
                            &data,
                            &self.control_point,
                            CBCharacteristicWriteType::WithResponse,
                        );
                    }
                }
                Ok(Cmd::Stop) => {
                    return false;
                }
                Err(RecvTimeoutError::Timeout) => {
                    // Normal timeout for polling SERVE_SLICE.
                }
                Err(RecvTimeoutError::Disconnected) => {
                    // Front end dropped.
                    return false;
                }
            }
        }
    }
}
