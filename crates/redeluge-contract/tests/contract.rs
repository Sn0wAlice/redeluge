// SPDX-License-Identifier: GPL-3.0-or-later
//! Invariants the frozen contract has to hold.
//!
//! These run without libtorrent, so they are the fast gate: a bad extraction is
//! caught here rather than three crates later.

use redeluge_contract::{AuthLevel, Contract, Transport};

#[test]
fn the_contract_parses() {
    let contract = Contract::get();
    assert!(!contract.methods().is_empty());
    assert!(!contract.events().is_empty());
    assert!(!contract.alerts().is_empty());
}

#[test]
fn the_method_count_is_the_one_we_committed_to() {
    // 99 is the Python daemon's 109 minus the 10 plugin-management methods.
    // If this changes, either the Python tree moved or the extractor did, and
    // the roadmap's figures need revisiting.
    assert_eq!(Contract::get().methods().len(), 99);
    assert_eq!(Contract::get().removed().len(), 10);
}

#[test]
fn method_names_are_unique() {
    let contract = Contract::get();
    let mut names: Vec<&str> = contract.methods().iter().map(|m| m.name.as_str()).collect();
    let total = names.len();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), total, "two methods share a name");
}

#[test]
fn every_method_is_namespaced() {
    for method in Contract::get().methods() {
        let (namespace, rest) = method
            .name
            .split_once('.')
            .unwrap_or_else(|| panic!("{} is not namespaced", method.name));
        assert_eq!(namespace, method.namespace);
        assert_eq!(rest, method.method);
        assert!(
            !rest.contains('.'),
            "{} has a nested namespace",
            method.name
        );
    }
}

#[test]
fn no_plugin_management_survived_the_cut() {
    let contract = Contract::get();
    for method in contract.methods() {
        assert!(
            !method.name.contains("plugin"),
            "{} should have been removed with the plugin system",
            method.name
        );
    }
    for method in contract.removed() {
        assert!(
            method.removed_because.is_some(),
            "{} was removed without a stated reason",
            method.name
        );
    }
}

#[test]
fn auth_levels_are_all_recognised() {
    for method in Contract::get().methods() {
        assert!(
            !matches!(method.auth_level, AuthLevel::Other(_)),
            "{} carries an auth level this crate does not name: {:?}",
            method.name,
            method.auth_level
        );
    }
}

#[test]
fn only_login_and_session_checks_are_unauthenticated() {
    // Anything reachable before a client proves who it is deserves a second
    // look. Today the list is short and it should stay that way.
    let open: Vec<&str> = Contract::get()
        .methods()
        .iter()
        .filter(|m| m.auth_level == AuthLevel::None)
        .map(|m| m.name.as_str())
        .collect();

    // core.get_auth_levels_mappings sits here too, which means the daemon
    // answers it before a client has proved anything. Noted in the phase 0
    // report; the Rust daemon should raise it to ReadOnly.
    assert_eq!(
        open,
        vec![
            "auth.check_session",
            "auth.login",
            "core.get_auth_levels_mappings"
        ],
        "the set of unauthenticated methods changed"
    );
}

#[test]
fn both_transports_carry_methods() {
    let contract = Contract::get();
    let daemon = contract.methods_for(Transport::Daemon).count();
    let web = contract.methods_for(Transport::Web).count();

    assert!(daemon > 0 && web > 0);
    assert_eq!(daemon + web, contract.methods().len());
}

#[test]
fn required_parameters_come_before_optional_ones() {
    // The wire protocol passes positional args, so a required parameter after
    // an optional one would be unimplementable.
    for method in Contract::get().methods() {
        let mut seen_optional = false;
        for param in &method.params {
            if param.required && seen_optional && !param.name.starts_with('*') {
                panic!(
                    "{} has required parameter `{}` after an optional one",
                    method.name, param.name
                );
            }
            seen_optional |= !param.required;
        }
    }
}

#[test]
fn every_method_points_back_at_its_source() {
    for method in Contract::get().methods() {
        let (path, line) = method
            .source
            .rsplit_once(':')
            .unwrap_or_else(|| panic!("{} has an unusable source reference", method.name));
        assert!(
            path.ends_with(".py"),
            "{} does not point at a file",
            method.name
        );
        assert!(
            line.parse::<u32>().is_ok(),
            "{} has no line number",
            method.name
        );
    }
}

#[test]
fn lookup_by_name_finds_a_known_method() {
    let contract = Contract::get();
    let method = contract
        .method("core.add_torrent_magnet")
        .expect("core.add_torrent_magnet must exist");

    assert_eq!(method.namespace, "core");
    assert_eq!(method.transport, Transport::Daemon);
    assert_eq!(method.auth_level, AuthLevel::Normal);
    assert!(method.required_params().count() >= 1);
    assert!(contract.method("core.nonexistent").is_none());
}

#[test]
fn missing_from_reports_the_whole_surface_when_nothing_is_built() {
    let contract = Contract::get();
    assert_eq!(contract.missing_from([]).len(), contract.methods().len());

    let all: Vec<&str> = contract.methods().iter().map(|m| m.name.as_str()).collect();
    assert!(contract.missing_from(all).is_empty());
}

#[test]
fn config_keys_are_present_and_unique() {
    let contract = Contract::get();
    assert_eq!(contract.core_config().len(), 77);
    assert_eq!(contract.web_config().len(), 19);

    for keys in [contract.core_config(), contract.web_config()] {
        let mut names: Vec<&str> = keys.iter().map(|k| k.key.as_str()).collect();
        let total = names.len();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), total, "duplicate configuration key");
    }
}

#[test]
fn only_the_known_path_defaults_are_computed() {
    // Five defaults are Python expressions rather than literals, and all five
    // build a path from the config directory. Those the Rust daemon can
    // reproduce. A sixth appearing here means an expression nobody has looked
    // at, so the test pins the set rather than forbidding it.
    let mut computed: Vec<&str> = Contract::get()
        .core_config()
        .iter()
        .chain(Contract::get().web_config())
        .filter(|key| key.kind == "computed")
        .map(|key| key.key.as_str())
        .collect();
    computed.sort_unstable();

    assert_eq!(
        computed,
        vec![
            "download_location",
            "move_completed_path",
            "plugins_location",
            "ssl_torrents_certs",
            "torrentfiles_location",
        ],
        "a configuration default became an expression, or stopped being one"
    );
}

#[test]
fn events_are_named_consistently() {
    for event in Contract::get().events() {
        assert!(
            event.name.ends_with("Event"),
            "{} does not look like an event",
            event.name
        );
    }
}

#[test]
fn alerts_carry_the_suffix_the_contract_promises() {
    for entry in Contract::get().alerts() {
        assert_eq!(entry.alert, format!("{}_alert", entry.handler_key));
    }
}
