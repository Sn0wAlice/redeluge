// SPDX-License-Identifier: GPL-3.0-or-later
//! Reading and writing `hostlist.conf`.
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

/// Writes the host list back, keeping the file's other keys.
///
/// The file is at format version 3 and Deluge reads it; writing a different
/// shape would make a Python client refuse it.
pub async fn save(config_dir: &std::path::Path, hosts: &[Host]) -> std::io::Result<()> {
    let path = config_dir.join("hostlist.conf");
    let mut config = ConfigFile::load(&path).unwrap_or_else(|_| ConfigFile {
        path: path.clone(),
        version: serde_json::Map::new(),
        settings: serde_json::Map::new(),
    });

    let entries: Vec<serde_json::Value> = hosts
        .iter()
        .map(|host| {
            serde_json::json!([host.id, host.host, host.port, host.username, host.password])
        })
        .collect();
    config
        .settings
        .insert("hosts".to_owned(), serde_json::Value::Array(entries));

    if config.version.is_empty() {
        config
            .version
            .insert("file".to_owned(), serde_json::json!(3));
        config
            .version
            .insert("format".to_owned(), serde_json::json!(1));
    }

    crate::persist::save_config(&config)
        .await
        .map_err(|err| std::io::Error::other(err.to_string()))
}

/// An id for a new host: the same shape Deluge used, a random 32-character
/// hex string, because a client may key its own state on it.
pub fn new_id() -> String {
    let mut bytes = [0u8; 16];
    ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes)
        .expect("the system random source");
    hex::encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_id_is_thirty_two_hex_characters_and_does_not_repeat() {
        let first = new_id();
        assert_eq!(first.len(), 32);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(first, new_id());
    }

    #[tokio::test]
    async fn hosts_round_trip_through_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let hosts = vec![Host {
            id: "abc".to_owned(),
            host: "127.0.0.1".to_owned(),
            port: 58846,
            username: "localclient".to_owned(),
            password: "secret".to_owned(),
        }];

        save(dir.path(), &hosts).await.unwrap();
        let config = ConfigFile::load(dir.path().join("hostlist.conf")).unwrap();
        let read_back = load(&config);

        assert_eq!(read_back.len(), 1);
        assert_eq!(read_back[0].id, "abc");
        assert_eq!(read_back[0].port, 58846);
        assert_eq!(read_back[0].username, "localclient");
    }

    #[tokio::test]
    async fn the_file_is_written_at_the_version_deluge_expects() {
        let dir = tempfile::tempdir().unwrap();
        save(dir.path(), &[]).await.unwrap();

        let text = std::fs::read_to_string(dir.path().join("hostlist.conf")).unwrap();
        assert!(text.contains("\"file\": 3"), "{text}");
    }
}
