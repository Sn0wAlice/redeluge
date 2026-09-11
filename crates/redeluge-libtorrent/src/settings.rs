// SPDX-License-Identifier: GPL-3.0-or-later
//! Session settings.
//!
//! libtorrent looks settings up by name and encodes the type in the index, so
//! the bridge takes a name and a tagged value. That is one code path for the
//! twenty-nine settings Deluge applies today and for whatever it applies later.
//!
//! Getting a name or a type wrong is an error rather than a silent no-op: a
//! rate limit that quietly stops applying is the kind of bug nobody notices
//! for a month.
//!
//! A setting does not always read back as written. libtorrent normalises some
//! values, notably turning a rate limit of -1, which is how Deluge spells "no
//! limit", into 0, which is how libtorrent spells it. Comparing what was
//! written with what reads back is therefore not a way to tell whether a change
//! took effect.

use crate::bridge::ffi::SettingValue;

/// One session setting, ready to apply.
#[derive(Debug, Clone, PartialEq)]
pub struct Setting {
    name: String,
    value: SettingKind,
}

#[derive(Debug, Clone, PartialEq)]
enum SettingKind {
    Int(i64),
    Bool(bool),
    Str(String),
}

impl Setting {
    /// An integer setting, such as `download_rate_limit`.
    pub fn int(name: impl Into<String>, value: i64) -> Self {
        Self {
            name: name.into(),
            value: SettingKind::Int(value),
        }
    }

    /// A boolean setting, such as `enable_dht`.
    pub fn boolean(name: impl Into<String>, value: bool) -> Self {
        Self {
            name: name.into(),
            value: SettingKind::Bool(value),
        }
    }

    /// A string setting, such as `listen_interfaces`.
    pub fn string(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: SettingKind::Str(value.into()),
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn to_ffi(&self) -> SettingValue {
        let mut out = SettingValue {
            name: self.name.clone(),
            kind: 0,
            int_value: 0,
            bool_value: false,
            str_value: String::new(),
        };
        match &self.value {
            SettingKind::Int(value) => {
                out.kind = 0;
                out.int_value = *value;
            }
            SettingKind::Bool(value) => {
                out.kind = 1;
                out.bool_value = *value;
            }
            SettingKind::Str(value) => {
                out.kind = 2;
                out.str_value = value.clone();
            }
        }
        out
    }
}

/// The settings Deluge applies, by their libtorrent names.
///
/// Not an exhaustive list of libtorrent's settings, which runs to hundreds.
/// These are the ones `deluge/core/preferencesmanager.py` maps a configuration
/// key onto, so they are the ones the daemon has to be able to set.
pub mod names {
    /// Integer settings.
    pub const INT: &[&str] = &[
        "active_downloads",
        "active_limit",
        "active_seeds",
        "alert_queue_size",
        "cache_expiry",
        "cache_size",
        "connection_speed",
        "connections_limit",
        "download_rate_limit",
        "half_open_limit",
        "peer_tos",
        "proxy_port",
        "proxy_type",
        "seed_time_limit",
        "seed_time_ratio_limit",
        "share_ratio_limit",
        "unchoke_slots_limit",
        "upload_rate_limit",
    ];

    /// Boolean settings.
    pub const BOOL: &[&str] = &[
        "announce_to_all_tiers",
        "anonymous_mode",
        "auto_manage_prefer_seeds",
        "dont_count_slow_torrents",
        "enable_dht",
        "enable_lsd",
        "enable_natpmp",
        "enable_upnp",
        "ignore_limits_on_local_network",
        "proxy_hostnames",
        "proxy_peer_connections",
        "proxy_tracker_connections",
        "rate_limit_ip_overhead",
    ];

    /// String settings.
    pub const STR: &[&str] = &[
        "listen_interfaces",
        "outgoing_interfaces",
        "proxy_hostname",
        "proxy_password",
        "proxy_username",
        "user_agent",
    ];
}
