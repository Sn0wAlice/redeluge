// SPDX-License-Identifier: GPL-3.0-or-later
//! The decoder runs on bytes an unauthenticated peer chose.
//!
//! The daemon accepts and decodes a message before it knows who is speaking, so
//! every failure mode here is remotely reachable. None of it may panic, none of
//! it may allocate on the peer's say-so, and none of it may recurse without a
//! bound. The reference implementation does none of these things, which is why
//! the Rust decoder deliberately diverges.

use redeluge_rencode::{codes, from_slice, to_vec, Error, StringMode, Value, MAX_DEPTH};
use serde_json::Value as Json;

const CORPUS: &str = include_str!("../../../contract/rencode-corpus.json");

fn corpus_encodings() -> Vec<Vec<u8>> {
    let parsed: Json = serde_json::from_str(CORPUS).unwrap();
    parsed["cases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| {
            let text = case["encoded"].as_str().unwrap();
            (0..text.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
                .collect()
        })
        .collect()
}

/// Deterministic noise, so a failure is reproducible without a fuzzer.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn byte(&mut self) -> u8 {
        (self.next() >> 24) as u8
    }
}

#[test]
fn every_truncation_of_valid_input_is_handled() {
    for encoding in corpus_encodings() {
        for length in 0..encoding.len() {
            let result = from_slice(&encoding[..length], StringMode::Utf8Lossy);
            // A prefix may happen to be a complete smaller value, which is fine.
            // What matters is that it never panics and never claims success on
            // something it did not fully read.
            if let Ok(value) = result {
                assert_eq!(
                    to_vec(&value).len(),
                    length,
                    "a truncated input decoded to a value that is not the whole input"
                );
            }
        }
    }
}

#[test]
fn every_single_byte_corruption_is_handled() {
    let mut rng = Rng(0x5eed_1234_abcd_0001);
    for encoding in corpus_encodings() {
        for index in 0..encoding.len() {
            let mut damaged = encoding.clone();
            damaged[index] = rng.byte();
            // Result does not matter; not panicking does.
            let _ = from_slice(&damaged, StringMode::Utf8Lossy);
            let _ = from_slice(&damaged, StringMode::Raw);
        }
    }
}

#[test]
fn random_bytes_never_panic() {
    let mut rng = Rng(0x1234_5678_9abc_def1);
    for length in [0usize, 1, 2, 3, 7, 16, 64, 255, 1024] {
        for _ in 0..200 {
            let noise: Vec<u8> = (0..length).map(|_| rng.byte()).collect();
            let _ = from_slice(&noise, StringMode::Utf8Lossy);
        }
    }
}

#[test]
fn deep_nesting_is_refused_rather_than_overflowing_the_stack() {
    // A few hundred bytes of nothing but list markers is enough to blow the
    // stack of a decoder that recurses without a limit.
    let mut bomb = vec![codes::LIST_FIXED_START + 1; MAX_DEPTH + 50];
    bomb.push(codes::CHR_NONE);

    match from_slice(&bomb, StringMode::Utf8Lossy) {
        Err(Error::TooDeep { limit, .. }) => assert_eq!(limit, MAX_DEPTH),
        other => panic!("expected TooDeep, got {other:?}"),
    }
}

#[test]
fn nesting_just_within_the_limit_still_works() {
    let mut nested = Value::None;
    for _ in 0..MAX_DEPTH - 1 {
        nested = Value::List(vec![nested]);
    }
    let encoded = to_vec(&nested);
    assert_eq!(from_slice(&encoded, StringMode::Raw).unwrap(), nested);
}

#[test]
fn an_enormous_declared_string_length_is_refused_without_allocating() {
    // Eleven bytes on the wire claiming a 99 gigabyte string. A decoder that
    // reserves first and reads second dies here.
    let claim = b"99999999999:abc";
    match from_slice(claim, StringMode::Utf8Lossy) {
        Err(Error::LengthLimit { .. }) | Err(Error::MalformedNumber { .. }) => {}
        other => panic!("expected a length refusal, got {other:?}"),
    }

    // And one just over the cap, which must be refused on the length alone
    // rather than after trying to read that many bytes.
    let mut just_over = format!("{}:", redeluge_rencode::MAX_LENGTH + 1).into_bytes();
    just_over.push(b'x');
    assert!(matches!(
        from_slice(&just_over, StringMode::Utf8Lossy),
        Err(Error::LengthLimit { .. })
    ));
}

#[test]
fn a_huge_fixed_list_header_does_not_reserve_memory() {
    // The fixed-list code promises up to 63 items but the input supplies none.
    // Reserving for the promise and then failing is still a refusal, but it
    // must not be a multi-gigabyte one, which is why the decoder caps its
    // with_capacity hint.
    let header = [codes::LIST_FIXED_START + 63];
    assert!(from_slice(&header, StringMode::Raw).is_err());
}

#[test]
fn unknown_type_codes_are_rejected_by_name() {
    // 45, 46, 47 and 58 are the gaps between the ranges. They mean nothing.
    for code in [45u8, 46, 47, 58] {
        match from_slice(&[code], StringMode::Raw) {
            Err(Error::UnknownTypeCode { code: reported, .. }) => assert_eq!(reported, code),
            other => panic!("code {code} should be unknown, got {other:?}"),
        }
    }
}

#[test]
fn trailing_bytes_after_a_value_are_rejected() {
    let mut encoded = to_vec(&Value::Int(7));
    encoded.push(codes::CHR_NONE);
    match from_slice(&encoded, StringMode::Raw) {
        Err(Error::TrailingBytes { extra }) => assert_eq!(extra, 1),
        other => panic!("expected TrailingBytes, got {other:?}"),
    }
}

#[test]
fn unterminated_containers_are_rejected() {
    assert!(matches!(
        from_slice(&[codes::CHR_LIST, codes::CHR_NONE], StringMode::Raw),
        Err(Error::Unterminated { .. })
    ));
    assert!(matches!(
        from_slice(&[codes::CHR_DICT, codes::CHR_NONE], StringMode::Raw),
        Err(Error::Unterminated { .. })
    ));
}

#[test]
fn a_dictionary_key_without_a_value_is_rejected() {
    let input = [codes::CHR_DICT, codes::CHR_NONE, codes::CHR_TERM];
    assert!(matches!(
        from_slice(&input, StringMode::Raw),
        Err(Error::DanglingKey { .. })
    ));
}

#[test]
fn non_canonical_integers_are_rejected() {
    // The reference implementation refuses these, so accepting them would let
    // two encodings mean the same value.
    for text in ["-0", "007", "0123", "", "-", "12a", " 12"] {
        let mut input = vec![codes::CHR_INT];
        input.extend_from_slice(text.as_bytes());
        input.push(codes::CHR_TERM);
        assert!(
            from_slice(&input, StringMode::Raw).is_err(),
            "{text:?} should not decode"
        );
    }
}

#[test]
fn an_overlong_decimal_integer_is_refused() {
    let mut input = vec![codes::CHR_INT];
    input.extend(std::iter::repeat_n(b'9', 200));
    input.push(codes::CHR_TERM);
    assert!(matches!(
        from_slice(&input, StringMode::Raw),
        Err(Error::LengthLimit { .. })
    ));
}

#[test]
fn integers_beyond_64_bits_survive_as_text() {
    // Python has no integer width limit, so a peer can send one we cannot hold.
    // Dropping it would corrupt the message; keeping the text does not.
    let huge = "123456789012345678901234567890";
    let mut input = vec![codes::CHR_INT];
    input.extend_from_slice(huge.as_bytes());
    input.push(codes::CHR_TERM);

    let decoded = from_slice(&input, StringMode::Raw).unwrap();
    assert_eq!(decoded, Value::BigInt(huge.to_owned()));
    assert_eq!(to_vec(&decoded), input, "it must re-encode unchanged");
}

#[test]
fn an_empty_input_is_truncated_not_a_panic() {
    match from_slice(&[], StringMode::Raw) {
        Err(err) => assert!(err.is_truncated()),
        other => panic!("expected a truncation error, got {other:?}"),
    }
}
