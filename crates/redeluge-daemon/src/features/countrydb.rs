// SPDX-License-Identifier: GPL-3.0-or-later
//! Keeping a country database on disk, so peers have flags.
//!
//! libtorrent does not say which country a peer is in, so it has to be looked
//! up, and that needs a database nobody ships. Deluge's answer was to point at
//! `/usr/share/GeoIP/GeoIP.dat`, a file the distribution's `geoip-database`
//! package installed in MaxMind's GeoLite Legacy format. MaxMind retired that
//! format in January 2019 and the distributions dropped the package with it,
//! so on any current system that path holds nothing and Deluge's own flags
//! have been blank for years.
//!
//! What replaced it is MaxMind DB, the `.mmdb` files everything uses now, and
//! that is what the reader in `geoip.rs` takes. MaxMind's own files may not be
//! redistributed, but the format is not theirs alone: DB-IP publish a country
//! database in it every month under CC BY 4.0, which is why that is the
//! default URL here.
//!
//! Off by default, deliberately. Turning it on makes this daemon fetch a file
//! from a third party, and that is the operator's decision to take rather than
//! one to find out about afterwards.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

/// The `countrydb` key of `core.conf`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub enabled: bool,

    /// Where to fetch it from.
    ///
    /// Defaults to DB-IP's free country database, which is CC BY 4.0 and so
    /// may actually be downloaded and used without agreeing to anything.
    #[serde(default = "default_url")]
    pub url: String,

    /// How old the copy may be before it is fetched again.
    ///
    /// The published file is monthly, so a week is frequent enough to catch a
    /// new one and rare enough to be no burden on anybody.
    #[serde(default = "seven")]
    pub check_after_days: i64,

    #[serde(default = "one_eighty")]
    pub timeout: u64,

    #[serde(default = "three")]
    pub try_times: u32,

    /// When it was last fetched, as a Unix timestamp. Written by the daemon.
    #[serde(default)]
    pub last_update: f64,
}

fn default_url() -> String {
    // The month is filled in when the URL is used; see `resolved_url`.
    "https://download.db-ip.com/free/dbip-country-lite-{YYYY-MM}.mmdb.gz".to_owned()
}
fn seven() -> i64 {
    7
}
fn one_eighty() -> u64 {
    180
}
fn three() -> u32 {
    3
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            url: default_url(),
            check_after_days: seven(),
            timeout: one_eighty(),
            try_times: three(),
            last_update: 0.0,
        }
    }
}

impl Settings {
    pub fn from_config(value: Option<&Json>) -> Self {
        match value {
            Some(value) => {
                serde_json::from_value(super::without_nulls(value)).unwrap_or_else(|err| {
                    super::warn_malformed("countrydb", &err.to_string());
                    Self::default()
                })
            }
            None => Self::default(),
        }
    }

    pub fn default_json() -> Json {
        serde_json::to_value(Self::default()).expect("the defaults serialise")
    }

    /// Where the downloaded database is kept.
    ///
    /// Beside the configuration rather than in the state directory: it is not
    /// per torrent and losing it costs a download, not data.
    pub fn cache_path(config_dir: &Path) -> PathBuf {
        config_dir.join("country.mmdb")
    }

    /// The URL to fetch, with `{YYYY-MM}` replaced.
    ///
    /// DB-IP publish one file per month at a dated URL, so the default cannot
    /// be a fixed string. A URL without the marker is used as it stands, which
    /// is what someone pointing at their own copy will have written.
    pub fn resolved_url(&self, year: i32, month: u32) -> String {
        self.url
            .replace("{YYYY-MM}", &format!("{year:04}-{month:02}"))
    }

    /// Whether the copy on disk is old enough to fetch again.
    ///
    /// Zero or less never refetches, which is how someone pins a file they
    /// provided themselves.
    pub fn is_stale(&self, now: f64) -> bool {
        if self.check_after_days <= 0 {
            return false;
        }
        if self.last_update <= 0.0 {
            return true;
        }
        now - self.last_update >= self.check_after_days as f64 * 86_400.0
    }
}

/// Whether these bytes look like a MaxMind DB file.
///
/// Checked before the download replaces a working database: a proxy's error
/// page saved over it would leave the daemon with no lookup and nothing to say
/// why. The format ends with a marker followed by the metadata, which is
/// cheaper to look for than parsing the whole file.
pub fn looks_like_a_database(bytes: &[u8]) -> bool {
    const MARKER: &[u8] = b"\xab\xcd\xefMaxMind.com";
    // The metadata sits in the last 128 KiB by the format's own rule.
    let tail = bytes.len().saturating_sub(128 * 1024);
    bytes[tail..]
        .windows(MARKER.len())
        .any(|window| window == MARKER)
}

/// Unpacks a gzip stream, which is how the published file arrives.
pub fn gunzip(bytes: &[u8]) -> Option<Vec<u8>> {
    use std::io::Read;

    // Not gzipped: some mirrors serve the plain file, and so does anyone
    // pointing at their own.
    if bytes.len() < 2 || bytes[0] != 0x1f || bytes[1] != 0x8b {
        return Some(bytes.to_vec());
    }

    let mut out = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_end(&mut out)
        .ok()?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn it_is_off_until_somebody_turns_it_on() {
        // Turning it on is the operator agreeing to an outbound request.
        assert!(!Settings::default().enabled);
    }

    #[test]
    fn the_month_is_filled_in_because_the_file_is_monthly() {
        let settings = Settings::default();
        assert_eq!(
            settings.resolved_url(2026, 9),
            "https://download.db-ip.com/free/dbip-country-lite-2026-09.mmdb.gz"
        );
        assert!(settings.resolved_url(2026, 12).contains("2026-12"));
    }

    #[test]
    fn a_url_of_your_own_is_left_alone() {
        let settings = Settings {
            url: "https://example.invalid/country.mmdb".to_owned(),
            ..Settings::default()
        };
        assert_eq!(
            settings.resolved_url(2026, 9),
            "https://example.invalid/country.mmdb"
        );
    }

    #[test]
    fn a_file_you_provided_yourself_is_never_refetched() {
        let settings = Settings {
            check_after_days: 0,
            ..Settings::default()
        };
        assert!(!settings.is_stale(1_800_000_000.0));
    }

    #[test]
    fn one_that_has_never_been_fetched_is_stale() {
        assert!(Settings::default().is_stale(1_800_000_000.0));
    }

    #[test]
    fn an_error_page_is_not_mistaken_for_a_database() {
        // The case this exists for: something answers 200 with HTML, and
        // writing it over a working database would leave no lookup at all.
        assert!(!looks_like_a_database(b"<html>404 not found</html>"));
        assert!(!looks_like_a_database(b""));

        let mut file = vec![0u8; 4096];
        file.extend_from_slice(b"\xab\xcd\xefMaxMind.com");
        file.extend_from_slice(b"whatever the metadata is");
        assert!(looks_like_a_database(&file));
    }

    #[test]
    fn a_plain_file_passes_through_the_unpacker() {
        assert_eq!(gunzip(b"not gzip").as_deref(), Some(&b"not gzip"[..]));
    }

    #[test]
    fn the_stored_shape_round_trips() {
        let settings = Settings::from_config(Some(&json!({
            "enabled": true,
            "url": "https://example.invalid/db.mmdb",
            "last_update": 1_800_000_000.0
        })));
        assert!(settings.enabled);
        assert_eq!(settings.check_after_days, seven());
        assert_eq!(settings.last_update, 1_800_000_000.0);
    }
}
