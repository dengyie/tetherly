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
    fn inbox_open_report_names_the_file_or_the_directory() {
        // §162a: POST /inbox/{name}/open answers the open report — a named
        // file or the directory reveal (name null), both with the on-disk
        // path so CLI/JSON printers keep the legacy detail level.
        let file = InboxOpenReport {
            name: Some("a.txt".into()),
            path: "/data/inbox/a.txt".into(),
        };
        let v = serde_json::to_value(&file).unwrap();
        assert_eq!(v["name"], "a.txt");
        assert_eq!(v["path"], "/data/inbox/a.txt");
        let back: InboxOpenReport = serde_json::from_value(v).unwrap();
        assert_eq!(back, file);

        let dir = InboxOpenReport {
            name: None,
            path: "/data/inbox".into(),
        };
        let v = serde_json::to_value(&dir).unwrap();
        // Null name serializes (the resource is the directory reveal).
        assert!(v.get("name").is_some());
        let back: InboxOpenReport = serde_json::from_value(v).unwrap();
        assert_eq!(back, dir);
        assert!(serde_json::from_value::<InboxOpenReport>(json!({"path": 1})).is_err());
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

    #[test]
    fn notify_domain_round_trips() {
        // Send request: sparse defaults parse; unknown fields rejected.
        let req: NotifySendRequest =
            serde_json::from_value(json!({"title": "t", "body": "b"})).unwrap();
        assert_eq!(req.peer, None);
        assert_eq!(req.app_id, None);
        assert!(serde_json::from_value::<NotifySendRequest>(json!({"z": 1})).is_err());

        let report = NotifySendReport {
            peer: "tdev_a".into(),
        };
        let back: NotifySendReport =
            serde_json::from_value(serde_json::to_value(&report).unwrap()).unwrap();
        assert_eq!(back, report);

        let cand = NotifyCandidate {
            id: "c1".into(),
            source: "tdev_a".into(),
            app_id: "com.example.sms".into(),
            app_name: "Messages".into(),
            uid: "uid-1".into(),
            title: "code".into(),
            has_otp: true,
            created_ms: 1,
            expires_ms: 2,
            actions: vec!["copy".into()],
        };
        let list = NotifyCandidates {
            count: 1,
            candidates: vec![cand],
        };
        let v = serde_json::to_value(&list).unwrap();
        assert_eq!(v["count"], 1);
        assert_eq!(v["candidates"][0]["has_otp"], true);
        let back_l: NotifyCandidates = serde_json::from_value(v).unwrap();
        assert_eq!(back_l, list);

        let replies = NotifyReplies {
            count: 1,
            replies: vec![NotifyReplyDto {
                id: "r1".into(),
                source: "tdev_a".into(),
                app_id: "com.example.sms".into(),
                uid: "uid-1".into(),
            }],
        };
        let back_r: NotifyReplies =
            serde_json::from_value(serde_json::to_value(&replies).unwrap()).unwrap();
        assert_eq!(back_r, replies);

        let otp = NotifyOtpReport {
            id: "c1".into(),
            otp: "123456".into(),
        };
        let back_o: NotifyOtpReport =
            serde_json::from_value(serde_json::to_value(&otp).unwrap()).unwrap();
        assert_eq!(back_o, otp);

        // Dismiss request: all-optional body parses empty; unknown rejected.
        let d: NotifyDismissRequest = serde_json::from_value(json!({})).unwrap();
        assert_eq!(d.id, None);
        assert_eq!(d.all, None);
        assert!(serde_json::from_value::<NotifyDismissRequest>(json!({"z": 1})).is_err());
        // Dismiss report folds single + batch shapes.
        let single: NotifyDismissReport = serde_json::from_value(
            json!({"id": "c1", "source": "tdev_a", "notified_remote": true}),
        )
        .unwrap();
        assert_eq!(single.id.as_deref(), Some("c1"));
        assert_eq!(single.count, None);
        let batch: NotifyDismissReport = serde_json::from_value(
            json!({"count": 2, "ids": ["a", "b"], "notified_remote_count": 1}),
        )
        .unwrap();
        assert_eq!(batch.count, Some(2));
        assert_eq!(batch.ids.as_deref().map(<[String]>::len), Some(2));

        let take_req: NotifyReplyTakeRequest = serde_json::from_value(json!({"id": "r1"})).unwrap();
        assert_eq!(take_req.id, "r1");
        let take = NotifyReplyTakeReport {
            id: "r1".into(),
            uid: "uid-1".into(),
            app_id: "com.example.sms".into(),
            text: "hi".into(),
        };
        let back_t: NotifyReplyTakeReport =
            serde_json::from_value(serde_json::to_value(&take).unwrap()).unwrap();
        assert_eq!(back_t, take);

        // Config: GET report + PATCH subset; DND mode is snake_case.
        let cfg = NotifyConfigReportDto {
            config: NotifyConfigDto {
                banners_enabled: true,
                dnd_mode: NotifyDndMode::Silent,
                sound_enabled: true,
                mask_otp: true,
                allow_otp_in_dnd: true,
            },
            totals: NotifyConfigTotals {
                dispatched: 3,
                suppressed: 1,
            },
        };
        let v = serde_json::to_value(&cfg).unwrap();
        assert_eq!(v["config"]["dnd_mode"], "silent");
        assert_eq!(v["totals"]["dispatched"], 3);
        let back_c: NotifyConfigReportDto = serde_json::from_value(v).unwrap();
        assert_eq!(back_c, cfg);
        let patch: NotifyConfigPatch = serde_json::from_value(json!({"dnd_mode": "all"})).unwrap();
        assert_eq!(patch.dnd_mode, Some(NotifyDndMode::All));
        assert_eq!(patch.banners_enabled, None);
        assert!(serde_json::from_value::<NotifyConfigPatch>(json!({"z": 1})).is_err());

        // Test request/report: sparse request, suppressed report keeps reason.
        let tr: NotifyTestRequestDto = serde_json::from_value(json!({})).unwrap();
        assert_eq!(tr.title, None);
        assert!(serde_json::from_value::<NotifyTestRequestDto>(json!({"z": 1})).is_err());
        let rep = NotifyTestReportDto {
            sent: false,
            suppressed: true,
            suppression_reason: Some("banners_disabled".into()),
            message: "notification suppressed".into(),
            title: "Tetherly 桌面通知测试".into(),
        };
        let back_p: NotifyTestReportDto =
            serde_json::from_value(serde_json::to_value(&rep).unwrap()).unwrap();
        assert_eq!(back_p, rep);
        let plain = NotifyTestReportDto {
            sent: true,
            suppressed: false,
            suppression_reason: None,
            message: "dispatched".into(),
            title: "t".into(),
        };
        let v = serde_json::to_value(&plain).unwrap();
        assert!(v.get("suppression_reason").is_none());
        let back_q: NotifyTestReportDto = serde_json::from_value(v).unwrap();
        assert_eq!(back_q, plain);
    }

    #[test]
    fn message_domain_round_trips() {
        let msg = MessageEntry {
            id: "msg_1".into(),
            peer: "tdev_a".into(),
            direction: MessageDirection::Outbound,
            text: "hello".into(),
            timestamp_ms: 42,
            read: true,
        };
        let send = MessageSendReport {
            message: msg.clone(),
        };
        let v = serde_json::to_value(&send).unwrap();
        assert_eq!(v["message"]["direction"], "outbound");
        let back: MessageSendReport = serde_json::from_value(v).unwrap();
        assert_eq!(back, send);

        // Send request: text required; selector fields optional; unknown
        // fields rejected.
        let req: MessageSendRequest = serde_json::from_value(json!({"text": "hi"})).unwrap();
        assert_eq!(req.peer, None);
        assert_eq!(req.all, None);
        assert!(serde_json::from_value::<MessageSendRequest>(json!({})).is_err());
        assert!(
            serde_json::from_value::<MessageSendRequest>(json!({"text": "x", "z": 1})).is_err()
        );

        let multi = MessageDispatchReport {
            selector: "tag:lab".into(),
            target_count: 2,
            success_count: 1,
            skipped_offline_count: 1,
            failed_count: 0,
            dispatches: vec![MessageDispatchItem {
                peer: "tdev_a".into(),
                peer_alias: Some("laptop".into()),
                message_id: Some("msg_1".into()),
                success: true,
                error: None,
            }],
        };
        let v = serde_json::to_value(&multi).unwrap();
        assert_eq!(v["target_count"], 2);
        assert_eq!(v["dispatches"][0]["peer_alias"], "laptop");
        let back_m: MessageDispatchReport = serde_json::from_value(v).unwrap();
        assert_eq!(back_m, multi);

        // History query + report.
        let q: MessageQuery = serde_json::from_value(json!({"limit": 5})).unwrap();
        assert_eq!(q.limit, Some(5));
        assert_eq!(q.unread_only, None);
        assert!(serde_json::from_value::<MessageQuery>(json!({"z": 1})).is_err());
        let hist = MessageHistory {
            total: 1,
            returned: 1,
            unread: 0,
            messages: vec![msg],
        };
        let back_h: MessageHistory =
            serde_json::from_value(serde_json::to_value(&hist).unwrap()).unwrap();
        assert_eq!(back_h, hist);

        // Clear request (sparse) + report.
        let cr: MessageClearRequest = serde_json::from_value(json!({})).unwrap();
        assert_eq!(cr.peer, None);
        assert!(serde_json::from_value::<MessageClearRequest>(json!({"z": 1})).is_err());
        let clr = MessageClearReport {
            target: "all".into(),
            cleared_count: 4,
            message: "cleared".into(),
        };
        let back_x: MessageClearReport =
            serde_json::from_value(serde_json::to_value(&clr).unwrap()).unwrap();
        assert_eq!(back_x, clr);
    }

    #[test]
    fn file_domain_reports_round_trip() {
        // §162b: file-domain resources mirror the legacy /api/file reports
        // field-for-field; CLI JSON printers depend on the exact shapes.

        // Offers resource with one pending offer.
        let offer = FileOffer {
            transfer_id: "t1".into(),
            filename: "a.bin".into(),
            total_bytes: 128,
            resume_from: 0,
            source: "peer-a".into(),
            peer: "peer-a".into(),
            peer_alias: None,
            size: 128,
            sha256: "abc".into(),
            timestamp_ms: 1000,
            offered_ms: 1000,
        };
        let offers = FileOffers {
            count: 1,
            auto_accept_trusted: false,
            offers: vec![offer],
        };
        let v = serde_json::to_value(&offers).unwrap();
        assert_eq!(v["count"], 1);
        assert_eq!(v["offers"][0]["filename"], "a.bin");
        let back: FileOffers = serde_json::from_value(v).unwrap();
        assert_eq!(back, offers);

        // Decision request is strict; the folded report covers single+batch.
        let q: FileDecisionRequest = serde_json::from_value(json!({"id": "t1"})).unwrap();
        assert_eq!(q.id.as_deref(), Some("t1"));
        assert!(serde_json::from_value::<FileDecisionRequest>(json!({"z": 1})).is_err());
        let rep = FileDecisionReport {
            accepted: Some(true),
            action: Some("accepted".into()),
            transfer_id: Some("t1".into()),
            peer: Some("peer-a".into()),
            filename: Some("a.bin".into()),
            total_bytes: Some(128),
            accepted_count: 1,
            rejected_count: 0,
            total_evaluated: 1,
            decisions: Vec::new(),
        };
        let back_r: FileDecisionReport =
            serde_json::from_value(serde_json::to_value(&rep).unwrap()).unwrap();
        assert_eq!(back_r, rep);

        // Send report + dispatch report (selector carries the label form).
        let send = FileSendReport {
            transfer_id: "t2".into(),
            name: "b.txt".into(),
            size: 64,
            peer: "peer-b".into(),
            sha256: None,
        };
        // Optional sha256 serializes as null when absent.
        assert!(serde_json::to_value(&send).unwrap()["sha256"].is_null());
        let disp = FileDispatchReport {
            selector: "tag:lab".into(),
            filename: "b.txt".into(),
            size: 64,
            sha256: "def".into(),
            target_count: 2,
            success_count: 1,
            skipped_offline_count: 1,
            failed_count: 0,
            dispatches: Vec::new(),
        };
        let back_d: FileDispatchReport =
            serde_json::from_value(serde_json::to_value(&disp).unwrap()).unwrap();
        assert_eq!(back_d, disp);

        // Batch report with one per-file item.
        let batch = FileBatchReport {
            batch_id: "b1".into(),
            peer: "peer-b".into(),
            peer_alias: None,
            total_files: 1,
            successful_files: 1,
            failed_files: 0,
            total_bytes: 64,
            items: vec![FileBatchItem {
                transfer_id: Some("t3".into()),
                name: "c.txt".into(),
                size: 64,
                sha256: None,
                status: "queued".into(),
                error: None,
            }],
        };
        let back_b: FileBatchReport =
            serde_json::from_value(serde_json::to_value(&batch).unwrap()).unwrap();
        assert_eq!(back_b, batch);
    }

    #[test]
    fn outbox_storage_resume_reports_round_trip() {
        // Outbox entry: status is the plain as_str form; optional extras
        // vanish when absent.
        let entry = OutboxEntryDto {
            id: "o1".into(),
            transfer_id: "t1".into(),
            peer: "peer-a".into(),
            peer_alias: Some("Desk".into()),
            filename: "a.bin".into(),
            size: 128,
            sha256: "abc".into(),
            source_path: None,
            status: "completed".into(),
            error: None,
            started_ms: 1000,
            completed_ms: Some(2000),
            speed_bps: 512,
        };
        // Optional extras serialize as null when absent.
        assert!(serde_json::to_value(&entry).unwrap()["error"].is_null());
        assert!(serde_json::to_value(&entry).unwrap()["source_path"].is_null());
        let ring = Outbox {
            total: 1,
            returned: 1,
            entries: vec![entry],
        };
        let back: Outbox = serde_json::from_value(serde_json::to_value(&ring).unwrap()).unwrap();
        assert_eq!(back, ring);

        // Clear/resend reports.
        let clr = OutboxClearReport {
            cleared_count: 3,
            target_peer: Some("peer-a".into()),
            target_status: Some("failed".into()),
        };
        let back_c: OutboxClearReport =
            serde_json::from_value(serde_json::to_value(&clr).unwrap()).unwrap();
        assert_eq!(back_c, clr);
        let rs = OutboxResendReport {
            original_id: "o1".into(),
            new_transfer_id: "t9".into(),
        };
        let back_r: OutboxResendReport =
            serde_json::from_value(serde_json::to_value(&rs).unwrap()).unwrap();
        assert_eq!(back_r, rs);

        // Storage snapshot + clean report.
        let st = Storage {
            data_dir: "/data".into(),
            inbox_dir: "/data/inbox".into(),
            inbox_total_bytes: 128,
            inbox_file_count: 1,
            partial_bytes: 0,
            partial_count: 0,
            orphaned_partial_count: 0,
            orphaned_partial_bytes: 0,
            meta_bytes: 64,
            resume_markers_count: 0,
        };
        let back_s: Storage = serde_json::from_value(serde_json::to_value(&st).unwrap()).unwrap();
        assert_eq!(back_s, st);
        let cl = StorageCleanReport {
            freed_bytes: 128,
            deleted_files_count: 2,
            cleared_resume_entries: 1,
        };
        let back_l: StorageCleanReport =
            serde_json::from_value(serde_json::to_value(&cl).unwrap()).unwrap();
        assert_eq!(back_l, cl);

        // Resume marker: progress is a float, optional alias vanishes.
        let mk = ResumeMarker {
            transfer_id: "t1".into(),
            source: "peer-a".into(),
            source_alias: None,
            total_bytes: 1024,
            offset_bytes: 512,
            progress_percent: 50.0,
            chunk_aligned: true,
            part_file_exists: true,
            part_file_bytes: 512,
        };
        let v = serde_json::to_value(&mk).unwrap();
        assert_eq!(v["progress_percent"], 50.0);
        assert!(v["source_alias"].is_null());
        let back_m: ResumeMarker = serde_json::from_value(v).unwrap();
        assert_eq!(back_m, mk);
        let markers = ResumeMarkers {
            count: 1,
            total_partial_bytes: 512,
            markers: vec![mk],
        };
        let back_ms: ResumeMarkers =
            serde_json::from_value(serde_json::to_value(&markers).unwrap()).unwrap();
        assert_eq!(back_ms, markers);
    }

    #[test]
    fn auto_accept_policy_patch_is_tri_state() {
        // §162b: the policy GET resource is flat; the PATCH body is
        // tri-state — absent leaves, JSON null clears, a value sets.
        let pol = AutoAcceptPolicyDto {
            enabled: true,
            allowed_groups: vec!["lab".into()],
            allowed_tags: Vec::new(),
            allowed_types: vec!["image/png".into()],
            max_file_size: Some(1024),
            auto_accepted_count: 7,
        };
        let v = serde_json::to_value(&pol).unwrap();
        assert_eq!(v["enabled"], true);
        assert_eq!(v["max_file_size"], 1024);
        assert_eq!(v["auto_accepted_count"], 7);
        let back: AutoAcceptPolicyDto = serde_json::from_value(v).unwrap();
        assert_eq!(back, pol);

        // Sparse patch: absent fields stay untouched.
        let p: AutoAcceptPolicyPatch = serde_json::from_value(json!({"enabled": true})).unwrap();
        assert_eq!(p.enabled, Some(true));
        assert!(matches!(p.max_file_size, Patch::Leave));
        // A JSON null clears the cap.
        let p: AutoAcceptPolicyPatch =
            serde_json::from_value(json!({"max_file_size": null})).unwrap();
        assert!(matches!(p.max_file_size, Patch::Clear));
        // Unknown fields rejected.
        assert!(serde_json::from_value::<AutoAcceptPolicyPatch>(json!({"z": 1})).is_err());
        // Clean body (sparse).
        let b: StorageCleanBody = serde_json::from_value(json!({})).unwrap();
        assert_eq!(b.orphaned_only, None);
        assert!(serde_json::from_value::<StorageCleanBody>(json!({"z": 1})).is_err());
    }

    #[test]
    fn clip_domain_reports_round_trip() {
        // §163: clipboard resources fold the legacy `ok` envelope off and
        // flatten the nested policy; the CLI printers read the flat shape.
        let pol = ClipPolicyDto {
            mode: "bidirectional_auto".into(),
            sensitive_guard: true,
            allowed_groups: vec!["lab".into()],
            allowed_tags: Vec::new(),
            allowed_types: vec!["desktop".into()],
            max_payload_bytes: 65536,
            auto_clear_timeout_secs: Some(30),
            auto_synced_out_count: 4,
            auto_synced_in_count: 2,
            sensitive_blocked_count: 1,
        };
        let v = serde_json::to_value(&pol).unwrap();
        assert_eq!(v["mode"], "bidirectional_auto");
        assert_eq!(v["max_payload_bytes"], 65536);
        let back: ClipPolicyDto = serde_json::from_value(v).unwrap();
        assert_eq!(back, pol);

        // Patch is sparse and strict.
        let p: ClipPolicyPatch = serde_json::from_value(json!({"mode": "send_only"})).unwrap();
        assert_eq!(p.mode.as_deref(), Some("send_only"));
        assert_eq!(p.sensitive_guard, None);
        assert!(serde_json::from_value::<ClipPolicyPatch>(json!({"z": 1})).is_err());

        // Poll report nests the same flat policy (count, not a list).
        let poll = ClipPollReport {
            dispatched_peers: 1,
            policy: pol.clone(),
        };
        let back_p: ClipPollReport =
            serde_json::from_value(serde_json::to_value(&poll).unwrap()).unwrap();
        assert_eq!(back_p, poll);

        // Status: hash nulls when the clipboard is empty.
        let st = ClipStatus {
            has_text: false,
            bytes: 0,
            hash: None,
            last_seq: 9,
            inbound_count: 3,
            outbound_count: 6,
            history_count: 5,
            clipboard_available: true,
            reason: None,
        };
        assert!(serde_json::to_value(&st).unwrap()["hash"].is_null());
        let back_s: ClipStatus =
            serde_json::from_value(serde_json::to_value(&st).unwrap()).unwrap();
        assert_eq!(back_s, st);

        // History entry + report.
        let e = ClipHistoryEntry {
            seq: 1,
            source: "local".into(),
            direction: "outbound".into(),
            bytes: 12,
            hash: "abc".into(),
            timestamp_ms: 1000,
        };
        let hist = ClipHistory {
            total: 1,
            entries: vec![e],
        };
        let back_h: ClipHistory =
            serde_json::from_value(serde_json::to_value(&hist).unwrap()).unwrap();
        assert_eq!(back_h, hist);

        // Clear report: reason nulls when a display server exists.
        let clr = ClipClearReport {
            cleared_history_count: 5,
            timestamp_ms: 2000,
            clipboard_available: true,
            reason: None,
        };
        let back_c: ClipClearReport =
            serde_json::from_value(serde_json::to_value(&clr).unwrap()).unwrap();
        assert_eq!(back_c, clr);

        // Content: the one plaintext readback; reason only on headless.
        let content = ClipContent {
            has_text: true,
            bytes: 12,
            hash: Some("abc".into()),
            text: Some("hello clipboard".into()),
            clipboard_available: false,
            reason: Some("no display server".into()),
        };
        let back_g: ClipContent =
            serde_json::from_value(serde_json::to_value(&content).unwrap()).unwrap();
        assert_eq!(back_g, content);

        // Sync request + report.
        let q: ClipSyncRequest = serde_json::from_value(json!({"text": "hi"})).unwrap();
        assert_eq!(q.peer, None);
        assert!(serde_json::from_value::<ClipSyncRequest>(json!({"z": 1})).is_err());
        let rep = ClipSyncReport {
            bytes: 2,
            hash: "def".into(),
            sent_peers: 1,
        };
        let back_r: ClipSyncReport =
            serde_json::from_value(serde_json::to_value(&rep).unwrap()).unwrap();
        assert_eq!(back_r, rep);
    }

    #[test]
    fn audit_domain_reports_round_trip() {
        // §163: the audit trail is a filtered view over the ring; entries
        // carry the category/severity as snake_case strings.
        let e = AuditEntryDto {
            id: 7,
            timestamp_ms: 1000,
            category: "peer".into(),
            severity: "warning".into(),
            action: "pair.rejected".into(),
            target: Some("peer-a".into()),
            summary: "pin mismatch".into(),
            success: false,
        };
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["category"], "peer");
        assert_eq!(v["severity"], "warning");
        assert!(!v["target"].is_null());
        let back_e: AuditEntryDto = serde_json::from_value(v).unwrap();
        assert_eq!(back_e, e);

        let trail = AuditTrail {
            total: 1,
            returned: 1,
            capacity: 256,
            entries: vec![e],
        };
        let back_t: AuditTrail =
            serde_json::from_value(serde_json::to_value(&trail).unwrap()).unwrap();
        assert_eq!(back_t, trail);

        // Query body mirrors the query params, strict.
        let q: AuditQuery =
            serde_json::from_value(json!({"severity": "alert", "limit": 5})).unwrap();
        assert_eq!(q.severity.as_deref(), Some("alert"));
        assert_eq!(q.limit, Some(5));
        assert!(serde_json::from_value::<AuditQuery>(json!({"z": 1})).is_err());

        let clr = AuditClearReport {
            cleared_count: 9,
            message: "audit log cleared".into(),
        };
        let back_c: AuditClearReport =
            serde_json::from_value(serde_json::to_value(&clr).unwrap()).unwrap();
        assert_eq!(back_c, clr);
    }
    #[test]
    fn action_domain_reports_round_trip() {
        // §164: action resources mirror the legacy /api/action reports
        // field-for-field; CLI printers depend on the exact shapes.

        let def = LocalActionDto {
            id: "backup".into(),
            name: "Run backup".into(),
            description: Some("nightly backup".into()),
            icon: None,
            command: "/usr/local/bin/backup".into(),
            args: vec!["--full".into()],
            timeout_secs: 30,
            enabled: true,
        };
        let v = serde_json::to_value(&def).unwrap();
        assert_eq!(v["command"], "/usr/local/bin/backup");
        assert!(v["icon"].is_null());
        let back_d: LocalActionDto = serde_json::from_value(v).unwrap();
        assert_eq!(back_d, def);

        let local = LocalActions {
            count: 1,
            actions: vec![def],
        };
        let back_l: LocalActions =
            serde_json::from_value(serde_json::to_value(&local).unwrap()).unwrap();
        assert_eq!(back_l, local);

        let item = RemoteActionDto {
            id: "screenshot".into(),
            name: "Screenshot".into(),
            description: None,
            icon: Some("camera".into()),
        };
        let v = serde_json::to_value(&item).unwrap();
        assert!(v["description"].is_null());
        let remote = RemoteActions {
            peer: "peer-a".into(),
            count: 1,
            actions: vec![item],
        };
        let back_r: RemoteActions =
            serde_json::from_value(serde_json::to_value(&remote).unwrap()).unwrap();
        assert_eq!(back_r, remote);

        // Registration request is strict; optional fields stay sparse.
        let req: LocalActionRequest = serde_json::from_value(json!({
            "id": "backup",
            "name": "Run backup",
            "command": "/usr/local/bin/backup",
        }))
        .unwrap();
        assert_eq!(req.timeout_secs, None);
        assert!(serde_json::from_value::<LocalActionRequest>(json!({"z": 1})).is_err());

        let trig_req: ActionTriggerRequest =
            serde_json::from_value(json!({"peer": "peer-a", "action_id": "screenshot"})).unwrap();
        assert_eq!(trig_req.action_id, "screenshot");
        assert!(serde_json::from_value::<ActionTriggerRequest>(json!({"peer": "p"})).is_err());

        let trig = ActionTriggerReport {
            trigger_id: "tg-1".into(),
            action_id: "screenshot".into(),
            target_peer: "peer-a".into(),
            status: "dispatched".into(),
        };
        let back_t: ActionTriggerReport =
            serde_json::from_value(serde_json::to_value(&trig).unwrap()).unwrap();
        assert_eq!(back_t, trig);

        let entry = ActionHistoryEntry {
            trigger_id: "tg-1".into(),
            action_id: "screenshot".into(),
            success: true,
            exit_code: Some(0),
            output_summary: Some("ok".into()),
            error_message: None,
            duration_ms: 120,
            timestamp_ms: 1000,
        };
        let v = serde_json::to_value(&entry).unwrap();
        assert!(v["error_message"].is_null());
        let history = ActionHistory {
            count: 1,
            results: vec![entry],
        };
        let back_h: ActionHistory =
            serde_json::from_value(serde_json::to_value(&history).unwrap()).unwrap();
        assert_eq!(back_h, history);

        // Policy resource is flat; the patch is strict and sparse.
        let pol = ActionPolicyDto {
            mode: "prompt_only".into(),
            max_timeout_secs: 30,
            allowed_groups: vec!["ops".into()],
            allowed_tags: vec![],
            allowed_types: vec!["laptop".into()],
            expose_action_list: true,
        };
        let v = serde_json::to_value(&pol).unwrap();
        assert_eq!(v["mode"], "prompt_only");
        assert_eq!(v["allowed_types"][0], "laptop");
        let back_p: ActionPolicyDto = serde_json::from_value(v).unwrap();
        assert_eq!(back_p, pol);

        let patch: ActionPolicyPatch =
            serde_json::from_value(json!({"mode": "allow", "clear_filters": true})).unwrap();
        assert_eq!(patch.mode.as_deref(), Some("allow"));
        assert_eq!(patch.clear_filters, Some(true));
        assert_eq!(patch.max_timeout_secs, None);
        assert!(serde_json::from_value::<ActionPolicyPatch>(json!({"z": 1})).is_err());
    }

    #[test]
    fn bench_domain_reports_round_trip() {
        // §164: bench resources mirror the legacy /api/bench + /api/recommendation
        // reports field-for-field; CLI printers depend on the exact shapes.

        let sess = ActiveBenchSessionDto {
            session_id: "s1".into(),
            peer_id: "peer-a".into(),
            role: "initiator".into(),
            direction: "push".into(),
            duration_secs: 10,
            chunk_size: 65536,
            chunks_processed: 3,
            bytes_transferred: 196608,
            started_at_ms: 1000,
            last_activity_ms: 2000,
        };
        let v = serde_json::to_value(&sess).unwrap();
        assert_eq!(v["role"], "initiator");
        assert_eq!(v["direction"], "push");
        let back_s: ActiveBenchSessionDto = serde_json::from_value(v).unwrap();
        assert_eq!(back_s, sess);

        let rep = BenchReportDto {
            session_id: "s1".into(),
            direction: "pull".into(),
            duration_ms: 10050,
            bytes_transferred: 1048576,
            chunks: 16,
            throughput_mbps: 834.5,
            avg_chunk_latency_us: 620,
            jitter_us: 40,
            stopped_early: Some(false),
            timestamp_ms: 11000,
        };
        let v = serde_json::to_value(&rep).unwrap();
        assert_eq!(v["throughput_mbps"], 834.5);
        let back_r: BenchReportDto = serde_json::from_value(v).unwrap();
        assert_eq!(back_r, rep);

        let pol = BenchPolicyDto {
            enabled: true,
            allow_inbound_bench: false,
            max_duration_secs: 30,
            max_bandwidth_mbps: 1000,
            allowed_groups: vec![],
            allowed_tags: vec![],
            allowed_types: vec!["desktop".into()],
        };
        let status = BenchStatus {
            active: true,
            session: Some(sess),
            policy: pol.clone(),
            last_report: Some(rep),
            timestamp_ms: 11000,
        };
        let v = serde_json::to_value(&status).unwrap();
        assert_eq!(v["active"], true);
        assert_eq!(v["policy"]["max_bandwidth_mbps"], 1000);
        let back_st: BenchStatus = serde_json::from_value(v).unwrap();
        assert_eq!(back_st, status);

        // start request defaults direction to push and is strict.
        let req: BenchStartRequest = serde_json::from_value(json!({"peer": "peer-a"})).unwrap();
        assert_eq!(req.direction, "push");
        assert_eq!(req.duration_secs, None);
        let req2: BenchStartRequest = serde_json::from_value(
            json!({"peer": "peer-a", "direction": "pull", "duration_secs": 5, "chunk_size": 32768}),
        )
        .unwrap();
        assert_eq!(req2.chunk_size, Some(32768));
        assert!(serde_json::from_value::<BenchStartRequest>(json!({"z": 1})).is_err());

        let dec = BenchDecision {
            action: "stop".into(),
            session_id: Some("s1".into()),
            peer_id: None,
            message: "benchmark stopped".into(),
            timestamp_ms: 12000,
        };
        let v = serde_json::to_value(&dec).unwrap();
        assert!(v["peer_id"].is_null());
        let back_d: BenchDecision = serde_json::from_value(v).unwrap();
        assert_eq!(back_d, dec);

        // Policy patch is strict and sparse.
        let patch: BenchPolicyPatch =
            serde_json::from_value(json!({"enabled": false, "clear_filters": true})).unwrap();
        assert_eq!(patch.enabled, Some(false));
        assert_eq!(patch.max_duration_secs, None);
        assert!(serde_json::from_value::<BenchPolicyPatch>(json!({"z": 1})).is_err());

        // Recommendation tiers: nested DTOs with codec strings.
        let rec = StreamingRecommendationDto {
            measured_mbps: 940.0,
            tier: "flagship".into(),
            display: DisplayTierDto {
                width: 3840,
                height: 2160,
                fps: 60,
            },
            camera: CameraTierDto {
                width: 1920,
                height: 1080,
                fps: 60,
                codec: "h264".into(),
            },
            audio: AudioTierDto {
                sample_rate: 48000,
                channels: 2,
                codec: "opus".into(),
            },
            mic: MicTierDto {
                sample_rate: 48000,
                channels: 1,
                codec: "opus".into(),
            },
        };
        let one = RecommendationDto {
            peer_id: "peer-a".into(),
            peer_alias: "Office Mac".into(),
            has_profile: true,
            measured_mbps: 940.0,
            direction: "push".into(),
            measured_at_ms: 9000,
            recommendation: rec.clone(),
            timestamp_ms: 9000,
        };
        let v = serde_json::to_value(&one).unwrap();
        assert_eq!(v["recommendation"]["tier"], "flagship");
        assert_eq!(v["recommendation"]["camera"]["codec"], "h264");
        let back_o: RecommendationDto = serde_json::from_value(v).unwrap();
        assert_eq!(back_o, one);

        let agg = RecommendationStatus {
            peers: vec![RecommendationSummaryDto {
                peer_id: "peer-a".into(),
                peer_alias: "Office Mac".into(),
                measured_mbps: 940.0,
                direction: "push".into(),
                measured_at_ms: 9000,
                tier: "flagship".into(),
            }],
            timestamp_ms: 9000,
        };
        let back_a: RecommendationStatus =
            serde_json::from_value(serde_json::to_value(&agg).unwrap()).unwrap();
        assert_eq!(back_a, agg);
    }

    #[test]
    fn power_domain_reports_round_trip() {
        // §165a: power resources mirror the legacy /api/power reports
        // field-for-field; CLI printers depend on the exact shapes.

        let st = BatteryStateDto {
            battery_level: 76,
            is_charging: false,
            power_source: "battery".into(),
            battery_saver: false,
            health_percent: Some(92),
            timestamp_ms: 1000,
        };
        let v = serde_json::to_value(&st).unwrap();
        assert_eq!(v["power_source"], "battery");
        assert_eq!(v["health_percent"], 92);
        let back_s: BatteryStateDto = serde_json::from_value(v).unwrap();
        assert_eq!(back_s, st);

        let local = BatteryState {
            peer: None,
            state: Some(st),
        };
        let v = serde_json::to_value(&local).unwrap();
        assert!(v["peer"].is_null());
        let back_l: BatteryState = serde_json::from_value(v).unwrap();
        assert_eq!(back_l, local);

        // Sparse patch is strict; unknown fields rejected.
        let patch: BatteryStatePatch =
            serde_json::from_value(json!({"battery_level": 55, "is_charging": true})).unwrap();
        assert_eq!(patch.battery_level, Some(55));
        assert_eq!(patch.is_charging, Some(true));
        assert_eq!(patch.power_source, None);
        assert!(serde_json::from_value::<BatteryStatePatch>(json!({"z": 1})).is_err());

        let pol = BatteryPolicyDto {
            low_battery_threshold: 30,
            critical_battery_threshold: 15,
            alert_on_low_battery: true,
            auto_throttle_on_battery_saver: false,
            allowed_groups: vec!["ops".into()],
            allowed_tags: vec![],
            allowed_types: vec!["laptop".into()],
        };
        let v = serde_json::to_value(&pol).unwrap();
        assert_eq!(v["low_battery_threshold"], 30);
        assert_eq!(v["allowed_types"][0], "laptop");
        let back_p: BatteryPolicyDto = serde_json::from_value(v).unwrap();
        assert_eq!(back_p, pol);

        let ppatch: BatteryPolicyPatch = serde_json::from_value(json!({
            "low_battery_threshold": 25,
            "auto_throttle_on_battery_saver": true,
        }))
        .unwrap();
        assert_eq!(ppatch.low_battery_threshold, Some(25));
        assert_eq!(ppatch.alert_on_low_battery, None);
        assert!(serde_json::from_value::<BatteryPolicyPatch>(json!({"z": 1})).is_err());
    }

    #[test]
    fn media_domain_reports_round_trip() {
        // §165a: media resources mirror the legacy /api/media reports
        // field-for-field; CLI printers depend on the exact shapes.

        // Request is strict + sparse; command is the normalized wire string.
        let req: MediaCommandRequest = serde_json::from_value(json!({
            "peer": "peer-a",
            "command": "volume",
            "volume": 40,
        }))
        .unwrap();
        assert_eq!(req.peer.as_deref(), Some("peer-a"));
        assert_eq!(req.volume, Some(40));
        assert_eq!(req.offset_ms, None);
        assert!(serde_json::from_value::<MediaCommandRequest>(json!({"z": 1})).is_err());

        let rep = MediaCommandReport {
            id: "c1".into(),
            peer: "peer-a".into(),
            peer_alias: Some("Laptop".into()),
            command: "seek:-30000".into(),
            target_player: None,
            timestamp_ms: 1000,
        };
        let v = serde_json::to_value(&rep).unwrap();
        assert!(v["target_player"].is_null());
        let back_r: MediaCommandReport = serde_json::from_value(v).unwrap();
        assert_eq!(back_r, rep);

        let multi = MediaCommandMultiReport {
            selector: "tag:kiosk".into(),
            command: "play".into(),
            target_count: 2,
            success_count: 1,
            skipped_offline_count: 1,
            failed_count: 0,
            dispatches: vec![MediaDispatchItem {
                peer: "peer-a".into(),
                peer_alias: None,
                command_id: Some("c1".into()),
                success: true,
                error: None,
            }],
        };
        let v = serde_json::to_value(&multi).unwrap();
        assert_eq!(v["success_count"], 1);
        assert_eq!(v["skipped_offline_count"], 1);
        let back_m: MediaCommandMultiReport = serde_json::from_value(v).unwrap();
        assert_eq!(back_m, multi);

        let frame = MediaStateDto {
            player_id: "spotify".into(),
            state: "playing".into(),
            title: "Song".into(),
            artist: "Artist".into(),
            album: "".into(),
            position_ms: 30_000,
            duration_ms: 200_000,
            volume_percent: 55,
            muted: false,
            can_play: true,
            can_pause: true,
            can_go_next: true,
            can_go_previous: false,
            can_seek: true,
            timestamp_ms: 1000,
        };
        let v = serde_json::to_value(&frame).unwrap();
        assert_eq!(v["state"], "playing");
        let back_f: MediaStateDto = serde_json::from_value(v).unwrap();
        assert_eq!(back_f, frame);

        let st = MediaState {
            peer: None,
            state: Some(frame.clone()),
        };
        let back_s: MediaState =
            serde_json::from_value(serde_json::to_value(&st).unwrap()).unwrap();
        assert_eq!(back_s, st);

        let upd = MediaStateUpdateReport { state: frame };
        let back_u: MediaStateUpdateReport =
            serde_json::from_value(serde_json::to_value(&upd).unwrap()).unwrap();
        assert_eq!(back_u, upd);

        // Sparse patch is strict; unknown fields rejected.
        let up: MediaStatePatch =
            serde_json::from_value(json!({"volume_percent": 70, "muted": true})).unwrap();
        assert_eq!(up.volume_percent, Some(70));
        assert_eq!(up.muted, Some(true));
        assert_eq!(up.position_ms, None);
        assert!(serde_json::from_value::<MediaStatePatch>(json!({"z": 1})).is_err());

        let pol = MediaPolicyDto {
            mode: "allow".into(),
            allowed_groups: vec!["media".into()],
            allowed_tags: vec![],
            allowed_types: vec!["desktop".into()],
            max_volume_limit: 80,
            allow_seek: true,
        };
        let v = serde_json::to_value(&pol).unwrap();
        assert_eq!(v["max_volume_limit"], 80);
        let back_p: MediaPolicyDto = serde_json::from_value(v).unwrap();
        assert_eq!(back_p, pol);

        let ppatch: MediaPolicyPatch =
            serde_json::from_value(json!({"mode": "deny", "clear_filters": true})).unwrap();
        assert_eq!(ppatch.mode.as_deref(), Some("deny"));
        assert_eq!(ppatch.allowed_groups, None);
        assert!(serde_json::from_value::<MediaPolicyPatch>(json!({"z": 1})).is_err());
    }

    #[test]
    fn ancs_domain_reports_round_trip() {
        // §165a: ANCS resources mirror the legacy /api/ancs reports
        // field-for-field; CLI printers depend on the exact shapes.

        let rule = AncsRule {
            app_id: "com.apple.mail".into(),
            url: String::new(),
        };
        let v = serde_json::to_value(&rule).unwrap();
        assert_eq!(v["url"], "");
        let back_r: AncsRule = serde_json::from_value(v).unwrap();
        assert_eq!(back_r, rule);

        let st = AncsStatus {
            attached: true,
            state: "connected".into(),
            peripheral: Some("iPhone".into()),
            retry_at_ms: None,
            rules: vec![rule.clone()],
            allowlist: vec!["com.apple.mail".into()],
        };
        let v = serde_json::to_value(&st).unwrap();
        assert_eq!(v["state"], "connected");
        assert_eq!(v["rules"][0]["app_id"], "com.apple.mail");
        assert!(v["retry_at_ms"].is_null());
        let back_s: AncsStatus = serde_json::from_value(v).unwrap();
        assert_eq!(back_s, st);

        let conn = AncsConnectReport {
            state: "connected".into(),
            retry_at_ms: None,
            message: "ancs attached".into(),
        };
        let v = serde_json::to_value(&conn).unwrap();
        assert!(v["retry_at_ms"].is_null());
        let back_c: AncsConnectReport = serde_json::from_value(v).unwrap();
        assert_eq!(back_c, conn);

        let disc = AncsDisconnectReport {
            state: "disconnected".into(),
            retry_at_ms: Some(2000),
            message: "ancs detached".into(),
        };
        let back_d: AncsDisconnectReport =
            serde_json::from_value(serde_json::to_value(&disc).unwrap()).unwrap();
        assert_eq!(back_d, disc);

        // Allow request: url defaults to empty; app_id required.
        let req: AncsAllowRequest =
            serde_json::from_value(json!({"app_id": "com.apple.mail"})).unwrap();
        assert_eq!(req.url, "");
        assert!(serde_json::from_value::<AncsAllowRequest>(json!({"url": "x"})).is_err());

        let rep = AncsRuleReport {
            action: "add".into(),
            rule: Some(rule),
            total_rules: 1,
            timestamp_ms: 1000,
        };
        let v = serde_json::to_value(&rep).unwrap();
        assert_eq!(v["action"], "add");
        let back_rp: AncsRuleReport = serde_json::from_value(v).unwrap();
        assert_eq!(back_rp, rep);

        let clr = AncsClearRulesReport {
            cleared_count: 3,
            total_rules: 0,
            timestamp_ms: 1000,
        };
        let back_cl: AncsClearRulesReport =
            serde_json::from_value(serde_json::to_value(&clr).unwrap()).unwrap();
        assert_eq!(back_cl, clr);

        // Remove request: sparse + strict (query `app_id` preferred on the wire).
        let rm: AncsRemoveRequest =
            serde_json::from_value(json!({"app_id": "com.apple.mail"})).unwrap();
        assert_eq!(rm.app_id.as_deref(), Some("com.apple.mail"));
        assert!(serde_json::from_value::<AncsRemoveRequest>(json!({"z": 1})).is_err());
    }
}

#[test]
fn screen_domain_reports_round_trip() {
    use serde_json::json;
    use tetherly_api::{
        AnnotationItemDto, AnnotationKindDto, ScreenColorPatch, ScreenSnapshot, ScreenTileBatch,
        ScreenWatermarkPatch,
    };

    // Tile batch report: `ok` folded, plain counters survive the round trip.
    let tb = ScreenTileBatch {
        enabled: true,
        target_chunk_bytes: 1200,
        pacing_interval_us: 200,
        batches_sent: 9,
        chunks_sent: 41,
        avg_chunk_bytes: 1180,
        pacing_pauses: 2,
    };
    let back: ScreenTileBatch = serde_json::from_value(serde_json::to_value(&tb).unwrap()).unwrap();
    assert_eq!(back, tb);

    // Snapshot: Option fields serialize explicitly (null when None).
    let sn = ScreenSnapshot {
        source: "live".into(),
        frame_seq: 7,
        pts_us: 123,
        width: 1920,
        height: 1080,
        format: "raw".into(),
        bytes_len: 4096,
        privacy_blanked: false,
        watermarked: true,
        path: None,
        data_base64: None,
    };
    let v = serde_json::to_value(&sn).unwrap();
    assert_eq!(v["path"], serde_json::Value::Null);
    let back: ScreenSnapshot = serde_json::from_value(v).unwrap();
    assert_eq!(back, sn);

    // Annotation item: tagged kind mirror survives the round trip.
    let item = AnnotationItemDto {
        id: 1,
        author: "alice".into(),
        kind: AnnotationKindDto::Stroke {
            points: vec![(1, 2), (3, 4)],
        },
        color_rgb: [255, 59, 48],
        thickness: 3,
        alpha: 230,
        ttl_ms: 0,
        created_at_ms: 42,
    };
    let v = serde_json::to_value(&item).unwrap();
    assert_eq!(v["kind"]["type"], "stroke");
    let back: AnnotationItemDto = serde_json::from_value(v).unwrap();
    assert_eq!(back, item);

    // Patches: sparse + strict (unknown fields rejected).
    let cp: ScreenColorPatch = serde_json::from_value(json!({"mode": "reduced"})).unwrap();
    assert_eq!(cp.mode.as_deref(), Some("reduced"));
    assert!(serde_json::from_value::<ScreenColorPatch>(json!({"z": 1})).is_err());
    let wp: ScreenWatermarkPatch = serde_json::from_value(json!({"opacity": 40})).unwrap();
    assert_eq!(wp.opacity, Some(40));
}
