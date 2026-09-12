// SPDX-License-Identifier: GPL-3.0-or-later
//! Turning the daemon's configuration into libtorrent settings.
//!
//! This is `deluge/core/preferencesmanager.py`'s mapping, in one place rather
//! than spread over thirty handlers. Two kinds of conversion happen here and
//! nowhere else: Deluge's speeds are in KiB/s and libtorrent's in bytes per
//! second, and Deluge spells "no limit" as -1 where libtorrent spells it 0.

use redeluge_libtorrent::Setting;

use crate::config::Config;

/// Every session setting the configuration implies.
pub fn to_settings(config: &Config) -> Vec<Setting> {
    let mut settings = Vec::new();

    let int = |key: &str, default: i64| config.integer(key).unwrap_or(default);
    let boolean = |key: &str, default: bool| config.boolean(key).unwrap_or(default);
    let rate = |key: &str| kib_to_bytes(config.number(key).unwrap_or(-1.0));

    settings.push(Setting::string(
        "listen_interfaces",
        listen_interfaces(config),
    ));
    settings.push(Setting::string(
        "outgoing_interfaces",
        config.string("outgoing_interface").unwrap_or("").to_owned(),
    ));

    settings.push(Setting::boolean("enable_dht", boolean("dht", true)));
    settings.push(Setting::boolean("enable_upnp", boolean("upnp", true)));
    settings.push(Setting::boolean("enable_natpmp", boolean("natpmp", true)));
    settings.push(Setting::boolean("enable_lsd", boolean("lsd", true)));

    settings.push(Setting::int(
        "connections_limit",
        int("max_connections_global", 200),
    ));
    settings.push(Setting::int("upload_rate_limit", rate("max_upload_speed")));
    settings.push(Setting::int(
        "download_rate_limit",
        rate("max_download_speed"),
    ));
    settings.push(Setting::int(
        "unchoke_slots_limit",
        int("max_upload_slots_global", 4),
    ));
    settings.push(Setting::int(
        "half_open_limit",
        int("max_half_open_connections", 50),
    ));
    settings.push(Setting::int(
        "connection_speed",
        int("max_connections_per_second", 20),
    ));
    settings.push(Setting::boolean(
        "ignore_limits_on_local_network",
        boolean("ignore_limits_on_local_network", true),
    ));

    settings.push(Setting::int(
        "active_downloads",
        int("max_active_downloading", 3),
    ));
    settings.push(Setting::int("active_seeds", int("max_active_seeding", 5)));
    settings.push(Setting::int("active_limit", int("max_active_limit", 8)));
    settings.push(Setting::boolean(
        "dont_count_slow_torrents",
        boolean("dont_count_slow_torrents", false),
    ));

    // What "slow" means, for the checkbox above and for the idle rule, which
    // is one number on purpose. Two mechanisms that stop a stalled torrent
    // holding up the queue, disagreeing about which torrents are stalled,
    // would be impossible to reason about.
    let idle = crate::features::idlepause::Settings::from_config(config.get("idle_pause")).sane();
    settings.push(Setting::int("inactive_down_rate", idle.inactive_rate));
    settings.push(Setting::int("inactive_up_rate", idle.inactive_rate));
    settings.push(Setting::boolean(
        "auto_manage_prefer_seeds",
        boolean("auto_manage_prefer_seeds", false),
    ));
    settings.push(Setting::boolean(
        "announce_to_all_tiers",
        boolean("announce_to_all_tiers", false),
    ));

    // libtorrent wants these as percentages and minutes, which is what Deluge
    // stores multiplied out.
    settings.push(Setting::int(
        "share_ratio_limit",
        (config.number("share_ratio_limit").unwrap_or(2.0) * 100.0) as i64,
    ));
    settings.push(Setting::int(
        "seed_time_ratio_limit",
        (config.number("seed_time_ratio_limit").unwrap_or(7.0) * 100.0) as i64,
    ));
    settings.push(Setting::int(
        "seed_time_limit",
        int("seed_time_limit", 180) * 60,
    ));

    settings.push(Setting::boolean(
        "rate_limit_ip_overhead",
        boolean("rate_limit_ip_overhead", true),
    ));
    settings.push(Setting::int("cache_size", int("cache_size", 512)));
    settings.push(Setting::int("cache_expiry", int("cache_expiry", 60)));

    if let Some(tos) = config.string("peer_tos") {
        // Stored as a hex string, which is how the preferences dialog shows it.
        if let Some(value) = parse_tos(tos) {
            settings.push(Setting::int("peer_tos", value));
        }
    }

    settings.extend(proxy_settings(config));
    settings
}

/// libtorrent takes one string listing every interface and port.
fn listen_interfaces(config: &Config) -> String {
    let ports = config
        .get("listen_ports")
        .and_then(|value| value.as_array())
        .and_then(|ports| ports.first().and_then(|port| port.as_i64()))
        .unwrap_or(6881);

    // A random port is what Deluge does by default, and 0 is how libtorrent
    // spells "choose one".
    let port = if config.boolean("random_port").unwrap_or(true) {
        config.integer("listen_random_port").unwrap_or(0)
    } else {
        ports
    };

    let interface = config.string("listen_interface").unwrap_or("").trim();
    if interface.is_empty() {
        format!("0.0.0.0:{port},[::]:{port}")
    } else if interface.contains(':') {
        // An IPv6 literal needs brackets or the port is ambiguous.
        format!("[{interface}]:{port}")
    } else {
        format!("{interface}:{port}")
    }
}

fn proxy_settings(config: &Config) -> Vec<Setting> {
    let Some(proxy) = config.get("proxy").and_then(|value| value.as_object()) else {
        return Vec::new();
    };
    let string = |key: &str| {
        proxy
            .get(key)
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .to_owned()
    };
    let boolean = |key: &str, default: bool| {
        proxy
            .get(key)
            .and_then(|value| value.as_bool())
            .unwrap_or(default)
    };

    vec![
        Setting::int(
            "proxy_type",
            proxy.get("type").and_then(|v| v.as_i64()).unwrap_or(0),
        ),
        Setting::string("proxy_hostname", string("hostname")),
        Setting::int(
            "proxy_port",
            proxy.get("port").and_then(|v| v.as_i64()).unwrap_or(8080),
        ),
        Setting::string("proxy_username", string("username")),
        Setting::string("proxy_password", string("password")),
        Setting::boolean("proxy_hostnames", boolean("proxy_hostnames", true)),
        Setting::boolean(
            "proxy_peer_connections",
            boolean("proxy_peer_connections", true),
        ),
        Setting::boolean(
            "proxy_tracker_connections",
            boolean("proxy_tracker_connections", true),
        ),
        Setting::boolean("anonymous_mode", boolean("anonymous_mode", false)),
    ]
}

/// KiB/s to bytes per second, keeping -1 as -1.
///
/// libtorrent normalises -1 to 0 itself, so passing it through is correct and
/// matches what the Python daemon sends.
pub fn kib_to_bytes(kib: f64) -> i64 {
    if kib < 0.0 {
        return -1;
    }
    (kib * 1024.0).min(i32::MAX as f64) as i64
}

fn parse_tos(text: &str) -> Option<i64> {
    let trimmed = text.trim();
    let digits = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"));
    match digits {
        Some(hex) => i64::from_str_radix(hex, 16).ok(),
        None => trimmed.parse().ok(),
    }
}
