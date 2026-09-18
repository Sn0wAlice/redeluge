// SPDX-License-Identifier: GPL-3.0-or-later
//! What this client tells the swarm it is.
//!
//! Two strings make up an identity on the wire, and they have to agree:
//!
//! * the **user agent**, sent as the HTTP header to trackers and as the `v`
//!   string in the extension handshake to peers, and
//! * the **peer id**, whose first bytes are a fingerprint like `-qB4650-`,
//!   sent to every tracker and every peer this client ever talks to.
//!
//! Faking one and not the other is worse than faking neither: `qBittorrent`
//! in the header with a libtorrent peer id is a combination no real client
//! produces, so it identifies this daemon more precisely than the truth would.
//! Everything here therefore sets both or neither.
//!
//! Four modes, and what each one is for:
//!
//! | mode | what the swarm sees |
//! |---|---|
//! | `show` | the truth: redeluge, and the libtorrent it is built on |
//! | `hide` | libtorrent's own anonymous mode: a generic agent, no version to peers |
//! | `rotate` | a credible client from [`CREDIBLE`], drawn again for each torrent added |
//! | `custom` | whatever you typed |
//!
//! **What none of them do is hide your address.** Every peer and tracker still
//! sees the IP packets arrive. This is about what the client says, not about
//! where it says it from, and the setting that changes the latter is the proxy.
//!
//! ## The limits of rotation
//!
//! Both strings are *session* settings in libtorrent. There is no API for
//! presenting one client to one peer and another to the next, so "a different
//! identity per peer" is not a thing that can be built on this library, and
//! this module does not pretend otherwise.
//!
//! What can be done is per torrent, and only half way: libtorrent generates a
//! peer id per torrent, from the fingerprint setting as it stands when the
//! torrent is added, so drawing a new fingerprint before each add gives each
//! torrent its own identity and keeps it for the torrent's life. The user
//! agent has no such split — it is one string for the whole daemon — so in
//! `rotate` it follows the most recent draw. A tracker that compares the two
//! across torrents can tell. That is the cost of the mode, and it is the
//! reason `show` and `custom` exist beside it.

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;

/// Tell the truth.
pub const SHOW: &str = "show";
/// libtorrent's `anonymous_mode`.
pub const HIDE: &str = "hide";
/// A new credible client for each torrent added.
pub const ROTATE: &str = "rotate";
/// Whatever was typed into the two boxes.
pub const CUSTOM: &str = "custom";

/// The modes, in the order the interface offers them.
pub const MODES: &[&str] = &[SHOW, HIDE, ROTATE, CUSTOM];

/// libtorrent truncates a fingerprint to twenty bytes and uses it as the whole
/// peer id when it is that long. Anything longer is silently cut, so it is cut
/// here instead, where it can be seen.
const MAX_PEER_ID: usize = 20;

/// A client this daemon can claim to be.
///
/// Every fingerprint here is the one that client really sends, written out
/// rather than derived: the conventional encoding cannot produce all of them —
/// Deluge's ends in a release-type letter — and a fingerprint that is close but
/// not exact is a fingerprint nobody else in the world has, which defeats the
/// entire purpose of blending in.
pub struct Client {
    /// What the interface calls it.
    pub name: &'static str,
    pub user_agent: &'static str,
    pub fingerprint: &'static str,
}

/// The clients `rotate` draws from.
///
/// Deliberately short and deliberately boring: four clients that between them
/// are most of any public swarm. A long list of rare clients would make this
/// daemon stand out in exactly the way the mode is meant to prevent.
pub const CREDIBLE: &[Client] = &[
    Client {
        name: "qBittorrent 4.6.5",
        user_agent: "qBittorrent/4.6.5",
        fingerprint: "-qB4650-",
    },
    Client {
        name: "Transmission 4.0.5",
        user_agent: "Transmission/4.0.5",
        fingerprint: "-TR4050-",
    },
    Client {
        name: "Deluge 2.1.1",
        user_agent: "Deluge/2.1.1 libtorrent/2.0.9.0",
        fingerprint: "-DE211s-",
    },
    Client {
        name: "libtorrent 2.0.10",
        user_agent: "libtorrent/2.0.10.0",
        fingerprint: "-LT20A0-",
    },
];

/// What the session is told to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// libtorrent's `anonymous_mode`, which is a mode of its own rather than
    /// an identity: it leaves the strings alone and suppresses them instead.
    pub anonymous: bool,
    pub user_agent: String,
    pub fingerprint: String,
}

/// The `identity` key of `core.conf`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    /// One of [`MODES`]. Anything else is read as `show`, because a daemon
    /// that cannot understand this key must not invent an identity.
    #[serde(default = "show")]
    pub mode: String,
    /// `custom` only.
    #[serde(default)]
    pub user_agent: String,
    /// `custom` only: the peer id prefix, conventionally `-XX1234-`.
    #[serde(default)]
    pub peer_id: String,
}

fn show() -> String {
    SHOW.to_owned()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            mode: show(),
            user_agent: String::new(),
            peer_id: String::new(),
        }
    }
}

impl Settings {
    pub fn from_config(value: Option<&Json>) -> Self {
        match value {
            Some(value) => {
                serde_json::from_value(super::without_nulls(value)).unwrap_or_else(|err| {
                    super::warn_malformed("identity", &err.to_string());
                    Self::default()
                })
            }
            None => Self::default(),
        }
    }

    pub fn default_json() -> Json {
        serde_json::to_value(Self::default()).expect("the defaults serialise")
    }

    /// Bounded, because these come from a client.
    ///
    /// An unknown mode becomes `show`: the one thing this must never do is
    /// invent an identity nobody asked for. Both strings lose anything that is
    /// not printable ASCII, because they go into an HTTP header and into a
    /// twenty-byte peer id, and a newline in either is a broken announce at
    /// best.
    pub fn sane(&self) -> Self {
        let mode = if MODES.contains(&self.mode.as_str()) {
            self.mode.clone()
        } else {
            show()
        };
        Self {
            mode,
            user_agent: printable(&self.user_agent, 200),
            peer_id: printable(&self.peer_id, MAX_PEER_ID),
        }
    }

    pub fn mode(&self) -> &str {
        &self.mode
    }

    /// Whether a new client is drawn for each torrent added.
    pub fn rotates(&self) -> bool {
        self.mode == ROTATE
    }

    /// What to tell libtorrent, given what this daemon honestly is.
    ///
    /// `rotate` draws here, so each call can answer differently; every other
    /// mode is a pure function of the settings.
    pub fn resolve(&self, honest: &Identity) -> Identity {
        match self.mode.as_str() {
            HIDE => Identity {
                anonymous: true,
                ..honest.clone()
            },
            ROTATE => {
                let client = draw();
                Identity {
                    anonymous: false,
                    user_agent: client.user_agent.to_owned(),
                    fingerprint: client.fingerprint.to_owned(),
                }
            }
            CUSTOM => Identity {
                anonymous: false,
                // An empty box is the honest value rather than an empty
                // header: a blank user agent is itself distinctive, and a
                // blank fingerprint would make every peer id pure noise.
                user_agent: if self.user_agent.is_empty() {
                    honest.user_agent.clone()
                } else {
                    self.user_agent.clone()
                },
                fingerprint: if self.peer_id.is_empty() {
                    honest.fingerprint.clone()
                } else {
                    self.peer_id.clone()
                },
            },
            // `show`, and anything this build does not recognise.
            _ => honest.clone(),
        }
    }
}

/// What this daemon actually is.
///
/// The fingerprint is libtorrent's own, for the libtorrent it is linked
/// against, so `show` restores exactly what would have been sent had nobody
/// ever touched this page. Computing it here rather than writing `-LT20B0-`
/// down means a libtorrent upgrade does not turn the honest mode into a lie.
pub fn honest() -> Identity {
    let version = redeluge_libtorrent::libtorrent_version();
    let mut parts = version
        .split('.')
        .map(|part| part.parse::<i32>().unwrap_or(0));
    let (major, minor, revision, tag) = (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    );

    Identity {
        anonymous: false,
        user_agent: format!(
            "redeluge/{} libtorrent/{version}",
            crate::core::REPORTED_VERSION
        ),
        fingerprint: redeluge_libtorrent::generate_fingerprint("LT", major, minor, revision, tag),
    }
}

/// One of [`CREDIBLE`], at random.
///
/// The clock rather than a generator: this picks which of four names to wear,
/// not a key, and pulling in a random number crate to choose between four
/// strings would be the largest dependency in this daemon's history for the
/// smallest reason.
pub fn draw() -> &'static Client {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.subsec_nanos() as usize)
        .unwrap_or(0);
    &CREDIBLE[nanos % CREDIBLE.len()]
}

/// Strips what cannot go in an HTTP header or a peer id, and cuts to length.
fn printable(value: &str, limit: usize) -> String {
    value
        .chars()
        .filter(|character| character.is_ascii_graphic() || *character == ' ')
        .take(limit)
        .collect::<String>()
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn honest_for_test() -> Identity {
        Identity {
            anonymous: false,
            user_agent: "redeluge/2.2.1 libtorrent/2.0.11.0".to_owned(),
            fingerprint: "-LT20B0-".to_owned(),
        }
    }

    #[test]
    fn the_default_is_the_truth() {
        // The one thing this must never do on its own is claim to be something
        // else. A daemon with no `identity` key has never been asked to.
        let settings = Settings::default();
        assert_eq!(settings.mode(), SHOW);
        assert_eq!(settings.resolve(&honest_for_test()), honest_for_test());
    }

    #[test]
    fn a_mode_this_build_does_not_know_is_the_truth_too() {
        // A newer build's mode, or a typo. Inventing an identity because a
        // string did not parse is how somebody ends up announcing as a client
        // they never chose.
        let settings = Settings {
            mode: "pretend-to-be-a-toaster".to_owned(),
            ..Settings::default()
        }
        .sane();
        assert_eq!(settings.mode(), SHOW);
        assert_eq!(settings.resolve(&honest_for_test()), honest_for_test());
    }

    #[test]
    fn hiding_changes_the_mode_and_not_the_strings() {
        // libtorrent's anonymous mode replaces the agent itself, at the point
        // it sends it. Rewriting the strings here as well would be two
        // mechanisms fighting over one answer.
        let settings = Settings {
            mode: HIDE.to_owned(),
            ..Settings::default()
        };
        let resolved = settings.resolve(&honest_for_test());
        assert!(resolved.anonymous);
        assert_eq!(resolved.user_agent, honest_for_test().user_agent);
        assert_eq!(resolved.fingerprint, honest_for_test().fingerprint);
    }

    #[test]
    fn every_credible_client_sets_both_halves() {
        // The whole premise: a user agent that says one thing and a peer id
        // that says another is more identifying than the truth.
        for client in CREDIBLE {
            assert!(!client.user_agent.is_empty(), "{}", client.name);
            assert!(
                client.fingerprint.len() <= MAX_PEER_ID,
                "{} would be truncated",
                client.name
            );
            assert!(
                client.fingerprint.starts_with('-') && client.fingerprint.ends_with('-'),
                "{} is not in the conventional form",
                client.name
            );
        }
    }

    #[test]
    fn rotation_answers_with_a_client_from_the_list() {
        let settings = Settings {
            mode: ROTATE.to_owned(),
            ..Settings::default()
        };
        let resolved = settings.resolve(&honest_for_test());
        assert!(!resolved.anonymous);
        assert!(
            CREDIBLE
                .iter()
                .any(|client| client.user_agent == resolved.user_agent
                    && client.fingerprint == resolved.fingerprint),
            "rotation invented an identity: {resolved:?}"
        );
    }

    #[test]
    fn a_custom_identity_is_taken_as_typed_and_an_empty_box_is_not() {
        let settings = Settings {
            mode: CUSTOM.to_owned(),
            user_agent: "Transmission/4.0.5".to_owned(),
            peer_id: "-TR4050-".to_owned(),
            ..Settings::default()
        };
        let resolved = settings.resolve(&honest_for_test());
        assert_eq!(resolved.user_agent, "Transmission/4.0.5");
        assert_eq!(resolved.fingerprint, "-TR4050-");

        // Half filled in: the other half stays honest rather than becoming
        // empty, because an empty agent is itself a distinguishing mark.
        let half = Settings {
            mode: CUSTOM.to_owned(),
            user_agent: "Transmission/4.0.5".to_owned(),
            peer_id: String::new(),
        };
        assert_eq!(
            half.resolve(&honest_for_test()).fingerprint,
            honest_for_test().fingerprint
        );
    }

    #[test]
    fn what_a_client_typed_cannot_break_an_announce() {
        // Both strings go into an HTTP request; one also goes into a twenty
        // byte peer id. A newline in either is a broken announce at best.
        let settings = Settings {
            mode: CUSTOM.to_owned(),
            user_agent: "  Evil/1.0\r\nX-Injected: yes  ".to_owned(),
            peer_id: "-qB4650-and-then-some-more-bytes".to_owned(),
        }
        .sane();

        assert_eq!(settings.user_agent, "Evil/1.0X-Injected: yes");
        assert_eq!(settings.peer_id.len(), MAX_PEER_ID);
        assert!(!settings.user_agent.contains('\n'));
        assert!(!settings.user_agent.contains('\r'));
    }
}
