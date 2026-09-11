// SPDX-License-Identifier: GPL-3.0-or-later
//! Writing configuration back.
//!
//! Files are written to a temporary name in the same directory and then
//! renamed, so a crash mid-write leaves the previous file intact rather than a
//! truncated one. The Python implementation does the same, and the reason is
//! the same: `web.conf` holds the only copy of the password.

use std::path::Path;

use serde_json::{json, Value as Json};

use crate::config::ConfigFile;
use crate::state::SharedState;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not write {path}: {source}")]
    Write {
        path: String,
        source: std::io::Error,
    },
    #[error("could not serialise the configuration: {0}")]
    Serialise(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Writes a config file in Deluge's two-object format.
pub async fn save_config(config: &ConfigFile) -> Result<()> {
    save_config_blocking(config)
}

/// The same, for callers that are not in an async context.
pub fn save_config_blocking(config: &ConfigFile) -> Result<()> {
    let version = if config.version.is_empty() {
        json!({"file": 1, "format": 1})
    } else {
        Json::Object(config.version.clone())
    };

    let body = format!(
        "{}{}",
        serde_json::to_string_pretty(&version)?,
        serde_json::to_string_pretty(&Json::Object(config.settings.clone()))?
    );

    write_atomically(&config.path, body.as_bytes())
}

/// Stores the current password into `web.conf`.
pub async fn save_password(state: &SharedState) -> Result<()> {
    let stored = state.password.read().await.clone();
    let Some(encoded) = stored.to_config_string() else {
        return Ok(());
    };

    let mut config = state.web_config.write().await;
    config
        .settings
        .insert("pwd_sha1".to_owned(), Json::String(encoded));
    // The salt lives inside the scrypt string now, so the old field would only
    // be misleading if it stayed.
    config
        .settings
        .insert("pwd_salt".to_owned(), Json::String(String::new()));
    let snapshot = config.clone();
    drop(config);

    save_config(&snapshot).await
}

fn write_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let temporary = path.with_extension("tmp");

    std::fs::write(&temporary, bytes).map_err(|source| Error::Write {
        path: temporary.display().to_string(),
        source,
    })?;

    if path.exists() {
        let backup = path.with_extension("bak");
        let _ = std::fs::rename(path, backup);
    }

    std::fs::rename(&temporary, path).map_err(|source| Error::Write {
        path: path.display().to_string(),
        source,
    })
}
