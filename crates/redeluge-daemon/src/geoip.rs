// SPDX-License-Identifier: GPL-3.0-or-later
//! Which country a peer is in.
//!
//! libtorrent does not report this; Deluge looked it up itself, in a
//! legacy-format `GeoIP.dat`. MaxMind retired that format years ago, so what
//! is read here is the MaxMind DB format that replaced it, which is what any
//! database someone can download today will be.
//!
//! The database is not shipped, and cannot be: its licence does not allow it.
//! `geoip_db_location` in `core.conf` points at one if the operator has
//! provided it, and the lookup is simply absent otherwise. That is why a
//! missing database is not an error anywhere in here.

use std::net::IpAddr;
use std::path::Path;
use std::sync::Arc;

/// A loaded country database.
#[derive(Clone)]
pub struct CountryLookup {
    reader: Arc<maxminddb::Reader<Vec<u8>>>,
}

impl std::fmt::Debug for CountryLookup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CountryLookup")
    }
}

impl CountryLookup {
    /// Opens a database, if the path names one this can read.
    ///
    /// Returns nothing rather than an error for every failure: no database is
    /// the ordinary case, and a daemon that refused to start because a country
    /// column would be empty would be absurd.
    pub fn open(path: &Path) -> Option<Self> {
        if !path.is_file() {
            return None;
        }
        // The path Deluge shipped as its default names the retired format. It
        // is worth saying so rather than reporting a parse failure.
        match maxminddb::Reader::open_readfile(path) {
            Ok(reader) => {
                tracing::info!(path = %path.display(), "loaded a GeoIP database");
                Some(Self {
                    reader: Arc::new(reader),
                })
            }
            Err(err) => {
                let legacy = path
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("dat"));
                if legacy {
                    tracing::warn!(path = %path.display(),
                        "this looks like the legacy GeoIP.dat format, which MaxMind \
                         retired; point geoip_db_location at a .mmdb country database");
                } else {
                    tracing::warn!(path = %path.display(), error = %err,
                        "could not read the GeoIP database");
                }
                None
            }
        }
    }

    /// The two-letter country code for an address.
    pub fn country_of(&self, address: IpAddr) -> Option<String> {
        let country: maxminddb::geoip2::Country = self.reader.lookup(address).ok()?;
        country
            .country
            .and_then(|country| country.iso_code)
            .map(str::to_owned)
    }

    /// The country's English name, for an address.
    ///
    /// Beside the code rather than instead of it: the code is what picks the
    /// flag, and the name is what a person reads in the tooltip. A database
    /// that has the code and no name is normal, so this can be absent on its
    /// own.
    pub fn name_of(&self, address: IpAddr) -> Option<String> {
        let country: maxminddb::geoip2::Country = self.reader.lookup(address).ok()?;
        country
            .country
            .and_then(|country| country.names)
            .and_then(|names| names.get("en").map(|name| (*name).to_owned()))
    }

    /// The name for an address written as text.
    pub fn name_of_text(&self, raw: &str) -> Option<String> {
        parse_peer_address(raw).and_then(|address| self.name_of(address))
    }

    /// The code for an address written as text, which is what a peer carries.
    ///
    /// The peer's address may arrive with a port, and an IPv6 one may be in
    /// brackets, so both are stripped before parsing.
    pub fn country_of_text(&self, raw: &str) -> Option<String> {
        parse_peer_address(raw).and_then(|address| self.country_of(address))
    }
}

/// An address out of a peer's `ip` field.
///
/// libtorrent reports a bare address here, but a client may hand back what it
/// displays, which is `address:port` or `[v6]:port`.
pub fn parse_peer_address(raw: &str) -> Option<IpAddr> {
    let trimmed = raw.trim();
    if let Ok(address) = trimmed.parse::<IpAddr>() {
        return Some(address);
    }
    if let Some(rest) = trimmed.strip_prefix('[') {
        let (inside, _) = rest.split_once(']')?;
        return inside.parse().ok();
    }
    if let Some((host, _port)) = trimmed.rsplit_once(':') {
        // Only for v4: a bare v6 address is full of colons and was handled by
        // the parse above.
        if !host.contains(':') {
            return host.parse().ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_address_parses() {
        assert_eq!(
            parse_peer_address("1.2.3.4"),
            Some("1.2.3.4".parse().unwrap())
        );
        assert_eq!(
            parse_peer_address("2001:db8::1"),
            Some("2001:db8::1".parse().unwrap())
        );
    }

    #[test]
    fn an_address_with_a_port_parses() {
        assert_eq!(
            parse_peer_address("1.2.3.4:6881"),
            Some("1.2.3.4".parse().unwrap())
        );
        assert_eq!(
            parse_peer_address("[2001:db8::1]:6881"),
            Some("2001:db8::1".parse().unwrap())
        );
    }

    #[test]
    fn rubbish_is_not_an_address() {
        assert_eq!(parse_peer_address(""), None);
        assert_eq!(parse_peer_address("not an address"), None);
        assert_eq!(parse_peer_address("1.2.3.4.5"), None);
    }

    #[test]
    fn a_missing_database_is_not_an_error() {
        // The ordinary case: no database is shipped, because its licence does
        // not allow it.
        assert!(CountryLookup::open(Path::new("/no/such/database.mmdb")).is_none());
    }

    #[test]
    fn a_file_that_is_not_a_database_is_not_an_error_either() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("GeoIP.dat");
        std::fs::write(&path, b"not a database").unwrap();
        assert!(CountryLookup::open(&path).is_none());
    }
}
