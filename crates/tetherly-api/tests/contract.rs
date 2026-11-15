// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Wire-shape tests for the v1 contract (ADR-139 §4.1).

use tetherly_api::{ApiError, ErrorBody, ErrorCode, Health, API_VERSION};

#[test]
fn error_envelope_wire_shape() {
    let body = ErrorBody::new(ErrorCode::NotFound, "no such device");
    let v = serde_json::to_value(&body).unwrap();
    assert_eq!(
        v,
        serde_json::json!({"error": {"code": "not_found", "message": "no such device"}})
    );
    let back: ErrorBody = serde_json::from_value(v).unwrap();
    assert_eq!(
        back.error,
        ApiError::new(ErrorCode::NotFound, "no such device")
    );
}

#[test]
fn every_error_code_maps_to_a_client_or_server_status() {
    use ErrorCode::*;
    let cases = [
        (BadRequest, 400),
        (Unauthorized, 401),
        (Forbidden, 403),
        (NotFound, 404),
        (MethodNotAllowed, 405),
        (Conflict, 409),
        (NeedsInteractiveSession, 409),
        (PayloadTooLarge, 413),
        (Internal, 500),
        (Unavailable, 503),
    ];
    assert_eq!(cases.len(), ErrorCode::ALL.len(), "table covers every code");
    for (code, status) in cases {
        assert_eq!(code.http_status(), status, "{code:?}");
    }
}

#[test]
fn needs_interactive_session_serializes_snake_case() {
    assert_eq!(
        serde_json::to_string(&ErrorCode::NeedsInteractiveSession).unwrap(),
        "\"needs_interactive_session\""
    );
}

#[test]
fn health_carries_api_major() {
    let h = Health::new("0.2.56");
    assert!(h.ok);
    assert_eq!(h.api, API_VERSION);
    assert_eq!(API_VERSION, 1);
}

mod events {
    use tetherly_api::*;

    fn device() -> Device {
        Device {
            id: "tdev_a".into(),
            name: "mac".into(),
            alias: None,
            platform: Some("macos".into()),
            device_type: "desktop".into(),
            online: true,
            trusted: true,
            blocked: false,
            path: Some(LinkPath::Lan),
            addr: Some("192.168.1.7:45717".into()),
            rtt_ms: Some(3),
            last_seen_ms: Some(1),
            caps: vec![],
            features: vec![],
            tags: vec![],
        }
    }

    #[test]
    fn envelope_flattens_seq_and_type_tag() {
        let env = EventEnvelope {
            seq: 7,
            event: Event::DeviceUpdated { device: device() },
        };
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["seq"], 7);
        assert_eq!(v["type"], "device.updated");
        assert_eq!(v["device"]["path"], "lan");
        let back: EventEnvelope = serde_json::from_value(v).unwrap();
        assert_eq!(back, env);
    }

    #[test]
    fn event_names_match_wire_tags_and_topics() {
        let cases = [
            (
                Event::DeviceUpdated { device: device() },
                "device.updated",
                Some(Topic::Devices),
            ),
            (
                Event::DeviceRemoved { id: "x".into() },
                "device.removed",
                Some(Topic::Devices),
            ),
            (
                Event::PairingChanged {
                    pairing: Pairing {
                        pin_active: true,
                        pin_expires_in_secs: 30,
                    },
                },
                "pairing.changed",
                Some(Topic::Pairing),
            ),
            (
                Event::InputChanged {
                    input: Input {
                        seat: "local".into(),
                        channel_open: false,
                        peer: None,
                    },
                },
                "input.changed",
                Some(Topic::Input),
            ),
            (
                Event::Invalidated {
                    topic: Topic::Layout,
                },
                "invalidated",
                Some(Topic::Layout),
            ),
            (
                Event::TransfersChanged {
                    transfers: Transfers {
                        active_count: 1,
                        terminal_count: 0,
                        transfers: vec![Transfer {
                            transfer_id: "t1".into(),
                            peer: "p1".into(),
                            direction: TransferDirection::Outbound,
                            filename: "a.zip".into(),
                            total: 10,
                            transferred: 4,
                            speed_bps: 2,
                            phase: TransferPhase::Transferring,
                            error: None,
                            started_ms: 1,
                            updated_ms: 2,
                        }],
                    },
                },
                "transfers.changed",
                Some(Topic::Transfers),
            ),
            (
                Event::InboxChanged {
                    inbox: Inbox {
                        count: 1,
                        total_bytes: 3,
                        files: vec![InboxFile {
                            name: "b.txt".into(),
                            size: 3,
                            modified_at_secs: 5,
                            is_safe: true,
                        }],
                    },
                },
                "inbox.changed",
                Some(Topic::Inbox),
            ),
            (
                Event::LayoutChanged {
                    layout: Layout {
                        display: DisplayRect {
                            width: 1470,
                            height: 956,
                            scale_factor: 2.0,
                        },
                        edges: vec![LayoutEdge {
                            edge: "right".into(),
                            peer: Some("p1".into()),
                        }],
                        config: CrossingConfig {
                            resistance_px: 15,
                            delay_ms: 100,
                            corner_lock_size_px: 8,
                            wrap_around: false,
                            redundancy_band_px: 30,
                            slow_dwell_ms: 600,
                            fast_push_delta_px: 12,
                            entry_return_grace_ms: 600,
                            remote_speed_percent: 100,
                        },
                        policy: LayoutPolicy {
                            enabled: true,
                            allow_inbound_control: true,
                            auto_link_paired_peers: true,
                        },
                    },
                },
                "layout.changed",
                Some(Topic::Layout),
            ),
            (
                Event::SettingsChanged {
                    settings: Settings {
                        general: SettingsGeneral {
                            name: "mac".into(),
                            advertise: true,
                            reconnect: true,
                            loopback_only: false,
                            overlay_enabled: true,
                        },
                        transfer: SettingsTransfer { auto_accept: false },
                    },
                },
                "settings.changed",
                Some(Topic::Settings),
            ),
        ];
        for (ev, name, topic) in cases {
            assert_eq!(ev.name(), name);
            assert_eq!(ev.topic(), topic);
            assert_eq!(serde_json::to_value(&ev).unwrap()["type"], name);
        }
    }

    #[test]
    fn topics_parse_their_wire_names_and_reject_unknown() {
        for t in Topic::ALL {
            assert_eq!(Topic::parse(t.as_str()), Some(t));
            assert_eq!(serde_json::to_value(t).unwrap(), t.as_str());
        }
        assert_eq!(Topic::parse("nope"), None);
        assert_eq!(
            Topic::parse_list("devices, input").unwrap(),
            vec![Topic::Devices, Topic::Input]
        );
        assert_eq!(Topic::parse_list("devices,bogus").unwrap_err(), "bogus");
    }
}

mod commands {
    use serde_json::json;
    use tetherly_api::*;

    #[test]
    fn device_patch_fields_are_optional_and_strict() {
        let p: DevicePatch = serde_json::from_value(json!({"alias": "Desk"})).unwrap();
        assert_eq!(p.alias.as_deref(), Some("Desk"));
        assert_eq!(p.tags, None);
        let p: DevicePatch = serde_json::from_value(json!({})).unwrap();
        assert!(p.is_empty());
        assert!(serde_json::from_value::<DevicePatch>(json!({"alais": "x"})).is_err());
    }

    #[test]
    fn optional_request_bodies_accept_empty_objects() {
        let c: ConnectRequest = serde_json::from_value(json!({})).unwrap();
        assert_eq!(c.addr, None);
        let b: BlockRequest = serde_json::from_value(json!({"reason": "spam"})).unwrap();
        assert_eq!(b.reason.as_deref(), Some("spam"));
        assert!(serde_json::from_value::<BlockRequest>(json!({"why": 1})).is_err());
    }

    #[test]
    fn diagnostics_wire_shape() {
        let d = Diagnostics {
            event_seq: 9,
            legacy_routes: vec![LegacyRouteHits {
                method: "GET".into(),
                route: "/api/peers".into(),
                caller: "cli".into(),
                count: 3,
            }],
        };
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["legacy_routes"][0]["caller"], "cli");
        assert_eq!(v["legacy_routes"][0]["count"], 3);
    }

    #[test]
    fn log_level_request_needs_filter_or_reset() {
        let r: LogLevelRequest = serde_json::from_value(json!({"filter": "debug"})).unwrap();
        assert_eq!(r.filter.as_deref(), Some("debug"));
        assert!(!r.reset);
        let r: LogLevelRequest = serde_json::from_value(json!({"reset": true})).unwrap();
        assert!(r.reset);
        let g = PinGrant {
            pin: "1234-5678".into(),
            expires_in_secs: 60,
        };
        assert_eq!(serde_json::to_value(&g).unwrap()["pin"], "1234-5678");
    }

    #[test]
    fn settings_patch_fields_are_optional_and_strict() {
        let p: SettingsPatch = serde_json::from_value(json!({"name": "Desk"})).unwrap();
        assert_eq!(p.name.as_deref(), Some("Desk"));
        assert_eq!(p.advertise, None);
        assert!(serde_json::from_value::<SettingsPatch>(json!({}))
            .unwrap()
            .is_empty());
        assert!(serde_json::from_value::<SettingsPatch>(json!({"nama": "x"})).is_err());
        let p: SettingsPatch = serde_json::from_value(json!({"auto_accept": true})).unwrap();
        assert_eq!(p.auto_accept, Some(true));
        assert!(!p.is_empty());
    }

    #[test]
    fn inbox_open_request_carries_the_name() {
        let r: InboxOpenRequest = serde_json::from_value(json!({"name": "a.txt"})).unwrap();
        assert_eq!(r.name, "a.txt");
        assert!(serde_json::from_value::<InboxOpenRequest>(json!({"id": 1})).is_err());
    }
}
