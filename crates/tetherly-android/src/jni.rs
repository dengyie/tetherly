// SPDX-License-Identifier: Apache-2.0 OR MIT
//! JNI surface for `app.tetherly.NativeBridge`. Thin: parse Java strings, call
//! the client, store the handle in a process-wide slot. No crypto or wire logic
//! lives here. Companion to `NativeBridge.kt`.
//!
//! `#[no_mangle]` is the one place this crate exempts itself from the workspace
//! `unsafe_code` lint; the JNI ABI requires a fixed symbol name.

#![allow(unsafe_code)]

use std::net::SocketAddr;
use std::sync::Mutex;

use jni::objects::{JByteArray, JClass, JString};
use jni::sys::{jint, jlong};
use jni::JNIEnv;
use tetherly_core::NotifyPush;

use crate::client::{AndroidClient, SessionHandle};

/// The one live session. `nativeAttach` replaces it; `nativeStop` clears it.
static SESSION: Mutex<Option<SessionHandle>> = Mutex::new(None);
/// Local (device_id, id_pk) for UI display. Set by `nativeAttach`.
static LOCAL_ID: Mutex<Option<(String, [u8; 32])>> = Mutex::new(None);

fn read(env: &mut JNIEnv<'_>, s: &JString<'_>) -> String {
    env.get_string(s).map(|j| j.into()).unwrap_or_default()
}

fn attach_inner(
    data_dir: &str,
    host: &str,
    port: u16,
    pin: &str,
) -> Result<(String, [u8; 32]), String> {
    let addr: SocketAddr = format!("{host}:{port}")
        .parse()
        .map_err(|e| format!("bad address {host}:{port}: {e}"))?;
    let pin_digits = if pin.is_empty() {
        None
    } else {
        Some(tetherly_core::pin::parse_pin(pin).map_err(|e| format!("bad PIN: {e}"))?)
    };
    let client = AndroidClient::load_or_create(data_dir, "Android").map_err(|e| e.to_string())?;
    let device_id = client.device_id().as_str().to_string();
    let id_pk = client.id_pk();
    let handle = client.start(addr, pin_digits, false);
    {
        let mut slot = SESSION.lock().map_err(|_| "session lock poisoned")?;
        if let Some(old) = slot.take() {
            old.stop();
        }
        *slot = Some(handle);
    }
    {
        let mut id = LOCAL_ID.lock().map_err(|_| "id lock poisoned")?;
        *id = Some((device_id.clone(), id_pk));
    }
    Ok((device_id, id_pk))
}

/// Start (or restart) the session. Returns an empty string on success, else a
/// human-readable error for the UI. Never returns secret material.
#[no_mangle]
pub extern "system" fn Java_app_tetherly_NativeBridge_nativeAttach<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    data_dir: JString<'local>,
    host: JString<'local>,
    port: jint,
    pin: JString<'local>,
) -> JString<'local> {
    let data_dir = read(&mut env, &data_dir);
    let host = read(&mut env, &host);
    let pin = read(&mut env, &pin);
    let port = u16::try_from(port).unwrap_or(45717);
    let msg = match attach_inner(&data_dir, &host, port, &pin) {
        Ok(_) => String::new(),
        Err(e) => e,
    };
    env.new_string(msg).unwrap_or_default()
}

/// Queue one notification. Ignored when no session is connected.
#[no_mangle]
pub extern "system" fn Java_app_tetherly_NativeBridge_nativeSendNotify<'local>(
    mut env: JNIEnv<'local>,
    _class: JClass<'local>,
    uid: JString<'local>,
    app_id: JString<'local>,
    app_name: JString<'local>,
    title: JString<'local>,
    body: JString<'local>,
    ts: jlong,
) {
    let push = NotifyPush {
        uid: read(&mut env, &uid),
        app_id: read(&mut env, &app_id),
        app_name: read(&mut env, &app_name),
        title: read(&mut env, &title),
        body: read(&mut env, &body),
        ts: ts.max(0) as u64,
        actions: vec!["copy".into(), "dismiss".into()],
    };
    if let Ok(slot) = SESSION.lock() {
        if let Some(handle) = slot.as_ref() {
            let _ = handle.notify(push);
        }
    }
}

/// Local device_id, or empty before `nativeAttach`.
#[no_mangle]
pub extern "system" fn Java_app_tetherly_NativeBridge_nativeDeviceId<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> JString<'local> {
    let id = LOCAL_ID
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|(d, _)| d.clone()))
        .unwrap_or_default();
    env.new_string(id).unwrap_or_default()
}

/// Local Ed25519 public key (32 bytes), or an empty array before `nativeAttach`.
#[no_mangle]
pub extern "system" fn Java_app_tetherly_NativeBridge_nativeIdPk<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> JByteArray<'local> {
    let pk = LOCAL_ID
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|(_, pk)| pk.to_vec()))
        .unwrap_or_default();
    env.byte_array_from_slice(&pk).unwrap_or_default()
}

/// Connection state code (0 idle, 1 connecting, 2 connected, 3 error).
#[no_mangle]
pub extern "system" fn Java_app_tetherly_NativeBridge_nativeState(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
) -> jint {
    SESSION
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|h| h.state_code()))
        .unwrap_or(crate::client::STATE_IDLE)
}

/// Last connection error, or empty. Never contains secret material.
#[no_mangle]
pub extern "system" fn Java_app_tetherly_NativeBridge_nativeLastError<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> JString<'local> {
    let e = SESSION
        .lock()
        .ok()
        .and_then(|g| g.as_ref().and_then(|h| h.last_error()))
        .unwrap_or_default();
    env.new_string(e).unwrap_or_default()
}

/// Stop the session and clear the handle.
#[no_mangle]
pub extern "system" fn Java_app_tetherly_NativeBridge_nativeStop(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
) {
    if let Ok(mut slot) = SESSION.lock() {
        if let Some(handle) = slot.take() {
            handle.stop();
        }
    }
}

/// Drain quick replies typed on the desktop since the last call, as a JSON
/// array of `{"uid","app_id","text"}`. Empty string when there are none. The
/// text is user content the phone needs in order to fire the reply action; the
/// caller must not write it to logs. Never returns secret material.
#[no_mangle]
pub extern "system" fn Java_app_tetherly_NativeBridge_nativeTakeReplies<'local>(
    env: JNIEnv<'local>,
    _class: JClass<'local>,
) -> JString<'local> {
    let replies = SESSION
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|h| h.take_replies()))
        .unwrap_or_default();
    let json = if replies.is_empty() {
        String::new()
    } else {
        serde_json::to_string(&replies).unwrap_or_default()
    };
    env.new_string(json).unwrap_or_default()
}
