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
            group: None,
            block_reason: None,
        }
    }

    #[test]
    fn device_carries_profile_group() {
        // §144: the CLI peers inventory and the SPA group-by view both need
        // the PeerProfile group; it must survive a serialize round trip.
        let mut d = device();
        d.group = Some("studio".into());
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["group"], "studio");
        let back: Device = serde_json::from_value(v).unwrap();
        assert_eq!(back, d);
    }

    #[test]
    fn device_carries_block_reason() {
        // §151: the CLI blocked inventory needs the reason attached to the
        // Device row (`None` = not blocked); round trip must survive.
        let mut d = device();
        d.blocked = true;
        d.block_reason = Some("spam".into());
        let v = serde_json::to_value(&d).unwrap();
        assert_eq!(v["block_reason"], "spam");
        let back: Device = serde_json::from_value(v).unwrap();
        assert_eq!(back, d);
    }

    fn system() -> System {
        System {
            device_id: "tdev_a".into(),
            name: "mac".into(),
            platform: "macos".into(),
            version: "0.2.59".into(),
            api: API_VERSION,
            pid: 7,
            uptime_secs: 60,
            ports: Ports {
                control: 45717,
                file: 45718,
                input: 45719,
                screen: 45720,
                ui: 45716,
            },
            peers: PeerCounts {
                live: 1,
                trusted: 1,
            },
            caps: vec![],
            features: vec![],
            lan_only: false,
        }
    }

    #[test]
    fn system_carries_lan_only() {
        // §145: the CLI status view and the SPA badge need the overlay
        // absence flag; it must survive a serialize round trip.
        let mut s = system();
        s.lan_only = true;
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["lan_only"], true);
        let back: System = serde_json::from_value(v).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn transfers_carries_session_stats() {
        // §145: the CLI status view needs the session aggregate (bytes in
        // and out, completed/failed) that rode on the legacy /api/state.
        let t = Transfers {
            active_count: 1,
            terminal_count: 2,
            transfers: vec![],
            stats: TransferStats {
                bytes_sent: 10,
                bytes_received: 20,
                completed_count: 1,
                failed_count: 1,
            },
        };
        let v = serde_json::to_value(&t).unwrap();
        assert_eq!(v["stats"]["bytes_sent"], 10);
        assert_eq!(v["stats"]["failed_count"], 1);
        let back: Transfers = serde_json::from_value(v).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn sys_telemetry_mirrors_the_legacy_report() {
        // §145: `GET /diagnostics/sys` keeps the legacy field names so the
        // CLI's human printer needs no reshaping.
        let s = SysTelemetry {
            ok: true,
            node_name: "mac".into(),
            device_id: "tdev_a".into(),
            process: SysProcess {
                pid: 7,
                uptime_secs: 60,
                rss_bytes: 1024,
                virt_bytes: 2048,
                thread_count: 4,
                open_fds: Some(9),
                cpu_time_ms: 500,
                memory_rss_formatted: "1 KB".into(),
                memory_virt_formatted: "2 KB".into(),
            },
            host: SysHost {
                os: "macos".into(),
                arch: "arm64".into(),
                num_cpus: 8,
                total_memory_bytes: Some(4096),
                total_memory_formatted: Some("4 KB".into()),
            },
            timestamp_ms: 1234,
        };
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["process"]["rss_bytes"], 1024);
        assert_eq!(v["host"]["num_cpus"], 8);
        assert_eq!(v["node_name"], "mac");
        let back: SysTelemetry = serde_json::from_value(v).unwrap();
        assert_eq!(back, s);
    }

    #[test]
    fn envelope_carries_type_tag_and_uniform_data() {
        // Wire shape: `{"seq":…,"type":"…","data":<bare payload>}`. A uniform
        // `data` key (not per-variant fields) keeps every consumer — the SPA
        // parser, the Tauri SSE forwarder — a single dumb unwrap.
        let env = EventEnvelope {
            seq: 7,
            event: Event::DeviceUpdated { device: device() },
        };
        let v = serde_json::to_value(&env).unwrap();
        assert_eq!(v["seq"], 7);
        assert_eq!(v["type"], "device.updated");
        assert_eq!(v["data"]["device"]["path"], "lan");
        assert!(v.get("device").is_none(), "payload must sit under data");
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
                        stats: TransferStats::default(),
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
                            tray_owner: TrayOwner::Daemon,
                        },
                        transfer: SettingsTransfer { auto_accept: false },
                    },
                },
                "settings.changed",
                Some(Topic::Settings),
            ),
            (
                Event::ActivityChanged {
                    activity: Activity {
                        candidates: vec![ActivityCandidate {
                            id: "c1".into(),
                            source: "tdev_a".into(),
                            app_id: "com.bank".into(),
                            app_name: "Bank".into(),
                            uid: "42".into(),
                            title: "Your code".into(),
                            has_otp: true,
                            created_ms: 100,
                            expires_ms: 200,
                            actions: vec!["open".into()],
                        }],
                        replies: vec![ActivityReply {
                            id: "r1".into(),
                            source: "tdev_a".into(),
                            app_id: "com.bank".into(),
                            uid: "42".into(),
                        }],
                        messages: vec![ActivityMessage {
                            id: "m1".into(),
                            peer: "tdev_a".into(),
                            direction: MessageDirection::Inbound,
                            text: "hi".into(),
                            timestamp_ms: 300,
                            read: false,
                        }],
                        handoffs: vec![ActivityHandoff {
                            id: "h1".into(),
                            sender_id: "tdev_a".into(),
                            sender_alias: "mac".into(),
                            url: "https://x.test".into(),
                            title: "page".into(),
                            status: HandoffStatus::Pending,
                            received_ms: 400,
                            opened_ms: None,
                        }],
                    },
                },
                "activity.changed",
                Some(Topic::Activity),
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
    fn heartbeat_config_and_patch_round_trip() {
        // §152: the CLI link config view and its write path; patch is
        // strict and partial.
        let cfg = HeartbeatConfig {
            interval_secs: 15,
            deadline_secs: 45,
        };
        let v = serde_json::to_value(&cfg).unwrap();
        assert_eq!(v["interval_secs"], 15);
        let back: HeartbeatConfig = serde_json::from_value(v).unwrap();
        assert_eq!(back, cfg);

        let p: HeartbeatPatch = serde_json::from_value(json!({"interval_secs": 30})).unwrap();
        assert_eq!(p.interval_secs, Some(30));
        assert_eq!(p.deadline_secs, None);
        assert!(serde_json::from_value::<HeartbeatPatch>(json!({}))
            .unwrap()
            .is_empty());
        assert!(serde_json::from_value::<HeartbeatPatch>(json!({"nope": 1})).is_err());
    }

    #[test]
    fn link_quality_snapshot_round_trip() {
        // §152: GET /link snapshot — field names mirror the legacy report
        // so the CLI table printer is unchanged.
        let snap = LinkQuality {
            heartbeat: HeartbeatConfig {
                interval_secs: 10,
                deadline_secs: 30,
            },
            total_peers: 1,
            online_peers: 1,
            links: vec![LinkQualityPeer {
                device_id: "tdev_a".into(),
                name: "mac".into(),
                addr: "192.168.1.7:45717".into(),
                is_online: true,
                last_seen_ms: 5,
                rtt_ms: Some(3),
                min_rtt_ms: Some(2),
                max_rtt_ms: Some(9),
                avg_rtt_ms: Some(4.5),
                jitter_ms: Some(0.4),
                pings_sent: 100,
                pongs_recv: 99,
                packet_loss_rate: 0.01,
                grade: "excellent".into(),
                samples_count: 20,
                connected_uptime_secs: 3600,
            }],
        };
        let v = serde_json::to_value(&snap).unwrap();
        assert_eq!(v["links"][0]["grade"], "excellent");
        let back: LinkQuality = serde_json::from_value(v).unwrap();
        assert_eq!(back, snap);
    }

    #[test]
    fn link_probe_and_ping_reports_round_trip() {
        // §152: device action reports (probe body optional fields strict).
        let req: LinkProbeRequest = serde_json::from_value(json!({"count": 5})).unwrap();
        assert_eq!(req.count, Some(5));
        assert!(serde_json::from_value::<LinkProbeRequest>(json!({"peer": "x"})).is_err());
        let rep = LinkProbeReport {
            device_id: "tdev_a".into(),
            count: 5,
            sent_count: 5,
            recv_count: 4,
            min_rtt_ms: Some(2),
            max_rtt_ms: Some(8),
            avg_rtt_ms: Some(3.5),
            jitter_ms: Some(0.2),
            packet_loss_rate: 0.2,
            samples_ms: vec![2, 3, 8, 2, 3],
            grade: "good".into(),
        };
        let v = serde_json::to_value(&rep).unwrap();
        assert_eq!(v["samples_ms"][2], 8);
        let back: LinkProbeReport = serde_json::from_value(v).unwrap();
        assert_eq!(back, rep);
        let ping = DevicePingReport {
            device_id: "tdev_a".into(),
            rtt_ms: 3,
            path: "lan".into(),
            addr: "192.168.1.7:45717".into(),
        };
        let v = serde_json::to_value(&ping).unwrap();
        let back: DevicePingReport = serde_json::from_value(v).unwrap();
        assert_eq!(back, ping);
    }

    #[test]
    fn inbox_open_request_carries_the_name() {
        let r: InboxOpenRequest = serde_json::from_value(json!({"name": "a.txt"})).unwrap();
        assert_eq!(r.name, "a.txt");
        assert!(serde_json::from_value::<InboxOpenRequest>(json!({"id": 1})).is_err());
    }
}
