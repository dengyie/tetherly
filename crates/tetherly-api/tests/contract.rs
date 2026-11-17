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
        let v = serde_json::to_value(cfg).unwrap();
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

    #[test]
    fn traffic_telemetry_round_trip_and_reset_semantics() {
        // §153: traffic/protocol telemetry mirrors the legacy /api/metrics
        // report field-for-field; a PATCH reset folds into the same GET
        // readback (all-zero snapshot, same shape).
        let rep = TrafficTelemetry {
            uptime_secs: 60,
            total_tx_bytes: 1024,
            total_rx_bytes: 2048,
            total_tx_frames: 10,
            total_rx_frames: 20,
            control: ChannelTraffic {
                tx_bytes: 512,
                rx_bytes: 1024,
                tx_frames: 5,
                rx_frames: 10,
            },
            file: ChannelTraffic::default(),
            input: ChannelTraffic::default(),
            screen: ChannelTraffic::default(),
            messages: ProtocolMessages {
                pings_sent: 4,
                pongs_recv: 4,
                ..ProtocolMessages::default()
            },
            sessions_established: 1,
            sessions_closed: 0,
            handshakes_failed: 0,
        };
        let v = serde_json::to_value(&rep).unwrap();
        assert_eq!(v["control"]["tx_bytes"], 512);
        assert_eq!(v["messages"]["pings_sent"], 4);
        let back: TrafficTelemetry = serde_json::from_value(v).unwrap();
        assert_eq!(back, rep);
        // Reset readback: same shape, zeroed counters.
        let reset = serde_json::from_value::<TrafficTelemetry>(json!({
            "uptime_secs": 0,
            "total_tx_bytes": 0,
            "total_rx_bytes": 0,
            "total_tx_frames": 0,
            "total_rx_frames": 0,
            "control": {"tx_bytes": 0, "rx_bytes": 0, "tx_frames": 0, "rx_frames": 0},
            "file": {"tx_bytes": 0, "rx_bytes": 0, "tx_frames": 0, "rx_frames": 0},
            "input": {"tx_bytes": 0, "rx_bytes": 0, "tx_frames": 0, "rx_frames": 0},
            "screen": {"tx_bytes": 0, "rx_bytes": 0, "tx_frames": 0, "rx_frames": 0},
            "messages": {"pings_sent": 0, "pings_recv": 0, "pongs_sent": 0, "pongs_recv": 0,
                "notifies_sent": 0, "notifies_recv": 0, "clips_sent": 0, "clips_recv": 0,
                "file_offers_sent": 0, "file_offers_recv": 0},
            "sessions_established": 0,
            "sessions_closed": 0,
            "handshakes_failed": 0,
        }))
        .unwrap();
        assert_eq!(reset, TrafficTelemetry::default());
    }

    #[test]
    fn dial_request_and_report_round_trip() {
        // §154: POST /pairing/dial — deny_unknown_fields, exactly-one target
        // enforced handler-side; report mirrors the legacy {ok, addr} shape.
        let req: DialRequest =
            serde_json::from_value(json!({"addr": "192.168.1.7:45717", "pin": "12345678"}))
                .unwrap();
        assert_eq!(req.addr.as_deref(), Some("192.168.1.7:45717"));
        assert!(serde_json::from_value::<DialRequest>(json!({"host": "x"})).is_err());
        let rep = DialReport {
            ok: true,
            addr: "192.168.1.7:45717".into(),
        };
        let v = serde_json::to_value(&rep).unwrap();
        assert_eq!(v["ok"], true);
        let back: DialReport = serde_json::from_value(v).unwrap();
        assert_eq!(back, rep);
    }

    #[test]
    fn profile_domain_round_trips_and_patch_semantics() {
        // §156a: GET/PATCH /profiles — DTOs mirror the runtime PeerProfile;
        // patch distinguish None (leave) / Some(None) (clear) / Some(Some).
        let prof = PeerProfileDto {
            device_id: "tdev_abc".into(),
            device_type: "desktop".into(),
            tags: vec!["lab".into()],
            group: Some("office".into()),
            notes: Some("main rig".into()),
            updated_ms: 1234,
        };
        let v = serde_json::to_value(&prof).unwrap();
        assert_eq!(v["group"], "office");
        let back: PeerProfileDto = serde_json::from_value(v).unwrap();
        assert_eq!(back, prof);

        let list = PeerProfiles {
            total: 1,
            profiles: vec![prof.clone()],
        };
        let back_list: PeerProfiles =
            serde_json::from_value(serde_json::to_value(&list).unwrap()).unwrap();
        assert_eq!(back_list, list);

        let set_group: PeerProfilePatch =
            serde_json::from_value(json!({"group": "office"})).unwrap();
        assert_eq!(set_group.group, Patch::Set("office".into()));
        let clear_group: PeerProfilePatch = serde_json::from_value(json!({"group": null})).unwrap();
        assert_eq!(clear_group.group, Patch::Clear);
        let leave_group: PeerProfilePatch = serde_json::from_value(json!({})).unwrap();
        assert_eq!(leave_group.group, Patch::Leave);
        assert!(leave_group.is_empty());
        assert!(!set_group.is_empty());
        assert!(serde_json::from_value::<PeerProfilePatch>(json!({"peer": "x"})).is_err());
        // Patch helpers
        assert_eq!(set_group.group.set(), Some(&"office".to_string()));
        assert!(clear_group.group.is_clear());

        let updated = PeerProfileUpdated {
            device_id: "tdev_abc".into(),
            profile: prof,
        };
        let back_u: PeerProfileUpdated =
            serde_json::from_value(serde_json::to_value(&updated).unwrap()).unwrap();
        assert_eq!(back_u, updated);
    }

    #[test]
    fn caps_domain_round_trips() {
        // §156a: GET/PATCH /caps + GET /devices/{id}/caps — LocalCaps mirrors
        // LocalCapsReport (ok folded), PeerCaps mirrors PeerCapsDto.
        let upd: CapsUpdate =
            serde_json::from_value(json!({"enable": ["file"], "disable": ["screen"]})).unwrap();
        assert_eq!(upd.enable, vec!["file".to_string()]);
        assert_eq!(upd.disable, vec!["screen".to_string()]);
        assert!(serde_json::from_value::<CapsUpdate>(json!({"other": 1})).is_err());

        let local = LocalCaps {
            device_id: "tdev_me".into(),
            caps: vec!["file".into()],
            features: vec!["file".into()],
            file_port: 45718,
            input_port: 45719,
            screen_port: 45720,
        };
        let v = serde_json::to_value(&local).unwrap();
        assert_eq!(v["file_port"], 45718);
        let back: LocalCaps = serde_json::from_value(v).unwrap();
        assert_eq!(back, local);

        let peer = PeerCaps {
            device_id: "tdev_p".into(),
            name: "DESKTOP".into(),
            alias: None,
            caps: vec![],
            features: vec![],
            file_port: None,
            input_port: None,
            screen_port: None,
            online: true,
        };
        let back_p: PeerCaps =
            serde_json::from_value(serde_json::to_value(&peer).unwrap()).unwrap();
        assert_eq!(back_p, peer);
    }

    fn topology_fixture() -> Topology {
        Topology {
            display: DisplayRect {
                width: 1920,
                height: 1080,
                scale_factor: 2.0,
            },
            neighbors: vec![TopologyEdgeNeighbor {
                peer_id: "tdev_p".into(),
                peer_edge: "right".into(),
                offset: -64,
            }],
            neighbor_edges: vec!["left".into()],
            config: CrossingConfig {
                resistance_px: 24,
                delay_ms: 350,
                corner_lock_size_px: 16,
                wrap_around: true,
                redundancy_band_px: 12,
                slow_dwell_ms: 600,
                fast_push_delta_px: 40,
                entry_return_grace_ms: 800,
                remote_speed_percent: 70,
            },
            peer_count: 1,
        }
    }

    #[test]
    fn topology_domain_round_trips_and_requests_are_strict() {
        // §156b: GET /topology + GET /topology/peers read shapes.
        let t = topology_fixture();
        let v = serde_json::to_value(&t).unwrap();
        assert_eq!(v["display"]["width"], 1920);
        assert_eq!(v["neighbors"][0]["peer_edge"], "right");
        assert_eq!(v["neighbor_edges"][0], "left");
        let back: Topology = serde_json::from_value(v).unwrap();
        assert_eq!(back, t);

        let peers = PeerTopologies {
            count: 1,
            peers: vec![PeerTopologyDto {
                peer_id: "tdev_p".into(),
                peer_alias: Some("DESKTOP".into()),
                display: t.display,
                config: t.config,
                timestamp_ms: 1_700_000_000_000,
                neighbors_count: 2,
            }],
        };
        let back_p: PeerTopologies =
            serde_json::from_value(serde_json::to_value(&peers).unwrap()).unwrap();
        assert_eq!(back_p, peers);

        // POST /topology/links — edge/peer required; unknown fields denied.
        let req: TopologyLinkRequest =
            serde_json::from_value(json!({"edge": "left", "peer": "DESKTOP", "offset": -64}))
                .unwrap();
        assert_eq!(req.peer_edge, None);
        assert_eq!(req.offset, Some(-64));
        assert!(serde_json::from_value::<TopologyLinkRequest>(json!({"edge": "left"})).is_err());
        assert!(serde_json::from_value::<TopologyLinkRequest>(
            json!({"edge": "left", "peer": "p", "x": 1})
        )
        .is_err());
        let unlink: TopologyUnlinkRequest =
            serde_json::from_value(json!({"edge": "bottom"})).unwrap();
        assert_eq!(unlink.edge, "bottom");

        // PATCH /topology/config — sparse: absent keys stay absent.
        let empty: TopologyConfigPatch = serde_json::from_value(json!({})).unwrap();
        assert_eq!(empty, TopologyConfigPatch::default());
        let partial: TopologyConfigPatch =
            serde_json::from_value(json!({"resistance_px": 48, "wrap_around": false})).unwrap();
        assert_eq!(partial.resistance_px, Some(48));
        assert_eq!(partial.delay_ms, None);
        assert!(serde_json::from_value::<TopologyConfigPatch>(json!({"nope": 1})).is_err());

        // PATCH /topology/policy — sparse + clear_filters flag.
        let pol: TopologyPolicyPatch = serde_json::from_value(json!({
            "allowed_tags": ["desk"], "clear_filters": true
        }))
        .unwrap();
        assert_eq!(pol.allowed_tags, Some(vec!["desk".to_string()]));
        assert_eq!(pol.clear_filters, Some(true));
        assert_eq!(pol.enabled, None);
        assert!(serde_json::from_value::<TopologyPolicyPatch>(json!({"nope": 1})).is_err());

        let rep = TopologySyncReport {
            sync_requests_sent: 2,
            broadcasts_sent: 1,
        };
        let back_r: TopologySyncReport =
            serde_json::from_value(serde_json::to_value(&rep).unwrap()).unwrap();
        assert_eq!(back_r, rep);
    }

    #[test]
    fn routes_and_network_domain_round_trips() {
        // §156b: GET /routes + GET /net + POST /routes + POST /net/refresh.
        let routes = Routes {
            count: 1,
            routes: vec![RouteEntryDto {
                device_id: "tdev_p".into(),
                addr: "192.168.1.20:45000".into(),
                alias: Some("desk".into()),
                name: None,
                online: true,
                trusted: true,
            }],
        };
        let v = serde_json::to_value(&routes).unwrap();
        assert_eq!(v["count"], 1);
        assert_eq!(v["routes"][0]["addr"], "192.168.1.20:45000");
        let back: Routes = serde_json::from_value(v).unwrap();
        assert_eq!(back, routes);

        let req: RouteSetRequest =
            serde_json::from_value(json!({"peer": "DESKTOP", "addr": "192.168.1.20:45000"}))
                .unwrap();
        assert_eq!(req.peer, "DESKTOP");
        assert!(serde_json::from_value::<RouteSetRequest>(json!({"peer": "p"})).is_err());
        assert!(serde_json::from_value::<RouteSetRequest>(
            json!({"peer": "p", "addr": "a", "z": 1})
        )
        .is_err());
        let rep = RouteMutateReport {
            device_id: "tdev_p".into(),
            addr: Some("192.168.1.20:45000".into()),
            action: "added".into(),
        };
        let back_r: RouteMutateReport =
            serde_json::from_value(serde_json::to_value(&rep).unwrap()).unwrap();
        assert_eq!(back_r, rep);

        let net = Network {
            interfaces: vec![IfaceReport {
                iface: "en0".into(),
                ip: "192.168.1.5".into(),
                listen: true,
                reason: None,
            }],
            overlay: OverlayStatus {
                enabled: true,
                present: true,
                source: "sidecar".into(),
                local_ips: vec!["10.144.144.2".into()],
                peer_count: 1,
                peers: vec!["tdev_p".into()],
            },
        };
        let back_n: Network = serde_json::from_value(serde_json::to_value(&net).unwrap()).unwrap();
        assert_eq!(back_n, net);

        let refresh = NetworkRefreshReport {
            ifaces_scanned: 6,
            ifaces_listening: 2,
            overlay_present: true,
            overlay_source: "sidecar".into(),
            overlay_local_ips: vec!["10.144.144.2".into()],
            overlay_peers_count: 1,
        };
        let back_f: NetworkRefreshReport =
            serde_json::from_value(serde_json::to_value(&refresh).unwrap()).unwrap();
        assert_eq!(back_f, refresh);
    }

    /// §160: discovery + daemon lifecycle round-trips and strictness.
    #[test]
    fn discovery_and_daemon_lifecycle_round_trips() {
        let disc = Discovery {
            count: 1,
            peers: vec![DiscoveredPeer {
                device_id: "tdev_q".into(),
                name: "laptop".into(),
                addrs: vec!["192.168.1.9:46100".into()],
                port: 46100,
                fp: Some("aabb".into()),
                caps: vec!["notify".into()],
                is_trusted: false,
                is_online: true,
                last_seen_ms: 42,
            }],
        };
        let v = serde_json::to_value(&disc).unwrap();
        assert_eq!(v["count"], 1);
        assert_eq!(v["peers"][0]["is_trusted"], false);
        let back: Discovery = serde_json::from_value(v).unwrap();
        assert_eq!(back, disc);

        let rescan = DiscoveryRescanReport {
            message: "discovery rescan started".into(),
        };
        let back_r: DiscoveryRescanReport =
            serde_json::from_value(serde_json::to_value(&rescan).unwrap()).unwrap();
        assert_eq!(back_r, rescan);

        // Restart request: sparse (all-default) must parse; unknown fields
        // must be rejected; delay folds to the contract field.
        let req: DaemonRestartRequest = serde_json::from_value(json!({})).unwrap();
        assert_eq!(req.replacement_path, None);
        assert_eq!(req.delay_ms, None);
        let req: DaemonRestartRequest = serde_json::from_value(json!({"delay_ms": 500})).unwrap();
        assert_eq!(req.delay_ms, Some(500));
        assert!(serde_json::from_value::<DaemonRestartRequest>(json!({"z": 1})).is_err());

        let rep = DaemonRestartReport {
            action: "replaced_and_restarted".into(),
            old_pid: 100,
            new_pid: 200,
            timestamp_ms: 7,
        };
        let back_p: DaemonRestartReport =
            serde_json::from_value(serde_json::to_value(&rep).unwrap()).unwrap();
        assert_eq!(back_p, rep);

        let shut = DaemonShutdownReport {
            status: "shutting_down".into(),
        };
        let back_s: DaemonShutdownReport =
            serde_json::from_value(serde_json::to_value(&shut).unwrap()).unwrap();
        assert_eq!(back_s, shut);
    }
}
