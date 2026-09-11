// SPDX-License-Identifier: GPL-3.0-or-later
//! Reading `hostlist.conf`.
//!
//! Entries are `[id, host, port, username, password]`. The bootstrap that runs
//! before the daemon starts keeps a localhost entry aligned with the auth file,
//! so the Web UI has credentials without anyone typing them.

use crate::config::ConfigFile;
use crate::state::Host;

/// Reads every host, skipping entries that are not the expected shape.
pub fn load(config: &ConfigFile) -> Vec<Host> {
    let Some(entries) = config.get("hosts").and_then(|value| value.as_array()) else {
        return Vec::new();
    };

    entries
        .iter()
        .filter_map(|entry| {
            let fields = entry.as_array()?;
            Some(Host {
                id: fields.first()?.as_str()?.to_owned(),
                host: fields.get(1)?.as_str()?.to_owned(),
                port: u16::try_from(fields.get(2)?.as_u64()?).ok()?,
                username: fields
                    .get(3)
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_owned(),
                password: fields
                    .get(4)
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_owned(),
            })
        })
        .collect()
}
