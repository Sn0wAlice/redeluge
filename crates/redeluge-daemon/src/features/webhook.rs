// SPDX-License-Identifier: GPL-3.0-or-later
//! Telling somebody when a torrent finishes, arrives or breaks, and when a
//! tracker stops answering.
//!
//! This is what most people installed the Execute plugin for: a download
//! finishes and a message lands on a phone. Execute did it by running a shell
//! script as the daemon user, with the torrent's name as an argument, which is
//! a remote code execution primitive wearing a convenience hat. A POST is the
//! same result and runs nothing.
//!
//! Four shapes, because four services cover nearly everybody:
//!
//! - **Discord**, the incoming-webhook URL from a channel's settings.
//! - **ntfy**, the topic URL you would paste into the app.
//! - **Gotify**, the server URL and an application token.
//! - **a plain webhook**, one JSON object for whatever is at the other end.
//!
//! Every one of them is a POST with a JSON body, which keeps the transport in
//! one place and, less obviously, keeps torrent names working: ntfy's own
//! header form cannot carry anything that is not ASCII, and a good half of the
//! names that matter here are not.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value as Json};

/// Which service is at the other end, and so what the body has to look like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Discord,
    Ntfy,
    Gotify,
    /// A JSON object posted as it is, for anything else.
    Webhook,
}

impl Kind {
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "discord" => Some(Self::Discord),
            "ntfy" => Some(Self::Ntfy),
            "gotify" => Some(Self::Gotify),
            "webhook" | "json" | "generic" => Some(Self::Webhook),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Discord => "discord",
            Self::Ntfy => "ntfy",
            Self::Gotify => "gotify",
            Self::Webhook => "webhook",
        }
    }
}

/// What happened, which decides the wording and how loud the message is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    Finished,
    Error,
    Added,
    /// A tracker domain that is failing every announce it makes, and the same
    /// domain once it answers again. Two events rather than one carrying a
    /// flag, because every destination here renders a title and a body and
    /// none of them renders a state machine.
    TrackerDown,
    TrackerUp,
    /// Sent by the Send Test button, so a URL can be checked before it matters.
    Test,
}

impl Trigger {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Finished => "finished",
            Self::Error => "error",
            Self::Added => "added",
            Self::TrackerDown => "tracker_down",
            Self::TrackerUp => "tracker_up",
            Self::Test => "test",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::Finished => "Download finished",
            Self::Error => "Torrent error",
            Self::Added => "Torrent added",
            Self::TrackerDown => "Tracker down",
            Self::TrackerUp => "Tracker back up",
            Self::Test => "redeluge test message",
        }
    }

    /// Discord's embed colour, as the integer it wants.
    fn colour(self) -> i64 {
        match self {
            // Green, red, blue, grey.
            Self::Finished | Self::TrackerUp => 0x2E_CC_71,
            Self::Error | Self::TrackerDown => 0xE7_4C_3C,
            Self::Added => 0x34_98_DB,
            Self::Test => 0x95_A5_A6,
        }
    }

    /// ntfy's tags, which it renders as the message's icon.
    fn tags(self) -> &'static [&'static str] {
        match self {
            Self::Finished => &["white_check_mark"],
            Self::Error => &["rotating_light"],
            Self::Added => &["inbox_tray"],
            Self::TrackerDown => &["warning"],
            Self::TrackerUp => &["arrow_up"],
            Self::Test => &["bell"],
        }
    }

    /// ntfy priority, 1 to 5, and Gotify's 0 to 10.
    fn urgency(self) -> (i64, i64) {
        match self {
            Self::Error | Self::TrackerDown => (4, 8),
            _ => (3, 5),
        }
    }

    /// Whether this event is about a tracker rather than about one torrent.
    ///
    /// The two are written from the same `Notice`, which is what keeps the
    /// transport and the retries in one place, but almost none of the fields
    /// mean anything to both.
    pub fn is_tracker(self) -> bool {
        matches!(self, Self::TrackerDown | Self::TrackerUp)
    }
}

/// One destination.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Endpoint {
    #[serde(default = "yes")]
    pub enabled: bool,

    /// `discord`, `ntfy`, `gotify` or `webhook`.
    #[serde(default = "default_kind")]
    pub kind: String,

    pub url: String,

    /// Gotify's application token, or a bearer token for ntfy and a plain
    /// webhook. Empty when the URL carries whatever secret there is, which is
    /// how Discord works.
    #[serde(default)]
    pub token: String,
}

impl Endpoint {
    pub fn service(&self) -> Option<Kind> {
        Kind::parse(&self.kind)
    }

    /// Whether this is worth trying at all.
    ///
    /// A row somebody added and did not fill in is not an error to report on
    /// every torrent; it is a row somebody added and did not fill in.
    pub fn is_usable(&self) -> bool {
        self.enabled
            && self.service().is_some()
            && (self.url.starts_with("http://") || self.url.starts_with("https://"))
    }
}

/// The `webhook` key of `core.conf`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub enabled: bool,

    #[serde(default)]
    pub endpoints: Vec<Endpoint>,

    #[serde(default = "yes")]
    pub on_finished: bool,
    #[serde(default = "yes")]
    pub on_error: bool,
    /// Off by default: on a busy day it is the noisiest of the three, and it
    /// is the one you already know about, because you added the torrent.
    #[serde(default)]
    pub on_added: bool,

    /// Both edges, not one: somebody who wants to hear that a tracker stopped
    /// answering wants to hear that it is answering again, and a switch that
    /// tells you only half of that is a switch that leaves you checking.
    ///
    /// Off by default, like every trigger added after the first two: an
    /// installation that was already sending messages is not made louder by
    /// upgrading.
    #[serde(default)]
    pub on_tracker: bool,

    #[serde(default = "fifteen")]
    pub timeout: u64,
    #[serde(default = "three")]
    pub try_times: u32,

    /// Set to true by the Send Test button; the daemon sends one message and
    /// sets it back. A flag rather than an RPC method, because the method list
    /// is the frozen contract and a button is not worth breaking it for.
    #[serde(default)]
    pub test: bool,

    /// What came of the last test, written by the daemon for the interface to
    /// show. Nobody should have to read a log to find out that a URL is wrong.
    #[serde(default)]
    pub last_test: String,
}

fn yes() -> bool {
    true
}
fn default_kind() -> String {
    "discord".to_owned()
}
fn fifteen() -> u64 {
    15
}
fn three() -> u32 {
    3
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoints: Vec::new(),
            on_finished: yes(),
            on_error: yes(),
            on_added: false,
            on_tracker: false,
            timeout: fifteen(),
            try_times: three(),
            test: false,
            last_test: String::new(),
        }
    }
}

impl Settings {
    pub fn from_config(value: Option<&Json>) -> Self {
        match value {
            Some(value) => {
                serde_json::from_value(super::without_nulls(value)).unwrap_or_else(|err| {
                    super::warn_malformed("webhook", &err.to_string());
                    Self::default()
                })
            }
            None => Self::default(),
        }
    }

    pub fn default_json() -> Json {
        serde_json::to_value(Self::default()).expect("the defaults serialise")
    }

    pub fn sane(&self) -> Self {
        Self {
            timeout: self.timeout.clamp(1, 120),
            try_times: self.try_times.clamp(1, 10),
            ..self.clone()
        }
    }

    /// Whether this event is one anybody asked to hear about.
    ///
    /// A test is always wanted: somebody has just pressed the button.
    pub fn wants(&self, trigger: Trigger) -> bool {
        match trigger {
            Trigger::Finished => self.on_finished,
            Trigger::Error => self.on_error,
            Trigger::Added => self.on_added,
            Trigger::TrackerDown | Trigger::TrackerUp => self.on_tracker,
            Trigger::Test => true,
        }
    }

    pub fn usable(&self) -> impl Iterator<Item = &Endpoint> {
        self.endpoints.iter().filter(|end| end.is_usable())
    }
}

/// What happened, in the terms a message is written from.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Notice {
    pub torrent_id: String,
    pub name: String,
    pub size: i64,
    pub save_path: String,
    pub label: String,
    pub tracker: String,
    pub ratio: f64,
    /// How many torrents are on this tracker. Only the tracker events fill it
    /// in; it is the figure that says whether an outage matters.
    pub torrents: i64,
    /// The error, for the events that have one: what the torrent said, or what
    /// the tracker said when it refused.
    pub message: String,
}

impl Notice {
    /// The sample the test button sends.
    pub fn sample() -> Self {
        Self {
            torrent_id: "0".repeat(40),
            name: "A torrent that does not exist".to_owned(),
            size: 4 * 1024 * 1024 * 1024,
            save_path: "/downloads".to_owned(),
            label: "test".to_owned(),
            tracker: "example.com".to_owned(),
            ratio: 1.0,
            torrents: 0,
            message: "If you are reading this, the endpoint works.".to_owned(),
        }
    }

    /// The lines under the title, which every service renders as plain text.
    fn body(&self, trigger: Trigger) -> String {
        if trigger.is_tracker() {
            return self.tracker_body();
        }

        let mut lines = vec![self.name.clone()];
        if self.size > 0 {
            lines.push(format!("Size: {}", human_size(self.size)));
        }
        if !self.label.is_empty() {
            lines.push(format!("Label: {}", self.label));
        }
        if trigger == Trigger::Finished {
            lines.push(format!("Ratio: {:.2}", self.ratio));
        }
        if !self.save_path.is_empty() {
            lines.push(format!("Saved to: {}", self.save_path));
        }
        if !self.message.is_empty() {
            lines.push(self.message.clone());
        }
        lines.join("\n")
    }

    /// The same, for the events whose subject is a tracker.
    ///
    /// Which way it went is in the title, so these lines are what the title
    /// does not say: which tracker, how much of the library is behind it, and
    /// what it said when it refused.
    fn tracker_body(&self) -> String {
        let mut lines = vec![self.tracker.clone()];
        if self.torrents > 0 {
            lines.push(format!("Torrents: {}", self.torrents));
        }
        if !self.message.is_empty() {
            lines.push(self.message.clone());
        }
        lines.join("\n")
    }
}

/// Bytes as a person reads them.
pub fn human_size(bytes: i64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// One request, ready to send. Built without touching the network, so the hard
/// part is the part that is testable.
#[derive(Debug, Clone, PartialEq)]
pub struct Delivery {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Json,
}

/// What to POST for this destination and this event, or nothing if the
/// destination cannot be made sense of.
pub fn delivery(endpoint: &Endpoint, trigger: Trigger, notice: &Notice) -> Option<Delivery> {
    let kind = endpoint.service()?;
    let title = trigger.title();
    let body = notice.body(trigger);
    let (ntfy_priority, gotify_priority) = trigger.urgency();
    let mut headers = Vec::new();

    let (url, payload) = match kind {
        Kind::Discord => (
            endpoint.url.clone(),
            json!({
                "username": "redeluge",
                "embeds": [{
                    "title": title,
                    "description": body,
                    "color": trigger.colour(),
                }],
            }),
        ),

        Kind::Ntfy => {
            // The topic travels in the body rather than in the path, because
            // the same request then carries a title and a UTF-8 torrent name,
            // which ntfy's header form cannot.
            let (base, topic) = ntfy_target(&endpoint.url)?;
            if !endpoint.token.is_empty() {
                headers.push((
                    "Authorization".to_owned(),
                    format!("Bearer {}", endpoint.token),
                ));
            }
            (
                base,
                json!({
                    "topic": topic,
                    "title": title,
                    "message": body,
                    "priority": ntfy_priority,
                    "tags": trigger.tags(),
                }),
            )
        }

        Kind::Gotify => {
            if !endpoint.token.is_empty() {
                headers.push(("X-Gotify-Key".to_owned(), endpoint.token.clone()));
            }
            (
                gotify_url(&endpoint.url),
                json!({
                    "title": title,
                    "message": body,
                    "priority": gotify_priority,
                }),
            )
        }

        Kind::Webhook => {
            if !endpoint.token.is_empty() {
                headers.push((
                    "Authorization".to_owned(),
                    format!("Bearer {}", endpoint.token),
                ));
            }
            let mut payload = json!({
                "source": "redeluge",
                "event": trigger.as_str(),
                "title": title,
                "message": body,
            });
            // One subject per event, under the key that names it. A tracker
            // event carrying an empty `torrent` object would have whatever is
            // at the other end filing an outage under a torrent with no name.
            let (subject, detail) = if trigger.is_tracker() {
                (
                    "tracker",
                    json!({
                        "host": notice.tracker,
                        "torrents": notice.torrents,
                        "down": trigger == Trigger::TrackerDown,
                        // What it said when it refused, which is empty once it
                        // is answering again.
                        "error": notice.message.clone(),
                    }),
                )
            } else {
                (
                    "torrent",
                    json!({
                        "id": notice.torrent_id,
                        "name": notice.name,
                        "size": notice.size,
                        "save_path": notice.save_path,
                        "label": notice.label,
                        "tracker": notice.tracker,
                        "ratio": notice.ratio,
                        // Only a real error goes in the error field: a test's
                        // friendly sentence there would read as a failure to
                        // whatever is parsing this.
                        "error": if trigger == Trigger::Error {
                            notice.message.clone()
                        } else {
                            String::new()
                        },
                    }),
                )
            };
            payload[subject] = detail;
            (endpoint.url.clone(), payload)
        }
    };

    headers.push(("Content-Type".to_owned(), "application/json".to_owned()));
    Some(Delivery {
        url,
        headers,
        body: payload,
    })
}

/// Splits an ntfy topic URL into what to post to and what topic to name.
///
/// The last path segment is the topic and everything before it is the server,
/// which handles both `https://ntfy.sh/mytopic` and an instance living under a
/// path of its own. A URL with no path has no topic in it, and there is nothing
/// sensible to guess.
pub fn ntfy_target(url: &str) -> Option<(String, String)> {
    let trimmed = url.trim_end_matches('/');
    let scheme_end = trimmed.find("://")? + 3;
    let (scheme, rest) = trimmed.split_at(scheme_end);
    let (host, path) = match rest.split_once('/') {
        Some((host, path)) => (host, path),
        None => return None,
    };

    let mut segments: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    let topic = segments.pop()?;
    let mut base = format!("{scheme}{host}");
    for segment in segments {
        base.push('/');
        base.push_str(segment);
    }
    Some((base, topic.to_owned()))
}

/// Where a Gotify message is posted, given whatever the operator pasted.
///
/// Gotify's own interface shows the server root, its documentation shows
/// `/message`, and a URL with the token already in the query is a third thing
/// people have in their notes. All three arrive here.
pub fn gotify_url(url: &str) -> String {
    let (path, query) = match url.split_once('?') {
        Some((path, query)) => (path, Some(query)),
        None => (url, None),
    };
    let path = path.trim_end_matches('/');
    let path = if path.ends_with("/message") {
        path.to_owned()
    } else {
        format!("{path}/message")
    };
    match query {
        Some(query) => format!("{path}?{query}"),
        None => path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn notice() -> Notice {
        Notice {
            torrent_id: "abc".to_owned(),
            name: "Un été à Québec".to_owned(),
            size: 3 * 1024 * 1024 * 1024,
            save_path: "/downloads".to_owned(),
            label: "films".to_owned(),
            tracker: "example.com".to_owned(),
            ratio: 1.25,
            torrents: 0,
            message: String::new(),
        }
    }

    fn endpoint(kind: &str, url: &str) -> Endpoint {
        Endpoint {
            enabled: true,
            kind: kind.to_owned(),
            url: url.to_owned(),
            token: String::new(),
        }
    }

    #[test]
    fn it_is_off_until_somebody_turns_it_on() {
        let settings = Settings::default();
        assert!(!settings.enabled);
        assert!(settings.endpoints.is_empty());
        assert!(settings.on_finished && settings.on_error);
        assert!(!settings.on_added, "the noisiest of the three");
        assert!(!settings.on_tracker, "added after, so it stays quiet");
    }

    #[test]
    fn discord_gets_an_embed_with_the_name_in_it() {
        let delivery = delivery(
            &endpoint("discord", "https://discord.com/api/webhooks/1/abc"),
            Trigger::Finished,
            &notice(),
        )
        .unwrap();

        assert_eq!(delivery.url, "https://discord.com/api/webhooks/1/abc");
        let embed = &delivery.body["embeds"][0];
        assert_eq!(embed["title"], "Download finished");
        let text = embed["description"].as_str().unwrap();
        assert!(text.contains("Un été à Québec"));
        assert!(text.contains("3.0 GiB"));
        assert!(text.contains("films"));
    }

    #[test]
    fn ntfy_posts_the_topic_in_the_body() {
        // The whole reason for the JSON form: a name with an accent in it
        // cannot go in an ntfy header, and plenty of names have one.
        let delivery = delivery(
            &endpoint("ntfy", "https://ntfy.sh/my-downloads"),
            Trigger::Finished,
            &notice(),
        )
        .unwrap();

        assert_eq!(delivery.url, "https://ntfy.sh");
        assert_eq!(delivery.body["topic"], "my-downloads");
        assert!(delivery.body["message"]
            .as_str()
            .unwrap()
            .contains("Un été à Québec"));
    }

    #[test]
    fn an_ntfy_instance_under_a_path_keeps_that_path() {
        assert_eq!(
            ntfy_target("https://example.com/ntfy/downloads"),
            Some((
                "https://example.com/ntfy".to_owned(),
                "downloads".to_owned()
            ))
        );
        assert_eq!(
            ntfy_target("https://ntfy.sh/downloads/"),
            Some(("https://ntfy.sh".to_owned(), "downloads".to_owned()))
        );
        // No topic to guess at.
        assert_eq!(ntfy_target("https://ntfy.sh"), None);
    }

    #[test]
    fn a_bearer_token_is_sent_when_there_is_one() {
        let mut end = endpoint("ntfy", "https://ntfy.sh/private");
        end.token = "tk_secret".to_owned();
        let delivery = delivery(&end, Trigger::Finished, &notice()).unwrap();
        assert!(delivery
            .headers
            .contains(&("Authorization".to_owned(), "Bearer tk_secret".to_owned())));
    }

    #[test]
    fn gotify_takes_the_server_or_the_message_path() {
        assert_eq!(
            gotify_url("https://gotify.example.com"),
            "https://gotify.example.com/message"
        );
        assert_eq!(
            gotify_url("https://gotify.example.com/"),
            "https://gotify.example.com/message"
        );
        assert_eq!(
            gotify_url("https://gotify.example.com/message"),
            "https://gotify.example.com/message"
        );
        // A token already in the query survives, which is how most people
        // have it written down.
        assert_eq!(
            gotify_url("https://gotify.example.com?token=AxB"),
            "https://gotify.example.com/message?token=AxB"
        );
    }

    #[test]
    fn gotify_sends_its_token_as_the_header_it_wants() {
        let mut end = endpoint("gotify", "https://gotify.example.com");
        end.token = "AxB".to_owned();
        let delivery = delivery(&end, Trigger::Error, &notice()).unwrap();
        assert!(delivery
            .headers
            .contains(&("X-Gotify-Key".to_owned(), "AxB".to_owned())));
        assert_eq!(delivery.body["priority"], 8, "an error is louder");
    }

    #[test]
    fn a_plain_webhook_gets_the_facts_rather_than_a_sentence() {
        // The point of this one: whatever is at the other end reads fields,
        // not English.
        let delivery = delivery(
            &endpoint("webhook", "https://example.com/hook"),
            Trigger::Error,
            &Notice {
                message: "no space left on device".to_owned(),
                ..notice()
            },
        )
        .unwrap();

        assert_eq!(delivery.body["event"], "error");
        assert_eq!(delivery.body["torrent"]["label"], "films");
        assert_eq!(delivery.body["torrent"]["size"], 3 * 1024 * 1024 * 1024_i64);
        assert_eq!(delivery.body["torrent"]["error"], "no space left on device");
    }

    #[test]
    fn a_row_nobody_filled_in_is_not_tried() {
        assert!(!endpoint("discord", "").is_usable());
        assert!(!endpoint("discord", "not a url").is_usable());
        assert!(!endpoint("pigeon", "https://example.com").is_usable());
        let mut off = endpoint("discord", "https://example.com");
        off.enabled = false;
        assert!(!off.is_usable());
        assert!(endpoint("discord", "https://example.com").is_usable());
    }

    #[test]
    fn only_the_events_that_were_asked_for() {
        let settings = Settings {
            on_finished: true,
            on_error: false,
            on_added: false,
            ..Settings::default()
        };
        assert!(settings.wants(Trigger::Finished));
        assert!(!settings.wants(Trigger::Error));
        assert!(!settings.wants(Trigger::Added));
        assert!(!settings.wants(Trigger::TrackerDown));
        assert!(settings.wants(Trigger::Test), "somebody just pressed it");

        // One switch for both edges: being told a tracker went away and not
        // being told it came back is worse than not being told at all.
        let watching = Settings {
            on_tracker: true,
            ..Settings::default()
        };
        assert!(watching.wants(Trigger::TrackerDown));
        assert!(watching.wants(Trigger::TrackerUp));
    }

    #[test]
    fn sizes_read_the_way_people_write_them() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1024), "1.0 KiB");
        assert_eq!(human_size(3 * 1024 * 1024 * 1024), "3.0 GiB");
    }

    #[test]
    fn the_stored_shape_round_trips() {
        let settings = Settings::from_config(Some(&serde_json::json!({
            "enabled": true,
            "endpoints": [
                {"kind": "gotify", "url": "https://gotify.example.com", "token": "AxB"}
            ],
            "on_added": true
        })));
        assert!(settings.enabled);
        assert_eq!(settings.endpoints.len(), 1);
        assert_eq!(settings.endpoints[0].service(), Some(Kind::Gotify));
        assert!(settings.endpoints[0].enabled, "a row defaults to on");
        assert!(settings.on_added);
        assert_eq!(settings.try_times, three());
    }

    fn outage() -> Notice {
        Notice {
            name: "tracker.example.com".to_owned(),
            tracker: "tracker.example.com".to_owned(),
            torrents: 12,
            message: "connection refused".to_owned(),
            ..Notice::default()
        }
    }

    #[test]
    fn a_tracker_outage_reads_as_one_rather_than_as_a_torrent() {
        let delivery = delivery(
            &endpoint("discord", "https://discord.com/api/webhooks/1/abc"),
            Trigger::TrackerDown,
            &outage(),
        )
        .unwrap();

        let embed = &delivery.body["embeds"][0];
        assert_eq!(embed["title"], "Tracker down");
        let text = embed["description"].as_str().unwrap();
        assert!(text.contains("tracker.example.com"));
        assert!(text.contains("Torrents: 12"), "how much is behind it");
        assert!(text.contains("connection refused"));
        assert_eq!(embed["color"], 0xE7_4C_3C, "red, like an error");
    }

    #[test]
    fn a_tracker_that_came_back_says_so_and_carries_no_complaint() {
        let delivery = delivery(
            &endpoint("ntfy", "https://ntfy.sh/my-downloads"),
            Trigger::TrackerUp,
            &Notice {
                message: String::new(),
                ..outage()
            },
        )
        .unwrap();

        assert_eq!(delivery.body["title"], "Tracker back up");
        assert_eq!(delivery.body["priority"], 3, "good news is not urgent");
        let text = delivery.body["message"].as_str().unwrap();
        assert!(text.contains("tracker.example.com"));
        assert!(!text.contains("refused"));
    }

    #[test]
    fn a_plain_webhook_files_an_outage_under_the_tracker() {
        // Whatever is at the other end keys off `event` and then reads one
        // object; an empty torrent on a tracker event would be a torrent with
        // no name appearing in somebody's dashboard.
        let delivery = delivery(
            &endpoint("webhook", "https://example.com/hook"),
            Trigger::TrackerDown,
            &outage(),
        )
        .unwrap();

        assert_eq!(delivery.body["event"], "tracker_down");
        assert_eq!(delivery.body["tracker"]["host"], "tracker.example.com");
        assert_eq!(delivery.body["tracker"]["torrents"], 12);
        assert_eq!(delivery.body["tracker"]["down"], true);
        assert_eq!(delivery.body["tracker"]["error"], "connection refused");
        assert!(delivery.body.get("torrent").is_none());
    }

    #[test]
    fn a_malformed_value_is_the_defaults_rather_than_an_error() {
        let settings = Settings::from_config(Some(&serde_json::json!({"endpoints": 4})));
        assert!(settings.endpoints.is_empty());
    }
}
