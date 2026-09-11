// SPDX-License-Identifier: GPL-3.0-or-later
//! The RPC surface the Rust daemon must reproduce, as data.
//!
//! `tools/extract_contract.py` reads the Python tree and writes `contract/*.json`.
//! This crate embeds those files at compile time and hands them back typed, so
//! the daemon can assert at build time that it implements every method rather
//! than discovering a gap from a client error.
//!
//! Nothing here is hand-written. To change the contract, change the Python
//! source and re-run the extractor.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

const RPC_API_JSON: &str = include_str!("../../../contract/rpc-api.json");
const EVENTS_JSON: &str = include_str!("../../../contract/events.json");
const CONFIG_JSON: &str = include_str!("../../../contract/config-keys.json");
const ALERTS_JSON: &str = include_str!("../../../contract/alerts.json");

/// What a caller has to prove before a method will run.
///
/// The numbers are the Python daemon's, and they are part of the wire contract:
/// a client that asks for its own auth level gets one of these back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(from = "u8")]
pub enum AuthLevel {
    None,
    ReadOnly,
    Normal,
    Admin,
    /// A level the Python source used that this enum does not name.
    Other(u8),
}

impl From<u8> for AuthLevel {
    fn from(raw: u8) -> Self {
        match raw {
            0 => Self::None,
            1 => Self::ReadOnly,
            5 => Self::Normal,
            10 => Self::Admin,
            other => Self::Other(other),
        }
    }
}

impl AuthLevel {
    pub fn as_u8(self) -> u8 {
        match self {
            Self::None => 0,
            Self::ReadOnly => 1,
            Self::Normal => 5,
            Self::Admin => 10,
            Self::Other(raw) => raw,
        }
    }
}

/// Which listener answers a method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Transport {
    /// The DelugeRPC listener, rencode over TLS.
    Daemon,
    /// The JSON-RPC endpoint the browser talks to.
    Web,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Param {
    pub name: String,
    pub annotation: Option<String>,
    pub required: bool,
    pub default: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Method {
    /// Fully qualified, as a client types it: `core.add_torrent_url`.
    pub name: String,
    pub namespace: String,
    pub method: String,
    pub transport: Transport,
    pub auth_level: AuthLevel,
    pub returns: Option<String>,
    pub params: Vec<Param>,
    pub summary: Option<String>,
    /// Where it lives in the Python tree, for when the shape is unclear.
    pub source: String,
    /// Set only on methods the Rust daemon deliberately drops.
    #[serde(default)]
    pub removed_because: Option<String>,
}

impl Method {
    /// Parameters a caller must supply.
    pub fn required_params(&self) -> impl Iterator<Item = &Param> {
        self.params.iter().filter(|p| p.required)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Event {
    pub name: String,
    pub args: Vec<String>,
    pub summary: Option<String>,
    pub source: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConfigKey {
    pub key: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub default: String,
    pub source: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AlertEntry {
    pub alert: String,
    pub handler_key: String,
}

#[derive(Debug, Deserialize)]
struct RpcDocument {
    methods: Vec<Method>,
    removed: Vec<Method>,
}

#[derive(Debug, Deserialize)]
struct EventDocument {
    events: Vec<Event>,
}

#[derive(Debug, Deserialize)]
struct ConfigDocument {
    core: Vec<ConfigKey>,
    web: Vec<ConfigKey>,
}

#[derive(Debug, Deserialize)]
struct AlertDocument {
    alerts: Vec<AlertEntry>,
}

/// The whole contract, parsed once.
pub struct Contract {
    methods: Vec<Method>,
    removed: Vec<Method>,
    by_name: BTreeMap<String, usize>,
    events: Vec<Event>,
    core_config: Vec<ConfigKey>,
    web_config: Vec<ConfigKey>,
    alerts: Vec<AlertEntry>,
}

impl Contract {
    /// The contract as extracted. Parsed on first use and then shared.
    pub fn get() -> &'static Contract {
        static CONTRACT: OnceLock<Contract> = OnceLock::new();
        CONTRACT.get_or_init(|| {
            let rpc: RpcDocument =
                serde_json::from_str(RPC_API_JSON).expect("contract/rpc-api.json is malformed");
            let events: EventDocument =
                serde_json::from_str(EVENTS_JSON).expect("contract/events.json is malformed");
            let config: ConfigDocument =
                serde_json::from_str(CONFIG_JSON).expect("contract/config-keys.json is malformed");
            let alerts: AlertDocument =
                serde_json::from_str(ALERTS_JSON).expect("contract/alerts.json is malformed");

            let by_name = rpc
                .methods
                .iter()
                .enumerate()
                .map(|(index, method)| (method.name.clone(), index))
                .collect();

            Contract {
                methods: rpc.methods,
                removed: rpc.removed,
                by_name,
                events: events.events,
                core_config: config.core,
                web_config: config.web,
                alerts: alerts.alerts,
            }
        })
    }

    /// Every method the Rust daemon has to answer.
    pub fn methods(&self) -> &[Method] {
        &self.methods
    }

    /// Methods that existed in the Python daemon and are deliberately gone.
    pub fn removed(&self) -> &[Method] {
        &self.removed
    }

    pub fn method(&self, name: &str) -> Option<&Method> {
        self.by_name.get(name).map(|index| &self.methods[*index])
    }

    /// Methods answered by one listener.
    pub fn methods_for(&self, transport: Transport) -> impl Iterator<Item = &Method> {
        self.methods
            .iter()
            .filter(move |m| m.transport == transport)
    }

    pub fn events(&self) -> &[Event] {
        &self.events
    }

    pub fn core_config(&self) -> &[ConfigKey] {
        &self.core_config
    }

    pub fn web_config(&self) -> &[ConfigKey] {
        &self.web_config
    }

    pub fn alerts(&self) -> &[AlertEntry] {
        &self.alerts
    }

    /// Names in the contract that `implemented` does not cover.
    ///
    /// This is what the daemon's own test will call as methods land, so the gap
    /// is always a number rather than a guess.
    pub fn missing_from<'a, I>(&'a self, implemented: I) -> Vec<&'a str>
    where
        I: IntoIterator<Item = &'a str>,
    {
        let covered: std::collections::BTreeSet<&str> = implemented.into_iter().collect();
        self.methods
            .iter()
            .map(|m| m.name.as_str())
            .filter(|name| !covered.contains(name))
            .collect()
    }
}
