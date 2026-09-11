// SPDX-License-Identifier: GPL-3.0-or-later
//! Reading Deluge's configuration files.
//!
//! A Deluge config file is two JSON objects concatenated: a version header,
//! then the settings. Nothing else writes that shape, so it is parsed here
//! rather than handed to a JSON library that would stop at the first object.
//!
//! Existing files are read as they are. An installation does not have to be
//! converted to run this server, which is the whole point of replacing one
//! process at a time.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value as Json};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("could not read {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("{path} is not a Deluge config file: {reason}")]
    Malformed { path: PathBuf, reason: String },
}

pub type Result<T> = std::result::Result<T, Error>;

/// A parsed config file: its version header and its settings.
#[derive(Debug, Clone)]
pub struct ConfigFile {
    pub path: PathBuf,
    pub version: Map<String, Json>,
    pub settings: Map<String, Json>,
}

impl ConfigFile {
    /// Reads a config file, or returns empty settings when it does not exist.
    ///
    /// A missing file is normal: Deluge writes `web.conf` only once something
    /// has been changed from the defaults.
    pub fn load(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();

        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self {
                    path,
                    version: Map::new(),
                    settings: Map::new(),
                })
            }
            Err(source) => return Err(Error::Read { path, source }),
        };

        Self::parse(&text, path)
    }

    fn parse(text: &str, path: PathBuf) -> Result<Self> {
        let objects = split_json_objects(text);

        let malformed = |reason: &str| Error::Malformed {
            path: path.clone(),
            reason: reason.to_owned(),
        };

        let (version, settings) = match objects.len() {
            // Older files hold only the settings.
            1 => (Map::new(), parse_object(&objects[0], &path)?),
            2 => (
                parse_object(&objects[0], &path)?,
                parse_object(&objects[1], &path)?,
            ),
            0 => return Err(malformed("no JSON object found")),
            count => {
                return Err(Error::Malformed {
                    path,
                    reason: format!("expected one or two JSON objects, found {count}"),
                })
            }
        };

        Ok(Self {
            path,
            version,
            settings,
        })
    }

    pub fn get(&self, key: &str) -> Option<&Json> {
        self.settings.get(key)
    }

    pub fn string(&self, key: &str) -> Option<&str> {
        self.get(key).and_then(Json::as_str)
    }

    pub fn integer(&self, key: &str) -> Option<i64> {
        self.get(key).and_then(Json::as_i64)
    }

    pub fn boolean(&self, key: &str) -> Option<bool> {
        self.get(key).and_then(Json::as_bool)
    }
}

fn parse_object(text: &str, path: &Path) -> Result<Map<String, Json>> {
    let parsed: Json = serde_json::from_str(text).map_err(|err| Error::Malformed {
        path: path.to_path_buf(),
        reason: err.to_string(),
    })?;
    match parsed {
        Json::Object(map) => Ok(map),
        other => Err(Error::Malformed {
            path: path.to_path_buf(),
            reason: format!("expected an object, found {other}"),
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

/// The directory Deluge keeps its configuration in.
pub fn config_dir() -> PathBuf {
    if let Ok(explicit) = std::env::var("DELUGE_CONFIG_DIR") {
        if !explicit.is_empty() {
            return PathBuf::from(explicit);
        }
    }
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return PathBuf::from(xdg).join("deluge");
        }
    }
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() => PathBuf::from(home).join(".config").join("deluge"),
        _ => PathBuf::from("/config"),
    }
}
