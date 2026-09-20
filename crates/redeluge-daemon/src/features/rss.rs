// SPDX-License-Identifier: GPL-3.0-or-later
//! Feeds, and the rules that pull torrents out of them.
//!
//! The watched folders cover the case where something else decides what to
//! download and drops a file in a directory. This is the other case: a feed
//! somebody follows, and a line saying which of its items are wanted.
//!
//! Three deliberate limits, because this is a rule that adds torrents by
//! itself and a rule like that has to be boring:
//!
//! * **A pattern is required.** A rule with an empty pattern would take every
//!   item in the feed, which is a way to fill a disk by leaving a field blank.
//! * **The first look at a feed downloads nothing.** Everything already in it
//!   is marked as seen instead. A feed's history is not a wish list, and a new
//!   feed with fifty items in it should not be fifty torrents.
//! * **An item is acted on once.** What has been seen is remembered per feed,
//!   capped, and kept across restarts.
//!
//! The feed is read with a scanner rather than an XML parser. What is needed
//! per item is a title, a link and something stable to call it by, and the two
//! formats put those in the same few tags. The ceiling is known: a feed that
//! nests an `<item>` inside an `<item>`, or that puts its title in an
//! attribute, is not understood, and no feed does either. Adding an XML
//! dependency to the daemon for the tags below would cost more than it buys.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

/// One feed to read.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Feed {
    /// What the rules call it. Blank names are allowed and match the rule that
    /// names no feed, which is the one that applies to all of them.
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub url: String,
    #[serde(default = "yes")]
    pub enabled: bool,
}

/// One rule: which items are wanted, and what to do with them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    #[serde(default)]
    pub name: String,
    /// The feed this applies to, by name. Empty means every feed.
    #[serde(default)]
    pub feed: String,
    /// What a title has to contain. `*` stands for anything; the comparison
    /// ignores case. An empty pattern matches nothing at all, deliberately.
    #[serde(default)]
    pub contains: String,
    /// What a title must not contain, under the same rules. Empty excludes
    /// nothing.
    #[serde(default)]
    pub excludes: String,
    /// The label to put what it takes in. Empty leaves the label alone.
    #[serde(default)]
    pub label: String,
    /// Where the files go. Empty uses the daemon's download folder.
    #[serde(default)]
    pub save_path: String,
    /// Add it stopped, so somebody looks before it starts.
    #[serde(default)]
    pub paused: bool,
    #[serde(default = "yes")]
    pub enabled: bool,
}

fn yes() -> bool {
    true
}

impl Default for Rule {
    fn default() -> Self {
        Self {
            name: String::new(),
            feed: String::new(),
            contains: String::new(),
            excludes: String::new(),
            label: String::new(),
            save_path: String::new(),
            paused: false,
            // On when it is made: a rule somebody has just added is one they
            // are about to fill in, not one they want ignored.
            enabled: true,
        }
    }
}

impl Rule {
    /// Whether this rule takes an item of this feed with this title.
    pub fn takes(&self, feed: &str, title: &str) -> bool {
        if !self.enabled || self.contains.trim().is_empty() {
            return false;
        }
        if !self.feed.trim().is_empty() && !self.feed.eq_ignore_ascii_case(feed.trim()) {
            return false;
        }
        if !matches(&self.contains, title) {
            return false;
        }
        if !self.excludes.trim().is_empty() && matches(&self.excludes, title) {
            return false;
        }
        true
    }
}

/// The `rss` key of `core.conf`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    /// Nothing is read at all until this is on.
    #[serde(default)]
    pub enabled: bool,
    /// Minutes between passes over the feeds.
    #[serde(default = "thirty")]
    pub interval: u64,
    #[serde(default)]
    pub feeds: Vec<Feed>,
    #[serde(default)]
    pub rules: Vec<Rule>,
}

fn thirty() -> u64 {
    30
}

impl Settings {
    pub fn from_config(value: Option<&Json>) -> Self {
        match value {
            Some(value) => {
                serde_json::from_value(super::without_nulls(value)).unwrap_or_else(|err| {
                    super::warn_malformed("rss", &err.to_string());
                    Self::default()
                })
            }
            None => Self::default(),
        }
    }

    pub fn default_json() -> Json {
        serde_json::to_value(Self {
            interval: thirty(),
            ..Self::default()
        })
        .expect("the defaults serialise")
    }

    pub fn to_json(&self) -> Json {
        serde_json::to_value(self).expect("the settings serialise")
    }

    /// The settings with anything unusable taken out.
    ///
    /// A feed with no address, a rule with no pattern: both are what a
    /// half-filled form leaves behind, and neither is something to act on.
    pub fn sane(mut self) -> Self {
        self.interval = self.interval.clamp(1, 24 * 60);
        self.feeds.retain(|feed| !feed.url.trim().is_empty());
        self.rules.retain(|rule| !rule.contains.trim().is_empty());
        self
    }

    /// Whether there is anything to do at all.
    pub fn worth_reading(&self) -> bool {
        self.enabled
            && self.feeds.iter().any(|feed| feed.enabled)
            && self.rules.iter().any(|rule| rule.enabled)
    }

    /// The first rule that takes this item, if any.
    ///
    /// First rather than every: two rules that both want an item disagree
    /// about where it goes, and adding it twice is not an answer.
    pub fn rule_for(&self, feed: &str, title: &str) -> Option<&Rule> {
        self.rules.iter().find(|rule| rule.takes(feed, title))
    }
}

/// Whether a title matches a pattern.
///
/// Case-insensitive, with `*` standing for any run of characters. Not a
/// regular expression: a pattern is something somebody types into a box while
/// looking at the titles in a feed, and `Winter.Harbour*1080p` is what they
/// mean. An empty pattern matches nothing, which is what stops a rule with a
/// blank field from taking the whole feed.
pub fn matches(pattern: &str, title: &str) -> bool {
    let pattern = pattern.trim().to_lowercase();
    if pattern.is_empty() {
        return false;
    }
    let title = title.to_lowercase();

    // Split on the wildcards and walk the parts in order. A pattern with no
    // wildcard is one part, which is a plain "contains"; a pattern with one is
    // "this, then that, somewhere later". Nothing is anchored: a title carries
    // a group name and a release date that nobody wants to have to write out.
    let mut at = 0usize;
    for part in pattern.split('*').filter(|part| !part.is_empty()) {
        let Some(found) = title[at..].find(part) else {
            return false;
        };
        at += found + part.len();
    }
    true
}

/// One thing a feed offers.
#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    /// What to call it when remembering that it has been seen: the feed's own
    /// id where there is one, the link otherwise.
    pub id: String,
    pub title: String,
    /// What to add: a `.torrent` address or a magnet link.
    pub link: String,
}

/// The items a feed body offers, in the order it lists them.
///
/// RSS and Atom, which differ in the tag names and not in the shape. An item
/// with no link is dropped: there is nothing to add.
pub fn items(body: &str) -> Vec<Item> {
    let mut out = Vec::new();
    for block in blocks(body, "item")
        .into_iter()
        .chain(blocks(body, "entry"))
    {
        let title = text_of(&block, "title").unwrap_or_default();
        // An enclosure is the torrent itself, where a feed offers one; the
        // link is usually the page about it, so the enclosure wins.
        let link = attribute_of(&block, "enclosure", "url")
            .or_else(|| attribute_of(&block, "link", "href"))
            .or_else(|| text_of(&block, "link"))
            .unwrap_or_default();
        if link.trim().is_empty() {
            continue;
        }
        let id = text_of(&block, "guid")
            .or_else(|| text_of(&block, "id"))
            .unwrap_or_else(|| link.clone());
        out.push(Item {
            id: id.trim().to_owned(),
            title: title.trim().to_owned(),
            link: link.trim().to_owned(),
        });
    }
    out
}

/// The inside of every `<tag>...</tag>` in the body.
fn blocks(body: &str, tag: &str) -> Vec<String> {
    let open = format!("<{tag}");
    let close = format!("</{tag}>");
    let mut out = Vec::new();
    let mut rest = body;

    while let Some(start) = rest.find(&open) {
        // `<item` must not match `<items`: the character after the name has to
        // end it.
        let after = rest[start + open.len()..].chars().next();
        if !matches!(after, Some(c) if c.is_whitespace() || c == '>' || c == '/') {
            rest = &rest[start + open.len()..];
            continue;
        }
        let Some(body_start) = rest[start..].find('>').map(|at| start + at + 1) else {
            break;
        };
        let Some(end) = rest[body_start..].find(&close).map(|at| body_start + at) else {
            break;
        };
        out.push(rest[body_start..end].to_owned());
        rest = &rest[end + close.len()..];
    }
    out
}

/// The text of the first `<tag>` in a block, with CDATA and entities resolved.
fn text_of(block: &str, tag: &str) -> Option<String> {
    let inner = blocks(block, tag).into_iter().next()?;
    let inner = inner.trim();
    let inner = inner
        .strip_prefix("<![CDATA[")
        .and_then(|rest| rest.strip_suffix("]]>"))
        .unwrap_or(inner);
    let text = unescape(inner.trim());
    (!text.is_empty()).then_some(text)
}

/// The value of an attribute on the first `<tag ...>` in a block.
fn attribute_of(block: &str, tag: &str, attribute: &str) -> Option<String> {
    let open = format!("<{tag}");
    let mut rest = block;
    while let Some(start) = rest.find(&open) {
        let after = &rest[start + open.len()..];
        if !matches!(after.chars().next(), Some(c) if c.is_whitespace() || c == '>' || c == '/') {
            rest = after;
            continue;
        }
        let end = after.find('>').unwrap_or(after.len());
        let attributes = &after[..end];
        let wanted = format!("{attribute}=");
        if let Some(at) = attributes.find(&wanted) {
            let value = attributes[at + wanted.len()..].trim_start();
            let quote = value.chars().next()?;
            if quote == '"' || quote == '\'' {
                let value = &value[1..];
                if let Some(close) = value.find(quote) {
                    let text = unescape(&value[..close]);
                    if !text.trim().is_empty() {
                        return Some(text);
                    }
                }
            }
        }
        rest = after;
    }
    None
}

/// The five entities XML defines. A feed that uses a numeric one for a
/// character in a title is left as it is: the title is compared against a
/// pattern somebody typed, not rendered.
fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        // Last, or an escaped ampersand would be unescaped twice.
        .replace("&amp;", "&")
}

/// What has already been acted on, per feed.
///
/// Capped: a feed that publishes daily for a year is three hundred and
/// sixty-five ids, and there is no reason to keep the ones that have scrolled
/// off the end of it. The cap is per feed and generous enough that an item
/// cannot come back round while it is still being offered.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Seen {
    #[serde(default)]
    pub feeds: BTreeMap<String, Vec<String>>,
}

/// Ids remembered per feed.
pub const SEEN_PER_FEED: usize = 500;

impl Seen {
    pub fn knows(&self, feed: &str, id: &str) -> bool {
        self.feeds
            .get(feed)
            .is_some_and(|ids| ids.iter().any(|known| known == id))
    }

    /// Remembers an id, dropping the oldest when the cap is reached.
    pub fn remember(&mut self, feed: &str, id: &str) {
        let ids = self.feeds.entry(feed.to_owned()).or_default();
        if ids.iter().any(|known| known == id) {
            return;
        }
        ids.push(id.to_owned());
        if ids.len() > SEEN_PER_FEED {
            let excess = ids.len() - SEEN_PER_FEED;
            ids.drain(..excess);
        }
    }

    /// Whether this feed has ever been read.
    ///
    /// The first pass marks without adding, so that following a feed does not
    /// download its back catalogue.
    pub fn read_before(&self, feed: &str) -> bool {
        self.feeds.contains_key(feed)
    }

    /// Starts a feed off as read, without acting on anything in it.
    pub fn start(&mut self, feed: &str) {
        self.feeds.entry(feed.to_owned()).or_default();
    }

    /// Feeds that are no longer configured stop being remembered.
    pub fn keep_only(&mut self, feeds: &[String]) {
        self.feeds
            .retain(|name, _| feeds.iter().any(|kept| kept == name));
    }
}

impl Seen {
    /// Where the record of what has been acted on lives.
    fn path(config_dir: &std::path::Path) -> std::path::PathBuf {
        config_dir.join("state").join("rss.json")
    }

    /// Reads it back, or starts empty.
    ///
    /// Empty is not the same as missing in one respect that matters: a feed
    /// this does not know is a feed whose first pass downloads nothing. A
    /// file that cannot be read therefore costs one quiet pass per feed, not
    /// a library of back catalogue.
    pub fn load(config_dir: &std::path::Path) -> Self {
        let Ok(text) = std::fs::read_to_string(Self::path(config_dir)) else {
            return Self::default();
        };
        serde_json::from_str(&text).unwrap_or_else(|err| {
            tracing::warn!(error = %err, "the rss record is malformed, starting again");
            Self::default()
        })
    }

    pub fn save(&self, config_dir: &std::path::Path) {
        let path = Self::path(config_dir);
        let Some(parent) = path.parent() else { return };
        if let Err(err) = std::fs::create_dir_all(parent)
            .and_then(|()| serde_json::to_vec(self).map_err(std::io::Error::other))
            .and_then(|body| std::fs::write(&path, body))
        {
            tracing::warn!(error = %err, "could not write the rss record");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RSS: &str = r#"<?xml version="1.0"?>
<rss version="2.0"><channel>
  <title>A tracker's feed</title>
  <item>
    <title><![CDATA[Winter.Harbour.S02E04.1080p.WEB-DL.x265-GROUP]]></title>
    <guid isPermaLink="false">abc123</guid>
    <link>https://tracker.invalid/torrents/1</link>
    <enclosure url="https://tracker.invalid/download/1.torrent" type="application/x-bittorrent"/>
  </item>
  <item>
    <title>Winter.Harbour.S02E04.720p.WEB-DL.x265-GROUP</title>
    <link>magnet:?xt=urn:btih:1111111111111111111111111111111111111111&amp;dn=small</link>
  </item>
  <item>
    <title>Nothing to download here</title>
  </item>
</channel></rss>"#;

    const ATOM: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<feed xmlns="http://www.w3.org/2005/Atom">
  <entry>
    <title>The Long Field S01E01 2160p</title>
    <id>tag:tracker.invalid,2026:1</id>
    <link rel="alternate" href="https://tracker.invalid/download/2.torrent"/>
  </entry>
</feed>"#;

    #[test]
    fn an_rss_feed_gives_up_its_items() {
        let found = items(RSS);
        assert_eq!(found.len(), 2, "the item with no link should be dropped");

        assert_eq!(
            found[0].title,
            "Winter.Harbour.S02E04.1080p.WEB-DL.x265-GROUP"
        );
        assert_eq!(found[0].id, "abc123");
        assert_eq!(
            found[0].link, "https://tracker.invalid/download/1.torrent",
            "the enclosure is the torrent; the link is the page about it"
        );

        assert!(found[1].link.starts_with("magnet:?xt="));
        assert!(
            found[1].link.contains("&dn=small"),
            "the escaped ampersand was not put back: {}",
            found[1].link
        );
        assert_eq!(
            found[1].id, found[1].link,
            "with no guid, the link names it"
        );
    }

    #[test]
    fn an_atom_feed_gives_up_its_entries() {
        let found = items(ATOM);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].title, "The Long Field S01E01 2160p");
        assert_eq!(found[0].id, "tag:tracker.invalid,2026:1");
        assert_eq!(found[0].link, "https://tracker.invalid/download/2.torrent");
    }

    #[test]
    fn a_feed_that_is_not_one_gives_nothing_rather_than_panicking() {
        for body in [
            "",
            "not xml at all",
            "<rss><channel><item>",
            "<item></item>",
        ] {
            let _ = items(body);
        }
    }

    #[test]
    fn a_pattern_is_a_contains_with_wildcards() {
        assert!(matches("winter.harbour", "Winter.Harbour.S02E04.1080p"));
        assert!(matches("WINTER", "winter.harbour"));
        assert!(matches("winter*1080p", "Winter.Harbour.S02E04.1080p"));
        assert!(!matches("winter*2160p", "Winter.Harbour.S02E04.1080p"));
        // Order matters: the parts have to appear in the order they are given.
        assert!(!matches("1080p*winter", "Winter.Harbour.S02E04.1080p"));
        assert!(matches("*harbour*", "Winter.Harbour.S02E04"));
    }

    #[test]
    fn an_empty_pattern_takes_nothing() {
        // A rule with a blank field would otherwise take the whole feed, which
        // is a way to fill a disk by leaving a box empty.
        assert!(!matches("", "anything at all"));
        assert!(!matches("   ", "anything at all"));
    }

    #[test]
    fn a_rule_takes_what_it_names_and_leaves_the_rest() {
        let rule = Rule {
            name: "series".to_owned(),
            feed: "tracker".to_owned(),
            contains: "winter.harbour*1080p".to_owned(),
            excludes: "*hdtv*".to_owned(),
            label: "series".to_owned(),
            save_path: String::new(),
            paused: false,
            enabled: true,
        };

        assert!(rule.takes("tracker", "Winter.Harbour.S02E04.1080p.WEB-DL"));
        assert!(!rule.takes("tracker", "Winter.Harbour.S02E04.1080p.HDTV.x264"));
        assert!(!rule.takes("another", "Winter.Harbour.S02E04.1080p.WEB-DL"));
        assert!(!rule.takes("tracker", "The Long Field S01E01 1080p"));

        let off = Rule {
            enabled: false,
            ..rule.clone()
        };
        assert!(!off.takes("tracker", "Winter.Harbour.S02E04.1080p.WEB-DL"));

        let every_feed = Rule {
            feed: String::new(),
            ..rule
        };
        assert!(every_feed.takes("whatever", "Winter.Harbour.S02E04.1080p.WEB-DL"));
    }

    #[test]
    fn the_first_rule_that_wants_an_item_gets_it() {
        // Two rules that both want it disagree about where it goes, and adding
        // it twice is not an answer.
        let settings = Settings {
            enabled: true,
            interval: 30,
            feeds: vec![Feed {
                name: "tracker".to_owned(),
                url: "https://tracker.invalid/rss".to_owned(),
                enabled: true,
            }],
            rules: vec![
                Rule {
                    name: "first".to_owned(),
                    contains: "*1080p*".to_owned(),
                    label: "films".to_owned(),
                    enabled: true,
                    ..Default::default()
                },
                Rule {
                    name: "second".to_owned(),
                    contains: "winter*".to_owned(),
                    label: "series".to_owned(),
                    enabled: true,
                    ..Default::default()
                },
            ],
        };

        let taken = settings.rule_for("tracker", "Winter.Harbour.S02E04.1080p");
        assert_eq!(taken.map(|rule| rule.label.as_str()), Some("films"));
        assert!(settings
            .rule_for("tracker", "something else entirely")
            .is_none());
    }

    #[test]
    fn half_filled_forms_are_dropped_rather_than_acted_on() {
        let settings = Settings {
            enabled: true,
            interval: 0,
            feeds: vec![
                Feed {
                    name: "good".to_owned(),
                    url: "https://x.invalid".to_owned(),
                    enabled: true,
                },
                Feed {
                    name: "blank".to_owned(),
                    url: "  ".to_owned(),
                    enabled: true,
                },
            ],
            rules: vec![
                Rule {
                    name: "good".to_owned(),
                    contains: "x".to_owned(),
                    enabled: true,
                    ..Default::default()
                },
                Rule {
                    name: "blank".to_owned(),
                    contains: String::new(),
                    enabled: true,
                    ..Default::default()
                },
            ],
        }
        .sane();

        assert_eq!(settings.feeds.len(), 1);
        assert_eq!(settings.rules.len(), 1);
        assert_eq!(settings.interval, 1, "an interval of zero would be a loop");
    }

    #[test]
    fn what_has_been_seen_is_remembered_and_capped() {
        let mut seen = Seen::default();
        assert!(!seen.read_before("feed"));
        seen.start("feed");
        assert!(
            seen.read_before("feed"),
            "the first pass marks the feed as read"
        );

        for i in 0..(SEEN_PER_FEED + 50) {
            seen.remember("feed", &format!("id-{i}"));
        }
        assert_eq!(seen.feeds["feed"].len(), SEEN_PER_FEED);
        assert!(seen.knows("feed", &format!("id-{}", SEEN_PER_FEED + 49)));
        assert!(!seen.knows("feed", "id-0"), "the oldest should have gone");

        // Remembering the same id twice does not grow the list.
        let before = seen.feeds["feed"].len();
        seen.remember("feed", "id-1000");
        seen.remember("feed", "id-1000");
        assert_eq!(seen.feeds["feed"].len(), before);

        seen.keep_only(&["another".to_owned()]);
        assert!(
            !seen.read_before("feed"),
            "a feed that is gone is forgotten"
        );
    }

    #[test]
    fn the_settings_round_trip_through_the_config() {
        let settings = Settings {
            enabled: true,
            interval: 15,
            feeds: vec![Feed {
                name: "tracker".to_owned(),
                url: "https://tracker.invalid/rss".to_owned(),
                enabled: true,
            }],
            rules: vec![Rule {
                name: "series".to_owned(),
                contains: "winter*".to_owned(),
                label: "series".to_owned(),
                enabled: true,
                ..Default::default()
            }],
        };

        let read = Settings::from_config(Some(&settings.to_json()));
        assert!(read.enabled);
        assert_eq!(read.interval, 15);
        assert_eq!(read.feeds.len(), 1);
        assert_eq!(read.rules[0].label, "series");
        assert!(read.worth_reading());
    }

    #[test]
    fn nothing_configured_is_nothing_to_do() {
        assert!(!Settings::default().worth_reading());
        let no_rules = Settings {
            enabled: true,
            feeds: vec![Feed {
                name: "f".to_owned(),
                url: "https://x.invalid".to_owned(),
                enabled: true,
            }],
            ..Default::default()
        };
        assert!(!no_rules.worth_reading());
    }
}
