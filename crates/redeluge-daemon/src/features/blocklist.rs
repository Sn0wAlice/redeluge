// SPDX-License-Identifier: GPL-3.0-or-later
//! The peer block list: a downloaded list of address ranges, fed to
//! libtorrent's IP filter.
//!
//! Deluge's Blocklist plugin, as a daemon feature. The list is downloaded,
//! decompressed if it needs it, parsed into ranges and installed. The cached
//! copy on disk is what a restart loads, so a daemon that starts without a
//! network still filters.
//!
//! Two things here are worth knowing before changing them. The whitelist is
//! not subtracted from the ranges: it is appended as allowing rules, because
//! libtorrent applies rules in order and a later one wins, which is what makes
//! a hole in a blocked range. And a line that will not parse is counted and
//! skipped rather than failing the import, because a public list of two
//! hundred thousand lines usually has a few.

use std::net::IpAddr;
use std::path::{Path, PathBuf};

use redeluge_libtorrent::IpRange;
use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

/// How the list was packed. Detected from the first two bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compression {
    None,
    Gzip,
    Zip,
    Bzip2,
}

impl Compression {
    pub fn detect(bytes: &[u8]) -> Self {
        match bytes {
            [0x1f, 0x8b, ..] => Self::Gzip,
            [b'P', b'K', ..] => Self::Zip,
            [b'B', b'Z', ..] => Self::Bzip2,
            _ => Self::None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Gzip => "gzip",
            Self::Zip => "zip",
            Self::Bzip2 => "bzip2",
        }
    }
}

/// How the ranges are written. Both formats Deluge could read are text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// `Some organisation:1.2.3.4-5.6.7.8`, also called SafePeer or p2p.
    PeerGuardian,
    /// `001.002.003.004 - 005.006.007.008 , 000 , Some organisation`.
    Emule,
}

impl Format {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PeerGuardian => "peerguardian",
            Self::Emule => "emule",
        }
    }

    /// The format of a list, from its first line that says anything.
    ///
    /// The plugin decided this the same way, by trying each reader on the
    /// first line that is neither blank nor a comment.
    pub fn detect(text: &str) -> Option<Self> {
        let line = text.lines().map(str::trim).find(|line| !is_ignored(line))?;
        [Self::PeerGuardian, Self::Emule]
            .into_iter()
            .find(|format| format.parse_line(line).is_some())
    }

    /// One line to one range, or nothing if it does not parse.
    pub fn parse_line(self, line: &str) -> Option<(IpAddr, IpAddr)> {
        let range = match self {
            // Everything after the last colon, so a name containing one is
            // not a problem. A name is not required.
            Self::PeerGuardian => line.rsplit(':').next()?,
            // The first comma-separated field, before the level and the name.
            Self::Emule => line.split(" , ").next()?,
        };

        let (first, last) = match self {
            Self::PeerGuardian => range.split_once('-')?,
            Self::Emule => range.split_once(" - ")?,
        };
        let first = parse_address(first)?;
        let last = parse_address(last)?;

        // A range that runs backwards, or crosses address families, is a
        // corrupt line rather than something to pass to libtorrent, which
        // asserts on the second case.
        if first.is_ipv4() != last.is_ipv4() || greater(&first, &last) {
            return None;
        }
        Some((first, last))
    }
}

fn greater(first: &IpAddr, last: &IpAddr) -> bool {
    match (first, last) {
        (IpAddr::V4(a), IpAddr::V4(b)) => a > b,
        (IpAddr::V6(a), IpAddr::V6(b)) => a > b,
        _ => false,
    }
}

/// Blank lines and comments, which both formats allow.
fn is_ignored(line: &str) -> bool {
    let line = line.trim();
    line.is_empty() || line.starts_with('#')
}

/// An address, tolerating the zero-padded form eMule lists are written in.
///
/// `001.002.003.004` is not something Rust's parser accepts, and it is how
/// every eMule list in the wild writes an address.
fn parse_address(text: &str) -> Option<IpAddr> {
    let text = text.trim();
    if let Ok(address) = text.parse::<IpAddr>() {
        return Some(address);
    }
    if text.contains(':') || !text.contains('.') {
        return None;
    }

    let mut octets = [0u8; 4];
    let mut seen = 0;
    for part in text.split('.') {
        if seen == 4 || part.is_empty() || part.len() > 3 {
            return None;
        }
        octets[seen] = part.parse::<u8>().ok()?;
        seen += 1;
    }
    if seen != 4 {
        return None;
    }
    Some(IpAddr::from(octets))
}

/// What one import produced.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Import {
    pub ranges: Vec<(IpAddr, IpAddr)>,
    /// Lines that said something and could not be read.
    pub skipped: usize,
}

/// Parses a whole list.
///
/// The format is detected once from the first line that says anything, and
/// then every line is read as that format, which is what makes a file of the
/// other format come back as skipped lines rather than as nonsense ranges.
pub fn parse(text: &str) -> Result<(Format, Import), Error> {
    let format = Format::detect(text).ok_or(Error::UnknownFormat)?;
    let mut import = Import::default();

    for line in text.lines() {
        if is_ignored(line) {
            continue;
        }
        match format.parse_line(line.trim()) {
            Some(range) => import.ranges.push(range),
            None => import.skipped += 1,
        }
    }
    Ok((format, import))
}

/// Unpacks a downloaded list.
pub fn decompress(bytes: &[u8]) -> Result<Vec<u8>, Error> {
    match Compression::detect(bytes) {
        Compression::None => Ok(bytes.to_vec()),
        Compression::Gzip => {
            use std::io::Read;
            let mut out = Vec::new();
            flate2::read::GzDecoder::new(bytes)
                .read_to_end(&mut out)
                .map_err(|err| Error::Decompress(err.to_string()))?;
            Ok(out)
        }
        other => Err(Error::UnsupportedCompression(other)),
    }
}

/// The rules to install, blocked ranges first and the whitelist over the top.
///
/// Order is the whole point: libtorrent lets a later rule win where it
/// overlaps an earlier one, so the whitelist has to come last or it does
/// nothing at all.
pub fn rules(ranges: &[(IpAddr, IpAddr)], whitelist: &[String]) -> Vec<IpRange> {
    let mut rules: Vec<IpRange> = ranges
        .iter()
        .map(|(first, last)| IpRange {
            first: first.to_string(),
            last: last.to_string(),
            blocked: true,
        })
        .collect();

    for entry in whitelist {
        if let Some((first, last)) = whitelist_range(entry) {
            rules.push(IpRange {
                first: first.to_string(),
                last: last.to_string(),
                blocked: false,
            });
        } else {
            tracing::warn!(entry, "ignoring a whitelist entry that is not an address");
        }
    }
    rules
}

/// A whitelist entry: one address, or a range written either way.
fn whitelist_range(entry: &str) -> Option<(IpAddr, IpAddr)> {
    let entry = entry.trim();
    if let Some((first, last)) = entry.split_once(" - ") {
        return pair(first, last);
    }
    // An IPv6 address is full of colons and hyphens are not part of it, so a
    // hyphen split is only tried when it cannot be one.
    if !entry.contains(':') {
        if let Some((first, last)) = entry.split_once('-') {
            return pair(first, last);
        }
    }
    let single = parse_address(entry)?;
    Some((single, single))
}

fn pair(first: &str, last: &str) -> Option<(IpAddr, IpAddr)> {
    let first = parse_address(first)?;
    let last = parse_address(last)?;
    if first.is_ipv4() != last.is_ipv4() || greater(&first, &last) {
        return None;
    }
    Some((first, last))
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no reader recognises this list")]
    UnknownFormat,
    #[error("{} lists are not supported, only gzip and plain text", .0.as_str())]
    UnsupportedCompression(Compression),
    #[error("could not decompress the list: {0}")]
    Decompress(String),
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
    #[error("could not download the list: {0}")]
    Download(String),
    #[error("the list is not valid UTF-8")]
    NotText,
}

/// The block list configuration, under the `blocklist` key of `core.conf`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub url: String,
    /// How old the cached list may be before it is downloaded again. Zero or
    /// less never refreshes it.
    #[serde(default = "four")]
    pub check_after_days: i64,
    #[serde(default = "one_eighty")]
    pub timeout: u64,
    #[serde(default = "three")]
    pub try_times: u32,
    /// Addresses or ranges that are never blocked, whatever the list says.
    #[serde(default)]
    pub whitelisted: Vec<String>,
    /// When the list was last downloaded, as a Unix timestamp. Written by the
    /// daemon, not by a person.
    #[serde(default)]
    pub last_update: f64,
    /// How many ranges the last import produced. Written by the daemon.
    #[serde(default)]
    pub list_size: i64,
}

fn four() -> i64 {
    4
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
            url: String::new(),
            check_after_days: 4,
            timeout: 180,
            try_times: 3,
            whitelisted: Vec::new(),
            last_update: 0.0,
            list_size: 0,
        }
    }
}

impl Settings {
    pub fn from_config(value: Option<&Json>) -> Self {
        match value {
            Some(value) => serde_json::from_value(value.clone()).unwrap_or_else(|err| {
                tracing::warn!(error = %err, "the blocklist configuration is malformed, ignoring it");
                Self::default()
            }),
            None => Self::default(),
        }
    }

    pub fn default_json() -> Json {
        serde_json::to_value(Self::default()).expect("the defaults serialise")
    }

    /// Whether the cached list is old enough to fetch again.
    ///
    /// A check period of zero or less means never, which is how an operator
    /// pins a list they downloaded themselves.
    pub fn is_stale(&self, now: f64) -> bool {
        if self.check_after_days <= 0 {
            return false;
        }
        if self.last_update <= 0.0 {
            return true;
        }
        now - self.last_update >= self.check_after_days as f64 * 86_400.0
    }

    /// Where the downloaded copy is kept.
    pub fn cache_path(config_dir: &Path) -> PathBuf {
        config_dir.join("blocklist.cache")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn v4(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    // --------------------------------------------------------- line formats

    #[test]
    fn a_peerguardian_line_is_the_range_after_the_last_colon() {
        assert_eq!(
            Format::PeerGuardian.parse_line("Some organisation:1.2.3.4-5.6.7.8"),
            Some((v4("1.2.3.4"), v4("5.6.7.8")))
        );
    }

    #[test]
    fn a_name_containing_a_colon_does_not_confuse_the_reader() {
        // Taking the first colon instead of the last is the obvious way to
        // write this and it is wrong for a great many real entries.
        assert_eq!(
            Format::PeerGuardian.parse_line("Bad Co: Europe:10.0.0.1-10.0.0.255"),
            Some((v4("10.0.0.1"), v4("10.0.0.255")))
        );
    }

    #[test]
    fn a_peerguardian_line_without_a_name_still_parses() {
        assert_eq!(
            Format::PeerGuardian.parse_line("1.2.3.4-1.2.3.9"),
            Some((v4("1.2.3.4"), v4("1.2.3.9")))
        );
    }

    #[test]
    fn an_emule_line_is_the_field_before_the_first_comma() {
        assert_eq!(
            Format::Emule.parse_line("001.002.003.004 - 005.006.007.008 , 000 , Some organisation"),
            Some((v4("1.2.3.4"), v4("5.6.7.8")))
        );
    }

    #[test]
    fn zero_padded_addresses_are_accepted() {
        // Rust's own parser rejects these, and every eMule list is written
        // this way, so a list would import as zero ranges without this.
        assert_eq!(parse_address("001.002.003.004"), Some(v4("1.2.3.4")));
        assert_eq!(parse_address("010.000.000.001"), Some(v4("10.0.0.1")));
    }

    #[test]
    fn an_octet_out_of_range_is_not_an_address() {
        assert_eq!(parse_address("1.2.3.256"), None);
        assert_eq!(parse_address("1.2.3"), None);
        assert_eq!(parse_address("1.2.3.4.5"), None);
        assert_eq!(parse_address(""), None);
    }

    #[test]
    fn a_range_that_runs_backwards_is_refused() {
        assert_eq!(Format::PeerGuardian.parse_line("Bad:5.6.7.8-1.2.3.4"), None);
    }

    #[test]
    fn a_range_that_mixes_address_families_is_refused() {
        // libtorrent asserts on this pair rather than reporting it.
        assert_eq!(
            Format::PeerGuardian.parse_line("Mixed:1.2.3.4-2001:db8::1"),
            None
        );
    }

    // ------------------------------------------------------------ detection

    #[test]
    fn the_format_is_detected_from_the_first_line_that_says_something() {
        let list = "# a comment\n\n   \nSome org:1.2.3.4-5.6.7.8\n";
        assert_eq!(Format::detect(list), Some(Format::PeerGuardian));

        let list = "# header\n001.002.003.004 - 005.006.007.008 , 000 , Some org\n";
        assert_eq!(Format::detect(list), Some(Format::Emule));
    }

    #[test]
    fn a_file_that_is_not_a_list_has_no_format() {
        assert_eq!(Format::detect("hello\nworld\n"), None);
        assert_eq!(Format::detect("# only comments\n"), None);
        assert_eq!(Format::detect(""), None);
    }

    // ---------------------------------------------------------- whole lists

    #[test]
    fn a_peerguardian_list_parses_and_counts_what_it_skipped() {
        let list = concat!(
            "# Example list\n",
            "One:1.2.3.4-1.2.3.9\n",
            "\n",
            "Two:10.0.0.0-10.255.255.255\n",
            "this line is rubbish\n",
            "Three:192.168.0.1-192.168.0.1\n",
        );
        let (format, import) = parse(list).unwrap();
        assert_eq!(format, Format::PeerGuardian);
        assert_eq!(import.ranges.len(), 3);
        assert_eq!(import.skipped, 1);
        assert_eq!(import.ranges[2], (v4("192.168.0.1"), v4("192.168.0.1")));
    }

    #[test]
    fn a_list_of_the_other_format_reports_skipped_lines_rather_than_wrong_ranges() {
        // An eMule list read as PeerGuardian would otherwise produce ranges
        // built out of the level and the name.
        let list = concat!(
            "001.002.003.004 - 005.006.007.008 , 000 , One\n",
            "Two:10.0.0.1-10.0.0.9\n",
        );
        let (format, import) = parse(list).unwrap();
        assert_eq!(format, Format::Emule);
        assert_eq!(import.ranges.len(), 1);
        assert_eq!(import.skipped, 1);
    }

    #[test]
    fn a_file_no_reader_recognises_is_an_error() {
        assert!(matches!(parse("nothing here\n"), Err(Error::UnknownFormat)));
    }

    #[test]
    fn a_large_list_parses_whole() {
        let mut list = String::from("# generated\n");
        for n in 0..5_000u32 {
            let a = (n >> 8) as u8;
            let b = (n & 0xff) as u8;
            list.push_str(&format!("Org {n}:10.{a}.{b}.0-10.{a}.{b}.255\n"));
        }
        let (_, import) = parse(&list).unwrap();
        assert_eq!(import.ranges.len(), 5_000);
        assert_eq!(import.skipped, 0);
    }

    // --------------------------------------------------------- compression

    #[test]
    fn compression_is_detected_from_the_first_two_bytes() {
        assert_eq!(Compression::detect(b"\x1f\x8brest"), Compression::Gzip);
        assert_eq!(Compression::detect(b"PK\x03\x04"), Compression::Zip);
        assert_eq!(Compression::detect(b"BZh9"), Compression::Bzip2);
        assert_eq!(
            Compression::detect(b"One:1.2.3.4-1.2.3.9"),
            Compression::None
        );
        assert_eq!(Compression::detect(b""), Compression::None);
    }

    #[test]
    fn a_gzipped_list_is_unpacked() {
        use std::io::Write;
        let plain = b"One:1.2.3.4-1.2.3.9\n";
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(plain).unwrap();
        let packed = encoder.finish().unwrap();

        assert_eq!(decompress(&packed).unwrap(), plain);
    }

    #[test]
    fn plain_text_passes_through_decompression_unchanged() {
        let plain = b"One:1.2.3.4-1.2.3.9\n";
        assert_eq!(decompress(plain).unwrap(), plain);
    }

    #[test]
    fn an_unsupported_archive_says_which_one_it_is() {
        let err = decompress(b"PK\x03\x04rest").unwrap_err();
        assert!(err.to_string().contains("zip"), "{err}");
    }

    #[test]
    fn a_truncated_gzip_stream_is_an_error_not_a_panic() {
        assert!(decompress(b"\x1f\x8b\x08\x00truncated").is_err());
    }

    // -------------------------------------------------------------- rules

    #[test]
    fn every_range_becomes_a_blocking_rule() {
        let ranges = vec![(v4("1.2.3.4"), v4("1.2.3.9"))];
        let rules = rules(&ranges, &[]);
        assert_eq!(rules.len(), 1);
        assert!(rules[0].blocked);
        assert_eq!(rules[0].first, "1.2.3.4");
        assert_eq!(rules[0].last, "1.2.3.9");
    }

    #[test]
    fn the_whitelist_comes_after_the_blocks_so_it_wins() {
        // Reversed, the whitelist would be overwritten by the very range it
        // exists to punch a hole in, and nobody would notice until a peer was
        // silently unreachable.
        let ranges = vec![(v4("10.0.0.0"), v4("10.255.255.255"))];
        let rules = rules(&ranges, &["10.1.2.3".to_owned()]);
        assert_eq!(rules.len(), 2);
        assert!(rules[0].blocked);
        assert!(!rules[1].blocked);
        assert_eq!(rules[1].first, "10.1.2.3");
        assert_eq!(rules[1].last, "10.1.2.3");
    }

    #[test]
    fn a_whitelist_entry_can_be_a_range_written_either_way() {
        let rules = rules(
            &[],
            &["1.2.3.4 - 1.2.3.9".to_owned(), "5.6.7.1-5.6.7.9".to_owned()],
        );
        assert_eq!(rules.len(), 2);
        assert_eq!(
            (rules[0].first.as_str(), rules[0].last.as_str()),
            ("1.2.3.4", "1.2.3.9")
        );
        assert_eq!(
            (rules[1].first.as_str(), rules[1].last.as_str()),
            ("5.6.7.1", "5.6.7.9")
        );
    }

    #[test]
    fn a_whitelist_entry_that_is_not_an_address_is_dropped_not_fatal() {
        let rules = rules(&[], &["not an address".to_owned(), "1.2.3.4".to_owned()]);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].first, "1.2.3.4");
    }

    #[test]
    fn an_ipv6_whitelist_entry_is_not_split_on_its_own_colons() {
        let rules = rules(&[], &["2001:db8::1".to_owned()]);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].first, "2001:db8::1");
    }

    // ----------------------------------------------------------- settings

    #[test]
    fn a_missing_configuration_blocks_nothing() {
        let settings = Settings::from_config(None);
        assert!(!settings.enabled);
        assert!(settings.url.is_empty());
    }

    #[test]
    fn a_blocklist_conf_from_the_plugin_is_understood_as_it_is() {
        let stored = json!({
            "url": "https://example.invalid/list.gz",
            "load_on_start": true,
            "check_after_days": 7,
            "list_compression": "gzip",
            "list_type": "SafePeer",
            "last_update": 1_700_000_000.0,
            "list_size": 12_345,
            "timeout": 120,
            "try_times": 2,
            "whitelisted": ["10.0.0.1"],
        });
        let settings = Settings::from_config(Some(&stored));
        assert_eq!(settings.url, "https://example.invalid/list.gz");
        assert_eq!(settings.check_after_days, 7);
        assert_eq!(settings.try_times, 2);
        assert_eq!(settings.whitelisted, vec!["10.0.0.1".to_owned()]);
        // The two keys the plugin needed and this does not are detected per
        // import instead, so they are simply ignored.
    }

    #[test]
    fn a_list_that_has_never_been_downloaded_is_stale() {
        let settings = Settings::default();
        assert!(settings.is_stale(1_700_000_000.0));
    }

    #[test]
    fn a_list_is_stale_once_the_check_period_has_passed() {
        let settings = Settings {
            last_update: 1_700_000_000.0,
            check_after_days: 4,
            ..Settings::default()
        };
        assert!(!settings.is_stale(1_700_000_000.0 + 3.0 * 86_400.0));
        assert!(settings.is_stale(1_700_000_000.0 + 4.0 * 86_400.0));
    }

    #[test]
    fn a_check_period_of_zero_pins_the_list() {
        let settings = Settings {
            check_after_days: 0,
            ..Settings::default()
        };
        assert!(!settings.is_stale(1_700_000_000.0), "never refetched");
    }
}
