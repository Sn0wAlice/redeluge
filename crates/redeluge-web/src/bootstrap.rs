// SPDX-License-Identifier: GPL-3.0-or-later
//! Making a fresh configuration usable without anyone editing a file.
//!
//! Deluge expects a human to open the connection manager and pick a daemon. In
//! a container there is exactly one, on localhost, so this wires the two
//! together on startup: it reads the daemon's own `localclient` credentials out
//! of the auth file, puts a matching entry in `hostlist.conf`, and points
//! `web.conf` at it.
//!
//! This was a Python script until the Python tree was deleted. It belongs here:
//! it is the Web UI's own setup, and a separate script is one more thing that
//! can be forgotten.

use std::path::Path;

use serde_json::{json, Value as Json};

use crate::auth::{hash_password, StoredPassword};
use crate::config::ConfigFile;
use crate::persist;

/// What the caller asked for through the environment.
#[derive(Debug, Clone, Default)]
pub struct Wanted {
    /// Applied on a first run, or when `reset` is set.
    pub password: Option<String>,
    pub reset_password: bool,
    pub daemon_port: u16,
}

/// What bootstrapping changed, for the log.
#[derive(Debug, Default)]
pub struct Outcome {
    pub password_set: bool,
    pub host_added: bool,
    pub host_updated: bool,
}

/// Reads the daemon's `localclient` credentials.
///
/// That account exists so a local tool can log in without anyone typing
/// anything, which is exactly what this is.
pub fn localclient(config_dir: &Path) -> Option<(String, String)> {
    let text = std::fs::read_to_string(config_dir.join("auth")).ok()?;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let mut fields = line.split(':');
        let username = fields.next()?;
        if username.trim() != "localclient" {
            continue;
        }
        let password = fields.next()?.trim().to_owned();
        return Some((username.trim().to_owned(), password));
    }
    None
}

/// Prepares `web.conf` and `hostlist.conf` for a first run.
///
/// Idempotent: a password already set is left alone unless a reset is asked
/// for, and an existing host entry is only realigned with the auth file, which
/// it has to be because the daemon regenerates that password whenever the auth
/// file is recreated.
pub fn run(config_dir: &Path, wanted: &Wanted) -> Result<Outcome, persist::Error> {
    let mut outcome = Outcome::default();

    let mut hosts = ConfigFile::load(config_dir.join("hostlist.conf"))
        .unwrap_or_else(|_| empty(config_dir.join("hostlist.conf")));
    let mut web = ConfigFile::load(config_dir.join("web.conf"))
        .unwrap_or_else(|_| empty(config_dir.join("web.conf")));

    let credentials = localclient(config_dir);
    let host_id = ensure_host(&mut hosts, wanted.daemon_port, credentials, &mut outcome);

    if hosts.version.is_empty() {
        // hostlist.conf is at file version 3; writing 1 would make a Python
        // client run its own migration over a file that never needed one.
        hosts.version.insert("file".to_owned(), json!(3));
        hosts.version.insert("format".to_owned(), json!(1));
    }
    persist::save_config_blocking(&hosts)?;

    if let Some(id) = host_id {
        web.settings
            .insert("default_daemon".to_owned(), Json::String(id));
    }

    let stored = StoredPassword::from_config(
        web.settings.get("pwd_salt").and_then(Json::as_str),
        web.settings.get("pwd_sha1").and_then(Json::as_str),
    );
    let unset = matches!(stored, StoredPassword::Unset);

    if let Some(password) = &wanted.password {
        if unset || wanted.reset_password {
            match hash_password(password) {
                Ok(hashed) => {
                    if let Some(encoded) = hashed.to_config_string() {
                        web.settings
                            .insert("pwd_sha1".to_owned(), Json::String(encoded));
                        web.settings
                            .insert("pwd_salt".to_owned(), Json::String(String::new()));
                        web.settings.insert("first_login".to_owned(), json!(false));
                        outcome.password_set = true;
                    }
                }
                Err(err) => tracing::error!(error = %err, "could not hash the web password"),
            }
        }
    }

    if web.version.is_empty() {
        // web.conf is at file version 2.
        web.version.insert("file".to_owned(), json!(2));
        web.version.insert("format".to_owned(), json!(1));
    }
    persist::save_config_blocking(&web)?;
    Ok(outcome)
}

fn empty(path: std::path::PathBuf) -> ConfigFile {
    ConfigFile {
        path,
        version: Default::default(),
        settings: Default::default(),
    }
}

/// Returns the host id of the localhost daemon, creating the entry if needed.
fn ensure_host(
    hosts: &mut ConfigFile,
    port: u16,
    credentials: Option<(String, String)>,
    outcome: &mut Outcome,
) -> Option<String> {
    let (username, password) = credentials?;

    let entries = hosts
        .settings
        .entry("hosts".to_owned())
        .or_insert_with(|| Json::Array(Vec::new()));
    let Json::Array(entries) = entries else {
        return None;
    };

    let matches_localhost = |entry: &Json| -> bool {
        let Some(fields) = entry.as_array() else {
            return false;
        };
        let host = fields.get(1).and_then(Json::as_str).unwrap_or("");
        let entry_port = fields.get(2).and_then(Json::as_u64).unwrap_or(0);
        matches!(host, "127.0.0.1" | "localhost" | "::1") && entry_port == u64::from(port)
    };

    if let Some(existing) = entries.iter_mut().find(|entry| matches_localhost(entry)) {
        let id = existing
            .as_array()
            .and_then(|fields| fields.first())
            .and_then(Json::as_str)
            .map(str::to_owned)?;

        // The stored password goes stale whenever the auth file is recreated,
        // so it is rewritten rather than trusted.
        *existing = json!([id, "127.0.0.1", port, username, password]);
        outcome.host_updated = true;
        return Some(id);
    }

    let id = random_host_id();
    entries.push(json!([id, "127.0.0.1", port, username, password]));
    outcome.host_added = true;
    Some(id)
}

/// A host id, in the shape Deluge uses: thirty-two hex characters.
fn random_host_id() -> String {
    use ring::rand::SecureRandom;

    let rng = ring::rand::SystemRandom::new();
    let mut raw = [0u8; 16];
    if rng.fill(&mut raw).is_err() {
        // Only reachable if the system has no randomness, in which case a
        // predictable id is the least of anyone's problems.
        return "00000000000000000000000000000000".to_owned();
    }
    hex::encode(raw)
}
