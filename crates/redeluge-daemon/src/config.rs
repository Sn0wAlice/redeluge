// SPDX-License-Identifier: GPL-3.0-or-later
//! `core.conf`, the daemon's configuration.
//!
//! The file format is Deluge's: two concatenated JSON objects, a version header
//! then the settings. Existing files are read as they are and written back in
//! the same shape, so an installation does not have to be converted.
//!
//! Values are kept as JSON rather than mapped onto a struct. The RPC exposes
//! `core.get_config` and `core.set_config` over the whole dictionary, including
//! keys this daemon does not itself read, and a struct would silently drop
//! anything it did not know about.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value as Json};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("could not write {path}: {source}")]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path} is not a Deluge config file: {reason}")]
    Malformed { path: PathBuf, reason: String },
    #[error("no such setting: {0}")]
    UnknownKey(String),
    #[error("setting {key} is a {expected}, not a {found}")]
    WrongType {
        key: String,
        expected: &'static str,
        found: &'static str,
    },
}

pub type Result<T> = std::result::Result<T, Error>;

/// The daemon's configuration.
pub struct Config {
    path: PathBuf,
    version: Map<String, Json>,
    values: Map<String, Json>,
    /// Keys changed since the last save, so a save is skippable when nothing
    /// moved. The daemon saves on a timer.
    dirty: bool,
}

impl Config {
    /// Loads `core.conf`, filling in any key the file does not have.
    pub fn load(config_dir: &Path) -> Result<Self> {
        let path = config_dir.join("core.conf");

        let (version, mut values) = match std::fs::read_to_string(&path) {
            Ok(text) => parse(&text, &path)?,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => (Map::new(), Map::new()),
            Err(source) => return Err(Error::Read { path, source }),
        };

        // Defaults fill the gaps rather than replacing the file: a key added in
        // a later version appears, and one the operator set stays set.
        let mut added = 0;
        for (key, value) in defaults(config_dir) {
            if !values.contains_key(&key) {
                values.insert(key, value);
                added += 1;
            }
        }
        if added > 0 {
            tracing::info!(added, "filled in missing configuration keys");
        }

        Ok(Self {
            path,
            version,
            values,
            dirty: added > 0,
        })
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        self.values.get(key)
    }

    pub fn string(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Json::as_str)
    }

    pub fn integer(&self, key: &str) -> Option<i64> {
        self.get(key).and_then(Json::as_i64)
    }

    /// A number that may be written as an integer or a float.
    ///
    /// Rate limits are floats in Deluge's config and integers everywhere else,
    /// and a client that writes 200 rather than 200.0 must not break them.
    pub fn number(&self, key: &str) -> Option<f64> {
        self.get(key).and_then(Json::as_f64)
    }

    pub fn boolean(&self, key: &str) -> Option<bool> {
        self.get(key).and_then(Json::as_bool)
    }

    /// Everything, for `core.get_config`.
    pub fn all(&self) -> &Map<String, Json> {
        &self.values
    }

    /// Sets one value, refusing an unknown key or a change of type.
    ///
    /// Both refusals matter: a typo in a client would otherwise add a key
    /// nothing reads, and a string where a number belongs would break whatever
    /// consumes it later, far from the call that caused it.
    pub fn set(&mut self, key: &str, value: Json) -> Result<()> {
        let Some(current) = self.values.get(key) else {
            return Err(Error::UnknownKey(key.to_owned()));
        };

        let (expected, found) = (type_name(current), type_name(&value));
        // Integers and floats are interchangeable; everything else is not.
        let compatible = expected == found
            || (matches!(expected, "number") && matches!(found, "number"))
            || current.is_null()
            || value.is_null();
        if !compatible {
            return Err(Error::WrongType {
                key: key.to_owned(),
                expected,
                found,
            });
        }

        if self.values.get(key) == Some(&value) {
            return Ok(()); // No change, so no event and no save.
        }
        self.values.insert(key.to_owned(), value);
        self.dirty = true;
        Ok(())
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Writes the file, if anything changed.
    pub fn save(&mut self) -> Result<()> {
        if !self.dirty {
            return Ok(());
        }

        let version = if self.version.is_empty() {
            json!({"file": 1, "format": 1})
        } else {
            Json::Object(self.version.clone())
        };

        // Deluge writes both objects with sorted keys and four-space indent, and
        // matching that keeps a diff between the two implementations readable.
        let body = format!(
            "{}{}",
            to_deluge_json(&version),
            to_deluge_json(&Json::Object(sorted(&self.values)))
        );

        write_atomically(&self.path, body.as_bytes())?;
        self.dirty = false;
        Ok(())
    }
}

fn sorted(values: &Map<String, Json>) -> Map<String, Json> {
    let ordered: BTreeMap<&String, &Json> = values.iter().collect();
    ordered
        .into_iter()
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn to_deluge_json(value: &Json) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".to_owned())
}

fn type_name(value: &Json) -> &'static str {
    match value {
        Json::Null => "null",
        Json::Bool(_) => "bool",
        Json::Number(_) => "number",
        Json::String(_) => "string",
        Json::Array(_) => "list",
        Json::Object(_) => "dict",
    }
}

fn parse(text: &str, path: &Path) -> Result<(Map<String, Json>, Map<String, Json>)> {
    let objects = split_json_objects(text);
    let object = |text: &str| -> Result<Map<String, Json>> {
        match serde_json::from_str::<Json>(text) {
            Ok(Json::Object(map)) => Ok(map),
            Ok(other) => Err(Error::Malformed {
                path: path.to_path_buf(),
                reason: format!("expected an object, found {}", type_name(&other)),
            }),
            Err(err) => Err(Error::Malformed {
                path: path.to_path_buf(),
                reason: err.to_string(),
            }),
        }
    };

    match objects.len() {
        1 => Ok((Map::new(), object(&objects[0])?)),
        2 => Ok((object(&objects[0])?, object(&objects[1])?)),
        0 => Err(Error::Malformed {
            path: path.to_path_buf(),
            reason: "no JSON object found".to_owned(),
        }),
        count => Err(Error::Malformed {
            path: path.to_path_buf(),
            reason: format!("expected one or two JSON objects, found {count}"),
        }),
    }
}

/// Splits concatenated top-level JSON objects.
///
/// Braces inside strings do not count, and a backslash escapes the next
/// character, which is why this cannot be a brace counter alone.
fn split_json_objects(text: &str) -> Vec<String> {
    let mut objects = Vec::new();
    let mut depth = 0usize;
    let mut start = None;
    let mut in_string = false;
    let mut escaped = false;

    for (index, character) in text.char_indices() {
        if in_string {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                in_string = false;
            }
            continue;
        }
        match character {
            '"' => in_string = true,
            '{' => {
                if depth == 0 {
                    start = Some(index);
                }
                depth += 1;
            }
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    if let Some(begin) = start.take() {
                        objects.push(text[begin..=index].to_owned());
                    }
                }
            }
            _ => {}
        }
    }
    objects
}

fn write_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, bytes).map_err(|source| Error::Write {
        path: temporary.clone(),
        source,
    })?;

    if path.exists() {
        let _ = std::fs::rename(path, path.with_extension("bak"));
    }
    std::fs::rename(&temporary, path).map_err(|source| Error::Write {
        path: path.to_path_buf(),
        source,
    })
}

/// Every key the daemon understands, with the value a fresh install gets.
///
/// Taken from `deluge/core/preferencesmanager.py`. The five path settings are
/// built from the configuration directory, which is why this takes one.
pub fn defaults(config_dir: &Path) -> Vec<(String, Json)> {
    let path = |name: &str| Json::String(config_dir.join(name).display().to_string());
    let downloads = Json::String(
        dirs_download()
            .unwrap_or_else(|| config_dir.to_path_buf())
            .display()
            .to_string(),
    );

    vec![
        ("send_info".into(), json!(false)),
        ("info_sent".into(), json!(0.0)),
        ("daemon_port".into(), json!(58846)),
        ("allow_remote".into(), json!(false)),
        ("pre_allocate_storage".into(), json!(false)),
        ("download_location".into(), downloads.clone()),
        ("listen_ports".into(), json!([6881, 6891])),
        ("listen_interface".into(), json!("")),
        ("outgoing_interface".into(), json!("")),
        ("random_port".into(), json!(true)),
        ("listen_random_port".into(), Json::Null),
        ("listen_use_sys_port".into(), json!(false)),
        ("listen_reuse_port".into(), json!(true)),
        ("outgoing_ports".into(), json!([0, 0])),
        ("random_outgoing_ports".into(), json!(true)),
        ("copy_torrent_file".into(), json!(false)),
        ("del_copy_torrent_file".into(), json!(false)),
        ("torrentfiles_location".into(), path("torrents")),
        ("plugins_location".into(), path("plugins")),
        ("prioritize_first_last_pieces".into(), json!(false)),
        ("sequential_download".into(), json!(false)),
        ("dht".into(), json!(true)),
        ("upnp".into(), json!(true)),
        ("natpmp".into(), json!(true)),
        ("utpex".into(), json!(true)),
        ("lsd".into(), json!(true)),
        ("enc_in_policy".into(), json!(1)),
        ("enc_out_policy".into(), json!(1)),
        ("enc_level".into(), json!(2)),
        ("max_connections_global".into(), json!(200)),
        ("max_upload_speed".into(), json!(-1.0)),
        ("max_download_speed".into(), json!(-1.0)),
        ("max_upload_slots_global".into(), json!(4)),
        ("max_half_open_connections".into(), json!(50)),
        ("max_connections_per_second".into(), json!(20)),
        ("ignore_limits_on_local_network".into(), json!(true)),
        ("max_connections_per_torrent".into(), json!(-1)),
        ("max_upload_slots_per_torrent".into(), json!(-1)),
        ("max_upload_speed_per_torrent".into(), json!(-1)),
        ("max_download_speed_per_torrent".into(), json!(-1)),
        ("enabled_plugins".into(), json!([])),
        ("add_paused".into(), json!(false)),
        ("max_active_seeding".into(), json!(5)),
        ("max_active_downloading".into(), json!(3)),
        ("max_active_limit".into(), json!(8)),
        ("dont_count_slow_torrents".into(), json!(false)),
        ("auto_manage_prefer_seeds".into(), json!(false)),
        ("queue_new_to_top".into(), json!(false)),
        ("stop_seed_at_ratio".into(), json!(false)),
        ("remove_seed_at_ratio".into(), json!(false)),
        ("stop_seed_ratio".into(), json!(2.0)),
        ("share_ratio_limit".into(), json!(2.0)),
        ("seed_time_ratio_limit".into(), json!(7.0)),
        ("seed_time_limit".into(), json!(180)),
        ("auto_managed".into(), json!(true)),
        ("move_completed".into(), json!(false)),
        ("move_completed_path".into(), downloads),
        ("move_completed_paths_list".into(), json!([])),
        ("download_location_paths_list".into(), json!([])),
        (
            "path_chooser_show_chooser_button_on_localhost".into(),
            json!(true),
        ),
        ("path_chooser_auto_complete_enabled".into(), json!(true)),
        ("path_chooser_accelerator_string".into(), json!("Tab")),
        ("path_chooser_max_popup_rows".into(), json!(20)),
        ("path_chooser_show_hidden_files".into(), json!(false)),
        ("new_release_check".into(), json!(true)),
        (
            "proxy".into(),
            json!({
                "type": 0,
                "hostname": "",
                "username": "",
                "password": "",
                "port": 8080,
                "proxy_hostnames": true,
                "proxy_peer_connections": true,
                "proxy_tracker_connections": true,
                "force_proxy": false,
                "anonymous_mode": false,
            }),
        ),
        ("peer_tos".into(), json!("0x00")),
        ("rate_limit_ip_overhead".into(), json!(true)),
        (
            "geoip_db_location".into(),
            json!("/usr/share/GeoIP/GeoIP.dat"),
        ),
        ("cache_size".into(), json!(512)),
        ("cache_expiry".into(), json!(60)),
        ("shared".into(), json!(false)),
        ("super_seeding".into(), json!(false)),
        ("announce_to_all_tiers".into(), json!(false)),
        // SSL torrents: a second listen port that only accepts peers
        // presenting a certificate.
        ("ssl_torrents".into(), json!(false)),
        ("ssl_listen_ports".into(), json!([6892, 6896])),
        ("ssl_torrents_certs".into(), path("ssl_torrents_certs")),
        // The four plugins that became features. Each is one key holding one
        // dictionary, so a client configures them through core.set_config like
        // anything else. Labels are the exception that also keeps an RPC
        // namespace: which label a torrent carries is a torrent option, but
        // the register of labels that exist has to be a setting, because a
        // label with nothing in it yet is the one an external client is about
        // to start using.
        (
            "autoadd".into(),
            crate::features::autoadd::Settings::default_json(),
        ),
        (
            "label".into(),
            crate::features::label::Settings::default_json(),
        ),
        (
            "blocklist".into(),
            crate::features::blocklist::Settings::default_json(),
        ),
        (
            "scheduler".into(),
            crate::features::scheduler::Settings::default_json(),
        ),
    ]
}

fn dirs_download() -> Option<PathBuf> {
    // The XDG user directory, which is where Deluge puts downloads by default.
    if let Ok(explicit) = std::env::var("DELUGE_DOWNLOAD_DIR") {
        if !explicit.is_empty() {
            return Some(PathBuf::from(explicit));
        }
    }
    std::env::var("HOME")
        .ok()
        .filter(|home| !home.is_empty())
        .map(|home| PathBuf::from(home).join("Downloads"))
}
