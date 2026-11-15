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
