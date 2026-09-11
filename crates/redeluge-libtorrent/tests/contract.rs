// SPDX-License-Identifier: GPL-3.0-or-later
//! The Rust alert enum and the frozen contract must not drift apart.
//!
//! `contract/alerts.json` is extracted from the Python daemon by
//! `tools/extract_contract.py`. If someone adds an alert handler there, or
//! renumbers one here, this test is what catches it.

use redeluge_libtorrent::AlertKind;

fn contract_alerts() -> Vec<String> {
    let raw = include_str!("../../../contract/alerts.json");
    let parsed: serde_json::Value =
        serde_json::from_str(raw).expect("contract/alerts.json is not valid JSON");
    parsed["alerts"]
        .as_array()
        .expect("contract/alerts.json has no alerts array")
        .iter()
        .map(|entry| {
            entry["handler_key"]
                .as_str()
                .expect("alert entry has no handler_key")
                .to_owned()
        })
        .collect()
}

#[test]
fn every_contract_alert_has_a_kind() {
    let expected = contract_alerts();
    let actual: Vec<String> = AlertKind::all()
        .map(|kind| kind.handler_key().to_owned())
        .collect();

    assert_eq!(
        expected, actual,
        "AlertKind no longer matches contract/alerts.json. \
         Re-run tools/extract_contract.py, then update src/alert.rs and src/shim.cc together."
    );
}

#[test]
fn kind_count_matches_the_contract() {
    assert_eq!(
        contract_alerts().len(),
        usize::from(AlertKind::COUNT),
        "AlertKind::COUNT is out of step with the contract"
    );
}

#[test]
fn discriminants_survive_a_round_trip() {
    for kind in AlertKind::all() {
        let raw = kind as u16;
        assert_eq!(
            AlertKind::from_raw(raw),
            kind,
            "discriminant {raw} does not round-trip"
        );
    }
}

#[test]
fn unmapped_discriminants_become_unknown() {
    // An alert libtorrent adds in a future release must not be mistaken for a
    // handled one, and must not panic.
    for raw in [0, AlertKind::COUNT + 1, u16::MAX] {
        assert_eq!(AlertKind::from_raw(raw), AlertKind::Unknown);
    }
}

#[test]
fn handler_keys_are_unique() {
    let mut keys: Vec<&str> = AlertKind::all().map(AlertKind::handler_key).collect();
    let total = keys.len();
    keys.sort_unstable();
    keys.dedup();
    assert_eq!(keys.len(), total, "two alert kinds share a handler key");
}
