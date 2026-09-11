// SPDX-License-Identifier: GPL-3.0-or-later
//! The framing layer, checked against frames the Python daemon actually emits.
//!
//! `contract/rpc-frames.json` is produced by `tools/gen_rpc_frames.py` running
//! deluge's own transfer code path, so these are the exact bytes that go over
//! the wire, not a reimplementation of them.

use redeluge_rencode::Value;
use redeluge_rpc::transfer::{Error, HEADER_SIZE};
use redeluge_rpc::{decode_body, encode_frame, FrameReader, Incoming, Limits, PROTOCOL_VERSION};
use serde_json::Value as Json;

const FRAMES: &str = include_str!("../../../contract/rpc-frames.json");

fn hex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

fn document() -> Json {
    serde_json::from_str(FRAMES).expect("the frame corpus is not valid JSON")
}

fn real_frames() -> Vec<(String, Vec<u8>)> {
    document()["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["description"].as_str().unwrap().to_owned(),
                hex(entry["frame"].as_str().unwrap()),
            )
        })
        .collect()
}

fn compression_bomb() -> Vec<u8> {
    hex(document()["compression_bomb"]["frame"].as_str().unwrap())
}

#[test]
fn every_frame_the_daemon_sends_can_be_read() {
    for (description, frame) in real_frames() {
        let mut reader = FrameReader::default();
        reader.feed(&frame);
        let message = reader
            .next_message()
            .unwrap_or_else(|err| panic!("`{description}` failed to decode: {err}"))
            .unwrap_or_else(|| panic!("`{description}` did not yield a complete message"));

        // A frame holds exactly one message and nothing is left over.
        assert!(reader.next_message().unwrap().is_none());
        assert_eq!(reader.buffered(), 0, "`{description}` left bytes behind");
        assert!(matches!(message, Value::List(_) | Value::Dict(_)));
    }
}

#[test]
fn frames_arriving_one_byte_at_a_time_still_reassemble() {
    // TLS records do not respect message boundaries, so this is the normal case
    // rather than an edge case.
    for (description, frame) in real_frames() {
        let mut reader = FrameReader::default();
        let mut completed = 0;

        for (index, byte) in frame.iter().enumerate() {
            reader.feed(&[*byte]);
            match reader.next_message().unwrap() {
                Some(_) => {
                    completed += 1;
                    assert_eq!(
                        index,
                        frame.len() - 1,
                        "`{description}` completed before its last byte"
                    );
                }
                None => assert!(index < frame.len() - 1, "`{description}` never completed"),
            }
        }
        assert_eq!(completed, 1, "`{description}` yielded {completed} messages");
    }
}

#[test]
fn several_frames_in_one_chunk_come_out_in_order() {
    let frames = real_frames();
    let mut joined = Vec::new();
    for (_, frame) in &frames {
        joined.extend_from_slice(frame);
    }

    let mut reader = FrameReader::default();
    reader.feed(&joined);

    let mut count = 0;
    while reader.next_message().unwrap().is_some() {
        count += 1;
    }
    assert_eq!(count, frames.len());
    assert_eq!(reader.buffered(), 0);
}

#[test]
fn a_compression_bomb_is_refused_instead_of_inflated() {
    // Two hundred kilobytes on the wire, two hundred megabytes once inflated.
    // The Python daemon inflates it, before authentication. This does not.
    let bomb = compression_bomb();
    let inflated = document()["compression_bomb"]["inflated_size"]
        .as_u64()
        .unwrap();

    assert!(
        bomb.len() < 1024 * 1024,
        "the bomb should be small on the wire"
    );
    assert!(inflated > 100 * 1024 * 1024);

    let limits = Limits {
        max_frame: 16 * 1024 * 1024,
        // Deliberately below the inflated size, as any sane limit would be.
        max_body: 8 * 1024 * 1024,
    };

    let mut reader = FrameReader::new(limits);
    reader.feed(&bomb);

    match reader.next_message() {
        Err(Error::BodyTooLarge { limit }) => assert_eq!(limit, limits.max_body),
        other => panic!("the bomb should have been refused, got {other:?}"),
    }
}

#[test]
fn an_oversized_frame_is_refused_from_its_header_alone() {
    // Five bytes claiming a four gigabyte body. The check has to happen here,
    // before anything is reserved, or the claim itself is the attack.
    let mut header = vec![PROTOCOL_VERSION];
    header.extend_from_slice(&u32::MAX.to_be_bytes());

    let limits = Limits::default();
    let mut reader = FrameReader::new(limits);
    reader.feed(&header);

    match reader.next_message() {
        Err(Error::FrameTooLarge { size, limit }) => {
            assert_eq!(size, u32::MAX as usize);
            assert_eq!(limit, limits.max_frame);
        }
        other => panic!("expected FrameTooLarge, got {other:?}"),
    }
    assert!(
        reader.buffered() <= HEADER_SIZE,
        "nothing should have been buffered for the claimed body"
    );
}

#[test]
fn a_wrong_protocol_version_is_refused() {
    let mut frame = real_frames()[0].1.clone();
    frame[0] = 9;

    let mut reader = FrameReader::default();
    reader.feed(&frame);
    match reader.next_message() {
        Err(Error::UnsupportedVersion { found }) => assert_eq!(found, 9),
        other => panic!("expected UnsupportedVersion, got {other:?}"),
    }
}

#[test]
fn a_corrupt_body_is_an_error_not_a_panic() {
    for (_, frame) in real_frames() {
        for index in HEADER_SIZE..frame.len() {
            let mut damaged = frame.clone();
            damaged[index] ^= 0xff;

            let mut reader = FrameReader::default();
            reader.feed(&damaged);
            let _ = reader.next_message();
        }
    }
}

#[test]
fn what_we_encode_we_can_read_back() {
    let request = Value::List(vec![Value::List(vec![
        Value::Int(0),
        Value::Str("daemon.login".into()),
        Value::List(vec![
            Value::Str("localclient".into()),
            Value::Str("pw".into()),
        ]),
        Value::Dict(vec![(
            Value::Str("client_version".into()),
            Value::Str("2.2.1".into()),
        )]),
    ])]);

    let frame = encode_frame(&request).unwrap();
    assert_eq!(frame[0], PROTOCOL_VERSION);

    let mut reader = FrameReader::default();
    reader.feed(&frame);
    assert_eq!(reader.next_message().unwrap().unwrap(), request);
}

#[test]
fn a_body_that_is_not_zlib_is_an_error() {
    assert!(matches!(
        decode_body(b"not compressed at all", Limits::default()),
        Err(Error::Decompress(_))
    ));
}

#[test]
fn the_real_messages_parse_into_their_shapes() {
    let by_description: Vec<(String, Vec<u8>)> = real_frames();

    let read = |name: &str| -> Incoming {
        let (_, frame) = by_description
            .iter()
            .find(|(description, _)| description == name)
            .unwrap_or_else(|| panic!("no frame called `{name}`"));
        let mut reader = FrameReader::default();
        reader.feed(frame);
        let value = reader.next_message().unwrap().unwrap();
        Incoming::from_value(&value).unwrap()
    };

    match read("response") {
        Incoming::Response { id, result } => {
            assert_eq!(id, 0);
            assert_eq!(result.as_str(), Some("2.2.1"));
        }
        other => panic!("expected a response, got {other:?}"),
    }

    match read("error with traceback") {
        Incoming::Error { id, failure } => {
            assert_eq!(id, 0);
            assert_eq!(failure.exception, "BadLoginError");
            assert_eq!(failure.message(), Some("Password does not match"));
            assert!(failure.traceback.is_some(), "the daemon did send one");
            // The traceback must not reach a user through Display.
            let rendered = failure.to_string();
            assert!(!rendered.contains("Traceback"), "leaked: {rendered}");
            assert!(!rendered.contains("rpcserver.py"), "leaked: {rendered}");
        }
        other => panic!("expected an error, got {other:?}"),
    }

    match read("event") {
        Incoming::Event { name, args } => {
            assert_eq!(name, "TorrentAddedEvent");
            assert_eq!(args.len(), 2);
            assert_eq!(args[1], Value::Bool(false));
        }
        other => panic!("expected an event, got {other:?}"),
    }

    match read("event without args") {
        Incoming::Event { name, args } => {
            assert_eq!(name, "SessionResumedEvent");
            assert!(args.is_empty());
        }
        other => panic!("expected an event, got {other:?}"),
    }
}

#[test]
fn a_message_that_is_not_one_of_the_four_shapes_is_rejected() {
    for bad in [
        Value::Int(7),
        Value::List(vec![]),
        Value::List(vec![Value::Int(99), Value::Int(0)]),
        Value::List(vec![Value::Int(1)]),
        Value::List(vec![Value::Int(3)]),
    ] {
        assert!(
            Incoming::from_value(&bad).is_err(),
            "{bad:?} should not parse"
        );
    }
}
