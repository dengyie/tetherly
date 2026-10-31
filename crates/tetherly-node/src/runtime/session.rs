// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Session accept path and the low-level frame send helper.

use super::*;
use crate::store::remember_route;
use std::net::SocketAddr;
use tetherly_core::frame::InnerFrame;
use tetherly_core::{CoreError, DeviceId};
use tetherly_net::session::accept_session;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tracing::warn;

impl Node {
    pub async fn send_to(&self, peer: &DeviceId, frame: InnerFrame) -> Result<(), CoreError> {
        let tx = {
            let map = self.shared.sessions.lock().await;
            map.get(peer).cloned()
        };
        match tx {
            Some(tx) => match tx.try_send(frame) {
                Ok(()) => Ok(()),
                Err(mpsc::error::TrySendError::Full(_)) => {
                    Err(CoreError::Json("peer busy; send queue full".into()))
                }
                Err(mpsc::error::TrySendError::Closed(_)) => {
                    Err(CoreError::Json("peer gone".into()))
                }
            },
            None => Err(CoreError::Json("not connected".into())),
        }
    }

    pub(super) async fn accept_one(
        &self,
        stream: TcpStream,
        addr: SocketAddr,
    ) -> Result<(), tetherly_net::NetError> {
        let cfg = self.session_config();
        let sess = match accept_session(stream, addr, &cfg).await {
            Ok(s) => s,
            Err(e) => {
                self.shared.telemetry.record_handshake_failed();
                return Err(e);
            }
        };
        if self.is_blocked(sess.peer_id.as_str()) {
            warn!(peer = %sess.peer_id, "incoming session rejected: peer is blocked");
            self.shared.telemetry.record_handshake_failed();
            return Err(CoreError::PeerBlocked(sess.peer_id.to_string()).into());
        }
        self.persist_trust();
        let _ = remember_route(
            &self.shared.cfg.data_dir,
            sess.peer_id.as_str(),
            &addr.to_string(),
        );
        self.attach(sess).await;
        Ok(())
    }
}
