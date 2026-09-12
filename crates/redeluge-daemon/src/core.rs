// SPDX-License-Identifier: GPL-3.0-or-later
//! The methods the daemon exposes.
//!
//! Authorisation levels are not written here: they come from
//! `contract/rpc-api.json`, extracted from the Python daemon. A method whose
//! level drifted would otherwise be invisible until someone with a read-only
//! account deleted a torrent.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use redeluge_contract::{Contract, Transport};
use redeluge_libtorrent::{flags, AddTorrent, FlagChange};
use redeluge_rencode::Value;
use tokio::sync::Mutex;

use crate::auth::{AuthLevel, AuthManager};
use crate::config::Config;
use crate::events::Event;
use crate::manager::{restore_request, Manager};
use crate::prefs;
use crate::rpc::{CallContext, Rpc, RpcError};
use crate::state::TorrentState;
use crate::torrent::{Torrent, TorrentOptions};

/// The daemon's version, as reported to clients.
///
/// Deluge's own version rather than this crate's: clients compare it against
/// what they know how to speak, and telling them redeluge's own number makes
/// every one of them refuse to connect. Not written out here, so a release
/// does not have to remember to edit a comment.
pub const REPORTED_VERSION: &str = "2.2.1";

/// The one plugin this daemon answers for, by the name Deluge gave it.
pub const EMULATED_PLUGIN: &str = "Label";

/// The methods that exist because of it.
///
/// Deluge's plugins register their own RPC methods, so a daemon with the Label
/// plugin enabled advertises these on top of the core's own. They are not in
/// `contract/rpc-api.json` because that was extracted from the core, and they
/// are listed here rather than derived so that adding one is a deliberate act
/// with a test to match.
///
/// Why they exist at all: every program built on Deluge's API asks
/// `core.get_enabled_plugins` whether Label is there and then calls these.
/// Radarr and Sonarr will not let you set a download category without it, and
/// say "Label plugin not activated" instead.
pub const PLUGIN_METHODS: &[&str] = &[
    "core.disable_plugin",
    "core.enable_plugin",
    "core.get_available_plugins",
    "core.get_enabled_plugins",
    "label.add",
    "label.get_config",
    "label.get_labels",
    "label.get_options",
    "label.remove",
    "label.set_config",
    "label.set_options",
    "label.set_torrent",
];

/// Everything a call can reach.
pub struct Core {
    pub manager: Manager,
    pub config: Mutex<Config>,
    pub auth: Mutex<AuthManager>,
    pub config_dir: PathBuf,
    /// Set when `daemon.shutdown` is called, so the main loop can stop.
    pub shutdown: tokio::sync::Notify,
}

impl Core {
    pub fn new(
        manager: Manager,
        config: Config,
        auth: AuthManager,
        config_dir: PathBuf,
    ) -> Arc<Self> {
        Arc::new(Self {
            manager,
            config: Mutex::new(config),
            auth: Mutex::new(auth),
            config_dir,
            shutdown: tokio::sync::Notify::new(),
        })
    }

    /// Restores the torrents a previous run left behind.
    pub async fn restore(&self) -> usize {
        let config_dir = self.config_dir.clone();
        let restored = self
            .manager
            .with(move |state| {
                let saved = crate::manager::SessionState::load_state(&config_dir);
                let state_dir = state.state_dir();
                let mut count = 0;

                for options in saved {
                    let Some(mut request) = restore_request(&options, &state_dir) else {
                        tracing::warn!(id = %options.torrent_id,
                            "no torrent file or magnet, skipping");
                        continue;
                    };
                    // Resume data is what makes this a restore rather than a
                    // fresh add: without it every torrent rechecks from disk.
                    if let Some(blob) = state.load_resume_data(&options.torrent_id) {
                        request.resume_data = blob;
                    }

                    match state.session.add_torrent(&request) {
                        Ok(id) => {
                            state.torrents.insert(id.clone(), Torrent::new(id, options));
                            count += 1;
                        }
                        Err(err) => tracing::error!(id = %options.torrent_id,
                            error = %err, "could not restore a torrent"),
                    }
                }
                count
            })
            .await
            .unwrap_or(0);

        if restored > 0 {
            tracing::info!(restored, "restored torrents from the previous run");
        }
        restored
    }

    /// Writes the configuration, so a fresh install has a file to edit.
    ///
    /// Loading fills in every missing key, which on a first run is all of them.
    /// Without this the file never appears and an operator has nothing to
    /// change, which is how the first run of this daemon shipped with no
    /// core.conf at all.
    pub async fn save_config(&self) {
        let mut config = self.config.lock().await;
        if let Err(err) = config.save() {
            tracing::error!(error = %err, "could not write the configuration");
        }
    }

    /// Loads the GeoIP database the configuration names, if there is one.
    ///
    /// Absent by default and absent in the container: the database cannot be
    /// shipped, because its licence does not allow it. Without one the peer
    /// country is simply empty, which is what it was before.
    pub async fn load_country_database(&self) {
        let path = {
            let config = self.config.lock().await;
            config
                .string("geoip_db_location")
                .unwrap_or_default()
                .to_owned()
        };
        if path.is_empty() {
            return;
        }
        let lookup = crate::geoip::CountryLookup::open(std::path::Path::new(&path));
        let _ = self
            .manager
            .with(move |state| state.set_country_lookup(lookup))
            .await;
    }

    /// Pushes the whole configuration into libtorrent.
    pub async fn apply_config(&self) {
        let settings = {
            let config = self.config.lock().await;
            prefs::to_settings(&config)
        };
        let count = settings.len();

        let outcome = self
            .manager
            .with(move |state| state.session.apply_settings(&settings))
            .await;

        match outcome {
            Ok(Ok(())) => tracing::info!(count, "applied session settings"),
            Ok(Err(err)) => tracing::error!(error = %err, "could not apply session settings"),
            Err(err) => tracing::error!(error = %err, "the torrent manager is not answering"),
        }
    }

    /// Pauses or resumes every torrent, and remembers which it is.
    ///
    /// The session flag is what makes a paused session report every torrent as
    /// paused rather than as whatever it was; the schedule uses this for its
    /// stopped hours, and so does `core.pause_session`.
    pub async fn set_session_paused(&self, paused: bool) -> Result<(), String> {
        self.manager
            .with(move |state| {
                state.session_paused = paused;

                if paused {
                    // Only what is running, and remember which those were. A
                    // torrent that was already paused is not this pause's to
                    // undo later.
                    state.paused_by_session.clear();
                    for status in state.session.all_torrent_status() {
                        if status.is_paused {
                            continue;
                        }
                        let change = FlagChange::new().set_to(flags::PAUSED, true);
                        if state.session.set_flags(&status.info_hash, change).is_ok() {
                            state.paused_by_session.insert(status.info_hash);
                        }
                    }
                    return;
                }

                // Resuming starts what this pause stopped, and nothing else.
                // Resuming everything undid every deliberate pause in the
                // session, and the scheduler performs a resume at startup, so
                // no pause of any kind survived a restart.
                for id in std::mem::take(&mut state.paused_by_session) {
                    let change = FlagChange::new().set_to(flags::PAUSED, false);
                    let _ = state.session.set_flags(&id, change);
                }
            })
            .await
            .map_err(|err| err.to_string())?;

        self.manager.announce(if paused {
            Event::SessionPaused
        } else {
            Event::SessionResumed
        });
        Ok(())
    }

    /// How long the idle rule waits before pausing, for the countdown.
    ///
    /// Read from the same settings the rule acts on, and bounded the same way,
    /// so what the interface counts down to is when it will actually happen.
    async fn idle_grace(&self) -> u64 {
        let config = self.config.lock().await;
        crate::features::idlepause::Settings::from_config(config.get("idle_pause"))
            .sane()
            .grace
    }

    // ------------------------------------------------------------- labels

    /// The register of labels, out of `core.conf`.
    async fn labels(&self) -> crate::features::label::Settings {
        let config = self.config.lock().await;
        crate::features::label::Settings::from_config(config.get("label"))
    }

    /// Writes the register back.
    async fn store_labels(
        &self,
        labels: &crate::features::label::Settings,
    ) -> Result<(), RpcError> {
        let mut config = self.config.lock().await;
        config
            .set("label", labels.to_json())
            .map_err(|err| RpcError::new("InvalidConfigError", err.to_string()))?;
        let _ = config.save();
        Ok(())
    }

    /// Puts a torrent in a label, creating the label if it is new.
    ///
    /// Creating it is a deliberate divergence from the plugin, which raised on
    /// an unknown label. Every program that drives this adds the label and
    /// then sets it, and the add is the call most likely to have been skipped,
    /// retried out of order, or lost against a daemon that restarted. Refusing
    /// here means a torrent silently lands with no category; creating it means
    /// the label exists, which is what was asked for either way.
    pub async fn assign_label(&self, torrent_id: &str, label: &str) -> Result<(), RpcError> {
        let label = normalise_label(label);

        let options = if label.is_empty() {
            None
        } else {
            let mut labels = self.labels().await;
            if labels.add(&label) {
                self.store_labels(&labels).await?;
                tracing::info!(%label, "label created because a torrent was put in it");
            }
            labels.options(&label).cloned()
        };

        let id = torrent_id.to_owned();
        let wanted = label.clone();
        let known = self
            .manager
            .with(move |state| {
                let Some(torrent) = state.torrents.get_mut(&id) else {
                    return false;
                };
                torrent.options.label = wanted;
                state.mark_dirty();
                true
            })
            .await
            .map_err(|err| RpcError::invalid_argument(err.to_string()))?;

        if !known {
            return Err(RpcError::new(
                "InvalidTorrentError",
                format!("no such torrent: {torrent_id}"),
            ));
        }

        if let Some(options) = options {
            let changes = options.to_torrent_options();
            if !changes.is_empty() {
                self.apply_torrent_options(
                    vec![torrent_id.to_owned()],
                    Value::Dict(
                        changes
                            .into_iter()
                            .map(|(key, value)| (Value::Str(key), json_to_value(&value)))
                            .collect(),
                    ),
                )
                .await?;
            }
        }
        Ok(())
    }

    /// Takes a label off every torrent that carries it.
    async fn clear_label(&self, label: &str) -> usize {
        let wanted = label.to_owned();
        self.manager
            .with(move |state| {
                let mut cleared = 0;
                for torrent in state.torrents.values_mut() {
                    if torrent.options.label == wanted {
                        torrent.options.label.clear();
                        cleared += 1;
                    }
                }
                if cleared > 0 {
                    state.mark_dirty();
                }
                cleared
            })
            .await
            .unwrap_or(0)
    }

    /// Imposes a label's options on everything already in it.
    async fn apply_label_options(&self, label: &str, options: &crate::features::label::Options) {
        let changes = options.to_torrent_options();
        if changes.is_empty() {
            return;
        }

        let wanted = label.to_owned();
        let ids = self
            .manager
            .with(move |state| {
                state
                    .torrents
                    .values()
                    .filter(|torrent| torrent.options.label == wanted)
                    .map(|torrent| torrent.id.clone())
                    .collect::<Vec<_>>()
            })
            .await
            .unwrap_or_default();

        if ids.is_empty() {
            return;
        }
        let dict = Value::Dict(
            changes
                .into_iter()
                .map(|(key, value)| (Value::Str(key), json_to_value(&value)))
                .collect(),
        );
        if let Err(err) = self.apply_torrent_options(ids, dict).await {
            tracing::warn!(%label, error = %err.message, "could not apply a label's options");
        }
    }

    /// The options a new torrent starts from, out of the configuration.
    ///
    /// Deluge builds this dictionary in `core.add_torrent_file` and every
    /// client relies on it: the preferences window's "Add Torrent Options",
    /// the per-torrent bandwidth limits and the seeding rules are all defaults
    /// for the next torrent rather than settings of their own. This daemon
    /// started every torrent from a constant instead, so seventeen settings
    /// were stored and never read.
    ///
    /// Public because a watched directory adds torrents without an RPC call
    /// ever arriving, and a torrent from a watched directory gets the same
    /// defaults as one added by hand.
    pub async fn torrent_defaults(&self) -> TorrentOptions {
        let config = self.config.lock().await;
        let mut options = TorrentOptions::default();

        if let Some(path) = config.string("download_location") {
            if !path.is_empty() {
                options.save_path = Some(path.to_owned());
            }
        }
        options.paused = config.boolean("add_paused").unwrap_or(false);
        options.auto_managed = config.boolean("auto_managed").unwrap_or(true);
        if config.boolean("pre_allocate_storage").unwrap_or(false) {
            options.storage_mode = "allocate".to_owned();
        }
        options.prioritize_first_last = config
            .boolean("prioritize_first_last_pieces")
            .unwrap_or(false);
        options.sequential_download = config.boolean("sequential_download").unwrap_or(false);
        options.super_seeding = config.boolean("super_seeding").unwrap_or(false);
        options.shared = config.boolean("shared").unwrap_or(false);

        options.move_completed = config.boolean("move_completed").unwrap_or(false);
        if let Some(path) = config.string("move_completed_path") {
            if !path.is_empty() {
                options.move_completed_path = Some(path.to_owned());
            }
        }

        options.stop_at_ratio = config.boolean("stop_seed_at_ratio").unwrap_or(false);
        options.stop_ratio = config.number("stop_seed_ratio").unwrap_or(2.0);
        options.remove_at_ratio = config.boolean("remove_seed_at_ratio").unwrap_or(false);

        options.max_connections = config.integer("max_connections_per_torrent").unwrap_or(-1);
        options.max_upload_slots = config.integer("max_upload_slots_per_torrent").unwrap_or(-1);
        options.max_download_speed = config
            .number("max_download_speed_per_torrent")
            .unwrap_or(-1.0);
        options.max_upload_speed = config
            .number("max_upload_speed_per_torrent")
            .unwrap_or(-1.0);

        options
    }

    /// Adds a torrent and remembers its options.
    ///
    /// Public because a watched directory adds torrents without an RPC call
    /// ever arriving, and it has to go through exactly the same path as one
    /// that did.
    pub async fn add(
        &self,
        mut request: AddTorrent,
        options: TorrentOptions,
    ) -> Result<Value, RpcError> {
        // A torrent with no save path of its own goes where the config says.
        if request.save_path.is_empty() {
            let config = self.config.lock().await;
            request.save_path = config
                .string("download_location")
                .unwrap_or_default()
                .to_owned();
        }
        if request.save_path.is_empty() {
            return Err(RpcError::invalid_argument("no download location is set"));
        }

        // `add_paused` is in the defaults every caller starts from now, so a
        // client that asks for a running torrent gets one even when the
        // configuration says otherwise. That is Deluge's order: the dictionary
        // the client sends is applied over the configured defaults.
        let mut options = options;
        options.save_path = Some(request.save_path.clone());
        request.flags = request
            .flags
            .set_to(flags::PAUSED, options.paused)
            .set_to(flags::AUTO_MANAGED, options.auto_managed);

        let stored = options.clone();
        // Kept so the torrent can be restored after a restart. Without it a
        // torrent added from a file comes back as nothing at all.
        let torrent_file = request.torrent_file.clone();

        // Read before the session is locked, because both want the config.
        let (queue_to_top, keep_a_copy) = {
            let config = self.config.lock().await;
            (
                config.boolean("queue_new_to_top").unwrap_or(false),
                config
                    .boolean("copy_torrent_file")
                    .unwrap_or(false)
                    .then(|| {
                        config
                            .string("torrentfiles_location")
                            .unwrap_or_default()
                            .to_owned()
                    }),
            )
        };
        let copy_name = stored.filename.clone();

        let id = self
            .manager
            .with(move |state| {
                let id = state.session.add_torrent(&request)?;

                if !torrent_file.is_empty() {
                    let path = crate::manager::torrent_file_path(&state.state_dir(), &id);
                    if let Err(err) = std::fs::create_dir_all(state.state_dir())
                        .and_then(|()| std::fs::write(&path, &torrent_file))
                    {
                        tracing::error!(torrent = %id, error = %err,
                            "could not store the torrent file; it will not survive a restart");
                    }
                }

                // The user's own copy, which is a different thing from the one
                // above: that one is the daemon's, under the state directory,
                // and is deleted with the torrent.
                if let (Some(directory), false) = (&keep_a_copy, torrent_file.is_empty()) {
                    copy_torrent_file(directory, &copy_name, &id, &torrent_file);
                }

                // Limits and flags that only `core.set_torrent_options` used to
                // apply, so a torrent added with them ran without them until
                // something set them again.
                apply_limits(state, &id, &stored);

                if queue_to_top {
                    let _ = state.session.queue_top(&id);
                }

                let mut stored = stored;
                stored.torrent_id = id.clone();
                state
                    .torrents
                    .insert(id.clone(), Torrent::new(id.clone(), stored));
                state.mark_dirty();
                let _ = state.save_state();
                Ok::<String, redeluge_libtorrent::Error>(id)
            })
            .await
            .map_err(|err| RpcError::invalid_argument(err.to_string()))?
            .map_err(|err| RpcError::invalid_argument(err.to_string()))?;

        self.manager.announce(Event::TorrentAdded {
            torrent_id: id.clone(),
            from_state: false,
        });
        Ok(Value::Str(id))
    }
}

fn string_arg(args: &[Value], index: usize, what: &str) -> Result<String, RpcError> {
    args.get(index)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| RpcError::invalid_argument(format!("{what} is required")))
}

fn bytes_arg(args: &[Value], index: usize, what: &str) -> Result<Vec<u8>, RpcError> {
    match args.get(index) {
        Some(Value::Bytes(raw)) => Ok(raw.clone()),
        // Clients that cannot send raw bytes send base64 text, which is what
        // the Web UI does when it forwards an uploaded file.
        Some(Value::Str(text)) => base64_decode(text)
            .ok_or_else(|| RpcError::invalid_argument(format!("{what} is not valid base64"))),
        _ => Err(RpcError::invalid_argument(format!("{what} is required"))),
    }
}

/// Minimal base64, because one decode does not justify a dependency.
fn base64_decode(text: &str) -> Option<Vec<u8>> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut lookup = [255u8; 256];
    for (index, byte) in TABLE.iter().enumerate() {
        lookup[*byte as usize] = index as u8;
    }

    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let mut buffer = 0u32;
    let mut bits = 0u32;

    for byte in text.bytes() {
        if byte == b'=' || byte.is_ascii_whitespace() {
            continue;
        }
        let value = lookup[byte as usize];
        if value == 255 {
            return None;
        }
        buffer = (buffer << 6) | u32::from(value);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    Some(out)
}

fn torrent_ids(args: &[Value], index: usize) -> Vec<String> {
    match args.get(index) {
        Some(Value::List(items)) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_owned))
            .collect(),
        Some(Value::Str(single)) => vec![single.clone()],
        _ => Vec::new(),
    }
}

/// The keys a client asked for, or all of them when it asked for none.
fn wanted_keys(args: &[Value], index: usize) -> Option<Vec<String>> {
    match args.get(index) {
        Some(Value::List(items)) if !items.is_empty() => Some(
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect(),
        ),
        _ => None,
    }
}

fn filtered(status: BTreeMap<String, Value>, keys: &Option<Vec<String>>) -> Vec<(Value, Value)> {
    match keys {
        None => status
            .into_iter()
            .map(|(key, value)| (Value::Str(key), value))
            .collect(),
        Some(wanted) => wanted
            .iter()
            .filter_map(|key| {
                status
                    .get(key)
                    .map(|value| (Value::Str(key.clone()), value.clone()))
            })
            .collect(),
    }
}

#[async_trait]
impl Rpc for Core {
    fn auth_level(&self, method: &str) -> Option<AuthLevel> {
        // The plugin's methods are not in the contract, which was extracted
        // from the daemon core. They take the level the plugin gave them,
        // which is the ordinary one: reading a label list is not privileged,
        // and changing one is no more privileged than changing any other
        // torrent option.
        if PLUGIN_METHODS.contains(&method) {
            return Some(
                if method.starts_with("label.get") || method == "core.get_enabled_plugins" {
                    AuthLevel::ReadOnly
                } else {
                    AuthLevel::Normal
                },
            );
        }
        // core.get_auth_levels_mappings is level 0 in the Python daemon, which
        // makes it reachable before a client has proved anything. Raised here,
        // as the phase 0 report said it should be.
        if method == "core.get_auth_levels_mappings" {
            return Some(AuthLevel::ReadOnly);
        }
        Contract::get()
            .method(method)
            .filter(|entry| entry.transport == Transport::Daemon)
            .and_then(|entry| AuthLevel::from_i64(entry.auth_level.as_u8().into()))
    }

    fn method_list(&self) -> Vec<String> {
        let mut methods: Vec<String> = Contract::get()
            .methods_for(Transport::Daemon)
            .map(|entry| entry.name.clone())
            .collect();
        // A Deluge daemon advertises its plugins' methods too, and this one
        // answers the Label plugin's. Leaving them out would be the same lie
        // in the other direction: a client that reads the list would not find
        // what it can call.
        methods.extend(PLUGIN_METHODS.iter().map(|name| (*name).to_owned()));
        methods.sort();
        methods
    }

    fn version(&self) -> String {
        REPORTED_VERSION.to_owned()
    }

    async fn authenticate(&self, username: &str, password: &str) -> Result<AuthLevel, RpcError> {
        let mut auth = self.auth.lock().await;
        auth.authorize(username, password).map_err(|err| match err {
            crate::auth::Error::UnknownAccount(_) => RpcError::bad_login("Username does not exist"),
            crate::auth::Error::BadPassword => RpcError::bad_login("Password does not match"),
            other => RpcError::new("AuthManagerError", other.to_string()),
        })
    }

    async fn disconnected(&self, session_id: i64) {
        tracing::debug!(session_id, "client gone");
    }

    async fn set_event_interest(&self, _session_id: i64, _events: Vec<String>) {}

    async fn call(
        &self,
        context: &CallContext,
        method: &str,
        args: Vec<Value>,
        _kwargs: Vec<(Value, Value)>,
    ) -> Result<Value, RpcError> {
        match method {
            // ------------------------------------------------------ the daemon
            "daemon.get_version" => Ok(Value::Str(REPORTED_VERSION.to_owned())),
            "daemon.shutdown" => {
                tracing::info!(by = %context.username, "shutdown requested");
                self.shutdown.notify_waiters();
                Ok(Value::None)
            }

            // ------------------------------------------------------- adding
            "core.add_torrent_file" | "core.add_torrent_file_async" => {
                let filename = string_arg(&args, 0, "a filename")?;
                let dump = bytes_arg(&args, 1, "the torrent file")?;
                let options = options_from(args.get(2), self.torrent_defaults().await);
                let mut stored = options.clone();
                stored.filename = filename;
                self.add(AddTorrent::from_file(dump, String::new()), stored)
                    .await
            }
            "core.add_torrent_magnet" => {
                let uri = string_arg(&args, 0, "a magnet uri")?;
                let mut stored = options_from(args.get(1), self.torrent_defaults().await);
                stored.magnet = Some(uri.clone());
                self.add(AddTorrent::from_magnet(uri, String::new()), stored)
                    .await
            }

            // ------------------------------------------------------ removing
            "core.remove_torrent" => {
                let id = string_arg(&args, 0, "a torrent id")?;
                let with_data = args.get(1).and_then(Value::as_bool).unwrap_or(false);

                self.manager.announce(Event::PreTorrentRemoved {
                    torrent_id: id.clone(),
                });
                let removed = {
                    let id = id.clone();
                    self.manager
                        .with(move |state| {
                            let outcome = state.session.remove_torrent(&id, with_data);
                            state.torrents.remove(&id);

                            state.forget(&id);
                            state.mark_dirty();
                            let _ = state.save_state();
                            outcome
                        })
                        .await
                };

                match removed {
                    Ok(Ok(())) => {
                        self.manager
                            .announce(Event::TorrentRemoved { torrent_id: id });
                        Ok(Value::Bool(true))
                    }
                    Ok(Err(err)) => Err(RpcError::new("InvalidTorrentError", err.to_string())),
                    Err(err) => Err(RpcError::invalid_argument(err.to_string())),
                }
            }

            // ------------------------------------------------------ control
            "core.pause_torrent" | "core.resume_torrent" => {
                let pause = method == "core.pause_torrent";
                let ids = match args.first() {
                    Some(Value::List(_)) => torrent_ids(&args, 0),
                    Some(Value::Str(single)) => vec![single.clone()],
                    _ => return Err(RpcError::invalid_argument("a torrent id is required")),
                };

                self.manager
                    .with(move |state| {
                        for id in ids {
                            // Pausing by hand also turns off auto-management,
                            // or the queue would start it again immediately.
                            // Resuming by hand ends any hold the idle rule
                            // had on this torrent. The person asked for it to
                            // run; a rule that put it back a moment later
                            // would be arguing with them.
                            if !pause {
                                if let Some(torrent) = state.torrents.get_mut(&id) {
                                    torrent.options.idle_resume_at = 0.0;
                                }
                            }
                            let change = FlagChange::new()
                                .set_to(flags::PAUSED, pause)
                                .set_to(flags::AUTO_MANAGED, !pause);
                            let _ = state.session.set_flags(&id, change);
                            if let Some(torrent) = state.torrents.get_mut(&id) {
                                torrent.options.paused = pause;
                                torrent.options.auto_managed = !pause;
                            }
                        }
                        state.mark_dirty();
                    })
                    .await
                    .map_err(|err| RpcError::invalid_argument(err.to_string()))?;
                Ok(Value::None)
            }

            "core.pause_session" | "core.resume_session" => {
                self.set_session_paused(method == "core.pause_session")
                    .await
                    .map_err(RpcError::invalid_argument)?;
                Ok(Value::None)
            }

            "core.is_session_paused" => {
                let paused = self
                    .manager
                    .with(|state| state.session_paused)
                    .await
                    .unwrap_or(false);
                Ok(Value::Bool(paused))
            }

            "core.force_recheck" => {
                let ids = torrent_ids(&args, 0);
                self.manager
                    .with(move |state| {
                        for id in ids {
                            let _ = state.session.force_recheck(&id);
                        }
                    })
                    .await
                    .map_err(|err| RpcError::invalid_argument(err.to_string()))?;
                Ok(Value::None)
            }

            "core.force_reannounce" => {
                let ids = torrent_ids(&args, 0);
                self.manager
                    .with(move |state| {
                        for id in ids {
                            let _ = state.session.force_reannounce(&id, 0);
                        }
                    })
                    .await
                    .map_err(|err| RpcError::invalid_argument(err.to_string()))?;
                Ok(Value::None)
            }

            "core.move_storage" => {
                let ids = torrent_ids(&args, 0);
                let destination = string_arg(&args, 1, "a destination")?;
                self.manager
                    .with(move |state| {
                        for id in ids {
                            if state.session.move_storage(&id, &destination).is_ok() {
                                if let Some(torrent) = state.torrents.get_mut(&id) {
                                    torrent.moving_to = Some(destination.clone());
                                }
                            }
                        }
                    })
                    .await
                    .map_err(|err| RpcError::invalid_argument(err.to_string()))?;
                Ok(Value::None)
            }

            "core.queue_top" | "core.queue_up" | "core.queue_down" | "core.queue_bottom" => {
                let ids = torrent_ids(&args, 0);
                let which = method.to_owned();
                self.manager
                    .with(move |state| {
                        for id in ids {
                            let _ = match which.as_str() {
                                "core.queue_top" => state.session.queue_top(&id),
                                "core.queue_up" => state.session.queue_up(&id),
                                "core.queue_down" => state.session.queue_down(&id),
                                _ => state.session.queue_bottom(&id),
                            };
                        }
                    })
                    .await
                    .map_err(|err| RpcError::invalid_argument(err.to_string()))?;
                self.manager.announce(Event::TorrentQueueChanged);
                Ok(Value::None)
            }

            "core.rename_files" => {
                let id = string_arg(&args, 0, "a torrent id")?;
                let renames: Vec<(i64, String)> = args
                    .get(1)
                    .and_then(Value::as_list)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| {
                                let pair = item.as_list()?;
                                Some((pair.first()?.as_i64()?, pair.get(1)?.as_str()?.to_owned()))
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                self.manager
                    .with(move |state| {
                        for (index, name) in renames {
                            let _ = state.session.rename_file(&id, index as i32, &name);
                        }
                    })
                    .await
                    .map_err(|err| RpcError::invalid_argument(err.to_string()))?;
                Ok(Value::None)
            }

            "core.set_torrent_options" => {
                let ids = torrent_ids(&args, 0);
                let options = args.get(1).cloned().unwrap_or(Value::Dict(Vec::new()));
                self.apply_torrent_options(ids, options).await
            }

            // Deluge deprecated these in favour of set_torrent_options and kept
            // them working. Every one is that call with a single key, so they
            // are written as such rather than duplicated.
            "core.set_torrent_max_connections"
            | "core.set_torrent_max_upload_slots"
            | "core.set_torrent_max_upload_speed"
            | "core.set_torrent_max_download_speed"
            | "core.set_torrent_prioritize_first_last"
            | "core.set_torrent_auto_managed"
            | "core.set_torrent_stop_at_ratio"
            | "core.set_torrent_stop_ratio"
            | "core.set_torrent_remove_at_ratio"
            | "core.set_torrent_move_completed"
            | "core.set_torrent_move_completed_path"
            | "core.set_torrent_file_priorities" => {
                let id = string_arg(&args, 0, "a torrent id")?;
                let value = args
                    .get(1)
                    .cloned()
                    .ok_or_else(|| RpcError::invalid_argument("a value is required"))?;
                let key = method
                    .strip_prefix("core.set_torrent_")
                    .expect("matched above");
                // One exception to the naming: the option is called
                // prioritize_first_last_pieces.
                let key = if key == "prioritize_first_last" {
                    "prioritize_first_last_pieces"
                } else {
                    key
                };

                self.apply_torrent_options(
                    vec![id],
                    Value::Dict(vec![(Value::Str(key.to_owned()), value)]),
                )
                .await
            }

            "core.set_torrent_trackers" => {
                let id = string_arg(&args, 0, "a torrent id")?;
                let trackers: Vec<(String, u8)> = args
                    .get(1)
                    .and_then(Value::as_list)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| {
                                let url = item.get("url")?.as_str()?.to_owned();
                                let tier = item
                                    .get("tier")
                                    .and_then(Value::as_i64)
                                    .unwrap_or(0)
                                    .clamp(0, 255) as u8;
                                Some((url, tier))
                            })
                            .collect()
                    })
                    .unwrap_or_default();

                let stored = trackers.clone();
                self.manager
                    .with(move |state| {
                        let urls: Vec<String> =
                            trackers.iter().map(|(url, _)| url.clone()).collect();
                        let tiers: Vec<u8> = trackers.iter().map(|(_, tier)| *tier).collect();
                        let _ = state.session.replace_trackers(&id, &urls, &tiers);
                        if let Some(torrent) = state.torrents.get_mut(&id) {
                            torrent.options.trackers = stored
                                .into_iter()
                                .map(|(url, tier)| crate::torrent::TrackerOption { url, tier })
                                .collect();
                        }
                        state.mark_dirty();
                    })
                    .await
                    .map_err(|err| RpcError::invalid_argument(err.to_string()))?;
                Ok(Value::None)
            }

            "core.connect_peer" => {
                let id = string_arg(&args, 0, "a torrent id")?;
                let ip = string_arg(&args, 1, "an ip address")?;
                let port = args
                    .get(2)
                    .and_then(Value::as_i64)
                    .ok_or_else(|| RpcError::invalid_argument("a port is required"))?;

                self.manager
                    .with(move |state| {
                        state
                            .session
                            .connect_peer(&id, &ip, port.clamp(0, 65535) as u16)
                    })
                    .await
                    .map_err(|err| RpcError::invalid_argument(err.to_string()))?
                    .map_err(|err| RpcError::invalid_argument(err.to_string()))?;
                Ok(Value::None)
            }

            "core.get_magnet_uri" => {
                let id = string_arg(&args, 0, "a torrent id")?;
                let uri = self
                    .manager
                    .with(move |state| {
                        let torrent = state.torrents.get(&id)?;
                        torrent.options.magnet.clone().or_else(|| {
                            // A torrent added from a file still has a magnet:
                            // the infohash is all a magnet needs.
                            Some(format!("magnet:?xt=urn:btih:{}", torrent.id))
                        })
                    })
                    .await
                    .map_err(|err| RpcError::invalid_argument(err.to_string()))?;
                Ok(uri.map(Value::Str).unwrap_or(Value::None))
            }

            "core.get_proxy" => {
                let config = self.config.lock().await;
                Ok(config
                    .get("proxy")
                    .map(json_to_value)
                    .unwrap_or(Value::Dict(Vec::new())))
            }

            "core.get_ssl_listen_port" => {
                let config = self.config.lock().await;
                let port = config
                    .get("ssl_listen_ports")
                    .and_then(|value| value.as_array())
                    .and_then(|ports| ports.first())
                    .and_then(|port| port.as_i64())
                    .unwrap_or(0);
                Ok(Value::Int(port))
            }

            // The plural forms, which every client uses for a multi-selection.
            "core.pause_torrents" => {
                Box::pin(self.call(context, "core.pause_torrent", args, _kwargs)).await
            }
            "core.resume_torrents" => {
                Box::pin(self.call(context, "core.resume_torrent", args, _kwargs)).await
            }
            "core.remove_torrents" => {
                let ids = torrent_ids(&args, 0);
                let with_data = args.get(1).and_then(Value::as_bool).unwrap_or(false);

                // Deluge returns the ones it could not remove, so a client can
                // say which failed rather than just that something did.
                let mut failures = Vec::new();
                for id in ids {
                    let single = vec![Value::Str(id.clone()), Value::Bool(with_data)];
                    if let Err(err) =
                        Box::pin(self.call(context, "core.remove_torrent", single, Vec::new()))
                            .await
                    {
                        failures.push(Value::List(vec![Value::Str(id), Value::Str(err.message)]));
                    }
                }
                Ok(Value::List(failures))
            }

            "core.add_torrent_files" => {
                // Each entry is (filename, filedump, options).
                let files = args
                    .first()
                    .and_then(Value::as_list)
                    .ok_or_else(|| {
                        RpcError::invalid_argument("a list of torrent files is required")
                    })?
                    .to_vec();

                let mut failures = Vec::new();
                for entry in files {
                    let Some(fields) = entry.as_list() else {
                        continue;
                    };
                    let single = fields.to_vec();
                    if let Err(err) =
                        Box::pin(self.call(context, "core.add_torrent_file", single, Vec::new()))
                            .await
                    {
                        failures.push(Value::List(vec![
                            fields.first().cloned().unwrap_or(Value::None),
                            Value::Str(err.message),
                        ]));
                    }
                }
                Ok(Value::List(failures))
            }

            "core.rename_folder" => {
                let id = string_arg(&args, 0, "a torrent id")?;
                let old = string_arg(&args, 1, "the current folder")?;
                let new = string_arg(&args, 2, "the new folder")?;

                // libtorrent has no folder rename: a folder is a prefix on file
                // paths, so this renames every file under it.
                let renamed = {
                    let (id, old, new) = (id.clone(), old.clone(), new.clone());
                    self.manager
                        .with(move |state| {
                            let files = state.session.files(&id).unwrap_or_default();
                            let prefix = old.trim_end_matches('/').to_owned() + "/";
                            let mut count = 0;
                            for file in files {
                                if let Some(rest) = file.path.strip_prefix(&prefix) {
                                    let target = format!("{}/{rest}", new.trim_end_matches('/'));
                                    if state.session.rename_file(&id, file.index, &target).is_ok() {
                                        count += 1;
                                    }
                                }
                            }
                            count
                        })
                        .await
                        .map_err(|err| RpcError::invalid_argument(err.to_string()))?
                };

                if renamed == 0 {
                    return Err(RpcError::invalid_argument(format!("no files under {old}")));
                }
                self.manager.announce(Event::TorrentFolderRenamed {
                    torrent_id: id,
                    old,
                    new,
                });
                Ok(Value::None)
            }

            "core.glob" => {
                // Used by the path chooser to complete a directory.
                let pattern = string_arg(&args, 0, "a path")?;
                Ok(Value::List(glob_directory(&pattern)))
            }

            "core.get_completion_paths" => {
                let request = args.first().cloned().unwrap_or(Value::Dict(Vec::new()));
                let path = request
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                Ok(Value::Dict(vec![
                    (Value::Str("value".into()), Value::Str(path.clone())),
                    (
                        Value::Str("paths".into()),
                        Value::List(glob_directory(&path)),
                    ),
                ]))
            }

            "core.add_torrent_url" => {
                let url = string_arg(&args, 0, "a url")?;
                let options = args.get(2).cloned();
                let dump = fetch(&url).await?;

                let mut stored = options_from(options.as_ref(), self.torrent_defaults().await);
                stored.filename = filename_from_url(&url);
                self.add(AddTorrent::from_file(dump, String::new()), stored)
                    .await
            }

            "core.test_listen_port" => {
                // Deluge asks its own site whether the port is reachable. A
                // failure here means the check could not run, not that the port
                // is closed, so it answers None rather than false.
                let port = self
                    .manager
                    .with(|state| state.session.listen_port())
                    .await
                    .unwrap_or(0);
                if port == 0 {
                    return Ok(Value::None);
                }

                let url = format!("https://deluge-torrent.org/test_port.php?port={port}");
                match fetch(&url).await {
                    Ok(body) => Ok(Value::Bool(body.starts_with(b"1"))),
                    Err(err) => {
                        tracing::debug!(error = %err.message, "port test failed");
                        Ok(Value::None)
                    }
                }
            }

            "core.prefetch_magnet_metadata" => {
                let uri = string_arg(&args, 0, "a magnet uri")?;
                let timeout = args
                    .get(1)
                    .and_then(Value::as_i64)
                    .unwrap_or(30)
                    .clamp(1, 120);
                self.prefetch_metadata(&uri, timeout as u64).await
            }

            "core.create_torrent" => {
                let path = string_arg(&args, 0, "a path")?;
                let trackers: Vec<String> = args
                    .get(1)
                    .and_then(Value::as_list)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| item.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                let piece_length = args
                    .get(2)
                    .and_then(Value::as_i64)
                    .unwrap_or(32 * 1024)
                    .clamp(16 * 1024, 64 * 1024 * 1024) as i32;
                let comment = args.get(3).and_then(Value::as_str).unwrap_or("").to_owned();
                let target = args.get(4).and_then(Value::as_str).map(str::to_owned);
                let web_seeds: Vec<String> = args
                    .get(5)
                    .and_then(Value::as_list)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| item.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                let private = args.get(6).and_then(Value::as_bool).unwrap_or(false);
                let creator = args
                    .get(7)
                    .and_then(Value::as_str)
                    .unwrap_or(concat!("redeluge ", "2.2.1"))
                    .to_owned();

                // Hashing reads every byte of the content, which can take
                // minutes. On the async runtime that would stall every other
                // client, so it goes to a blocking thread.
                //
                // Progress crosses back from the hashing loop in C++. It is
                // throttled to a hundred events for the whole run: a torrent
                // can have hundreds of thousands of pieces, and one event each
                // would drown every client to tell them about a progress bar.
                let manager = self.manager.clone();
                let built = tokio::task::spawn_blocking(move || {
                    let mut last_reported = -1i64;
                    let mut progress =
                        redeluge_libtorrent::HashProgress::new(Box::new(move |piece, total| {
                            let total = i64::from(total).max(1);
                            let piece = i64::from(piece) + 1;
                            let step = (total / 100).max(1);
                            if piece % step != 0 && piece != total {
                                return;
                            }
                            if piece == last_reported {
                                return;
                            }
                            last_reported = piece;
                            manager.announce(Event::CreateTorrentProgress {
                                piece_count: piece,
                                num_pieces: total,
                            });
                        }));

                    redeluge_libtorrent::Session::create_torrent_with_progress(
                        &path,
                        piece_length,
                        &comment,
                        &creator,
                        private,
                        &trackers,
                        &web_seeds,
                        &mut progress,
                    )
                })
                .await
                .map_err(|err| RpcError::invalid_argument(err.to_string()))?
                .map_err(|err| RpcError::invalid_argument(err.to_string()))?;

                match target {
                    Some(path) if !path.is_empty() => {
                        std::fs::write(&path, &built).map_err(|err| {
                            RpcError::invalid_argument(format!("could not write {path}: {err}"))
                        })?;
                        Ok(Value::None)
                    }
                    _ => Ok(Value::Bytes(built)),
                }
            }

            "core.set_ssl_torrent_cert" => {
                let id = string_arg(&args, 0, "a torrent id")?;
                let certificate = bytes_arg(&args, 1, "a certificate")?;
                let private_key = bytes_arg(&args, 2, "a private key")?;
                let dh_params = args
                    .get(3)
                    .and_then(|value| match value {
                        Value::Bytes(raw) => Some(raw.clone()),
                        Value::Str(text) => base64_decode(text),
                        _ => None,
                    })
                    .unwrap_or_default();

                self.manager
                    .with(move |state| {
                        state.session.set_ssl_certificate(
                            &id,
                            &certificate,
                            &private_key,
                            &dh_params,
                            "",
                        )
                    })
                    .await
                    .map_err(|err| RpcError::invalid_argument(err.to_string()))?
                    .map_err(|err| RpcError::invalid_argument(err.to_string()))?;
                Ok(Value::None)
            }

            "daemon.authorized_call" => {
                let wanted = string_arg(&args, 0, "a method name")?;
                let allowed = self
                    .auth_level(&wanted)
                    .map(|required| context.level >= required)
                    .unwrap_or(false);
                Ok(Value::Bool(allowed))
            }

            // -------------------------------------------------------- status
            "core.get_torrent_status" => {
                let id = string_arg(&args, 0, "a torrent id")?;
                let keys = wanted_keys(&args, 1);
                // The peers tab is the only caller that asks for them, and it
                // asks for that key alone.
                let peers = keys
                    .as_ref()
                    .map(|keys| keys.iter().any(|key| key == "peers"))
                    .unwrap_or(true);
                // The file list is three more calls into libtorrent, so like
                // the peers it is only paid for when a client asks.
                let files = keys
                    .as_ref()
                    .map(|keys| {
                        keys.iter().any(|key| {
                            matches!(key.as_str(), "files" | "file_progress" | "file_priorities")
                        })
                    })
                    .unwrap_or(true);
                let status = self.status_of_with(&id, peers, files).await?;
                Ok(Value::Dict(filtered(status, &keys)))
            }

            "core.get_torrents_status" => {
                let filter = args.first().cloned();
                let keys = wanted_keys(&args, 1);
                self.all_status(filter, keys).await
            }

            "core.get_session_status" => {
                let wanted: Vec<String> = args
                    .first()
                    .and_then(Value::as_list)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| item.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                self.session_status(wanted).await
            }

            "core.get_filter_tree" => self.filter_tree().await,

            // ------------------------------------------------ the Label plugin
            //
            // Deluge's plugin surface, answered by the feature that replaced
            // it. See PLUGIN_METHODS for why this is here at all.
            "core.get_enabled_plugins" | "core.get_available_plugins" => {
                Ok(Value::List(vec![Value::Str(EMULATED_PLUGIN.to_owned())]))
            }
            "core.enable_plugin" | "core.disable_plugin" => {
                let wanted = string_arg(&args, 0, "a plugin name")?;
                if !wanted.eq_ignore_ascii_case(EMULATED_PLUGIN) {
                    // Truthful rather than silent: there is no plugin system,
                    // so nothing else can ever be turned on.
                    return Ok(Value::Bool(false));
                }
                // Labels are part of the daemon; they cannot be turned off.
                Ok(Value::Bool(method == "core.enable_plugin"))
            }

            "label.get_labels" => {
                let labels = self.labels().await;
                Ok(Value::List(
                    labels.names().into_iter().map(Value::Str).collect(),
                ))
            }
            "label.add" => {
                let id = normalise_label(&string_arg(&args, 0, "a label")?);
                if id.is_empty() {
                    return Err(RpcError::invalid_argument("a label cannot be empty"));
                }
                let mut labels = self.labels().await;
                if !labels.add(&id) {
                    // The plugin raised here. Clients add before every use and
                    // swallow the error, so the outcome is the same either
                    // way; this one says what happened without an exception.
                    return Ok(Value::Bool(false));
                }
                self.store_labels(&labels).await?;
                tracing::info!(label = %id, "label added");
                Ok(Value::Bool(true))
            }
            "label.remove" => {
                let id = normalise_label(&string_arg(&args, 0, "a label")?);
                let mut labels = self.labels().await;
                if !labels.remove(&id) {
                    return Ok(Value::Bool(false));
                }
                self.store_labels(&labels).await?;
                // Torrents keep their label as an option, so removing the
                // label from the register without clearing them would leave
                // torrents in a group that no longer exists.
                let cleared = self.clear_label(&id).await;
                tracing::info!(label = %id, torrents = cleared, "label removed");
                Ok(Value::Bool(true))
            }
            "label.get_options" => {
                let id = normalise_label(&string_arg(&args, 0, "a label")?);
                let labels = self.labels().await;
                let Some(options) = labels.options(&id) else {
                    return Err(RpcError::invalid_argument(format!("no such label: {id}")));
                };
                Ok(json_to_value(
                    &serde_json::to_value(options).unwrap_or(serde_json::Value::Null),
                ))
            }
            "label.set_options" => {
                let id = normalise_label(&string_arg(&args, 0, "a label")?);
                let Some(given) = args.get(1) else {
                    return Err(RpcError::invalid_argument("options are required"));
                };
                let mut labels = self.labels().await;
                if !labels.contains(&id) {
                    return Err(RpcError::invalid_argument(format!("no such label: {id}")));
                }

                // Merged over what is stored, because a client that sets one
                // option sends one option.
                let mut merged =
                    serde_json::to_value(labels.options(&id).cloned().unwrap_or_default())
                        .unwrap_or_else(|_| serde_json::json!({}));
                if let (Some(target), serde_json::Value::Object(changes)) =
                    (merged.as_object_mut(), value_to_json(given))
                {
                    for (key, value) in changes {
                        target.insert(key, value);
                    }
                }
                let options: crate::features::label::Options = serde_json::from_value(merged)
                    .map_err(|err| RpcError::invalid_argument(err.to_string()))?;

                labels.labels.insert(id.clone(), options.clone());
                self.store_labels(&labels).await?;

                // Applied to what already carries the label, which is what the
                // plugin did: the options are the label's, not the torrent's.
                self.apply_label_options(&id, &options).await;
                Ok(Value::None)
            }
            "label.set_torrent" => {
                let torrent_id = string_arg(&args, 0, "a torrent id")?;
                let id = normalise_label(&string_arg(&args, 1, "a label")?);
                self.assign_label(&torrent_id, &id).await?;
                Ok(Value::None)
            }
            "label.get_config" => {
                let labels = self.labels().await;
                Ok(json_to_value(&labels.to_json()))
            }
            "label.set_config" => {
                let Some(given) = args.first() else {
                    return Err(RpcError::invalid_argument("a dictionary is required"));
                };
                let settings =
                    crate::features::label::Settings::from_config(Some(&value_to_json(given)));
                self.store_labels(&settings).await?;
                Ok(Value::None)
            }

            "core.get_session_state" => {
                let hashes = self
                    .manager
                    .with(|state| state.session.torrent_hashes())
                    .await
                    .unwrap_or_default();
                Ok(Value::List(hashes.into_iter().map(Value::Str).collect()))
            }

            "core.get_external_ip" => {
                let ip = self
                    .manager
                    .with(|state| state.external_ip.clone())
                    .await
                    .ok()
                    .flatten()
                    .unwrap_or_default();
                Ok(Value::Str(ip))
            }

            "core.get_listen_port" => {
                let port = self
                    .manager
                    .with(|state| state.session.listen_port())
                    .await
                    .unwrap_or(0);
                Ok(Value::Int(i64::from(port)))
            }

            "core.get_libtorrent_version" => {
                Ok(Value::Str(redeluge_libtorrent::libtorrent_version()))
            }

            "core.get_free_space" | "core.get_available_space" => {
                let path = match args.first().and_then(Value::as_str) {
                    Some(path) if !path.is_empty() => path.to_owned(),
                    _ => {
                        let config = self.config.lock().await;
                        config.string("download_location").unwrap_or("/").to_owned()
                    }
                };
                Ok(Value::Int(free_space(&path)))
            }

            "core.get_path_size" => {
                let path = string_arg(&args, 0, "a path")?;
                Ok(Value::Int(path_size(&path)))
            }

            // ------------------------------------------------------- config
            "core.get_config" => {
                let config = self.config.lock().await;
                Ok(json_map_to_value(config.all()))
            }

            "core.get_config_value" => {
                let key = string_arg(&args, 0, "a key")?;
                let config = self.config.lock().await;
                Ok(config.get(&key).map(json_to_value).unwrap_or(Value::None))
            }

            "core.get_config_values" => {
                let keys: Vec<String> = args
                    .first()
                    .and_then(Value::as_list)
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| item.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                let config = self.config.lock().await;
                Ok(Value::Dict(
                    keys.into_iter()
                        .map(|key| {
                            let value = config.get(&key).map(json_to_value).unwrap_or(Value::None);
                            (Value::Str(key), value)
                        })
                        .collect(),
                ))
            }

            "core.set_config" => {
                let Some(Value::Dict(changes)) = args.first() else {
                    return Err(RpcError::invalid_argument("a dictionary is required"));
                };

                let mut applied = Vec::new();
                {
                    let mut config = self.config.lock().await;
                    for (key, value) in changes {
                        let Some(key) = key.as_str() else { continue };
                        let json = value_to_json(value);
                        match config.set(key, json.clone()) {
                            Ok(()) => applied.push((key.to_owned(), value.clone())),
                            Err(err) => {
                                // One bad key must not lose the rest, and the
                                // client should hear which one was refused.
                                tracing::warn!(key, error = %err, "refused a config change");
                            }
                        }
                    }
                    let _ = config.save();
                }

                for (key, value) in applied {
                    self.manager
                        .announce(Event::ConfigValueChanged { key, value });
                }
                self.apply_config().await;
                Ok(Value::None)
            }

            // -------------------------------------------------------- accounts
            "core.get_auth_levels_mappings" => Ok(Value::List(vec![
                Value::Dict(
                    [
                        AuthLevel::None,
                        AuthLevel::ReadOnly,
                        AuthLevel::Normal,
                        AuthLevel::Admin,
                    ]
                    .into_iter()
                    .map(|level| {
                        (
                            Value::Str(level.as_str().to_owned()),
                            Value::Int(level.as_i64()),
                        )
                    })
                    .collect(),
                ),
                Value::Dict(
                    [
                        AuthLevel::None,
                        AuthLevel::ReadOnly,
                        AuthLevel::Normal,
                        AuthLevel::Admin,
                    ]
                    .into_iter()
                    .map(|level| {
                        (
                            Value::Int(level.as_i64()),
                            Value::Str(level.as_str().to_owned()),
                        )
                    })
                    .collect(),
                ),
            ])),

            "core.get_known_accounts" => {
                let auth = self.auth.lock().await;
                Ok(Value::List(
                    auth.accounts()
                        .into_iter()
                        .map(|(name, level)| {
                            Value::Dict(vec![
                                (Value::Str("username".into()), Value::Str(name)),
                                (
                                    Value::Str("authlevel".into()),
                                    Value::Str(level.as_str().to_owned()),
                                ),
                                (
                                    Value::Str("authlevel_int".into()),
                                    Value::Int(level.as_i64()),
                                ),
                            ])
                        })
                        .collect(),
                ))
            }

            "core.create_account" | "core.update_account" => {
                let username = string_arg(&args, 0, "a username")?;
                let password = string_arg(&args, 1, "a password")?;
                let level = args
                    .get(2)
                    .and_then(Value::as_str)
                    .and_then(AuthLevel::from_name)
                    .ok_or_else(|| RpcError::invalid_argument("an auth level is required"))?;

                let mut auth = self.auth.lock().await;
                let outcome = if method == "core.create_account" {
                    auth.create_account(&username, &password, level)
                } else {
                    auth.update_account(&username, &password, level)
                };
                outcome.map_err(|err| RpcError::new("AuthManagerError", err.to_string()))?;
                Ok(Value::Bool(true))
            }

            "core.remove_account" => {
                let username = string_arg(&args, 0, "a username")?;
                if username == context.username {
                    return Err(RpcError::new(
                        "AuthManagerError",
                        "You cannot delete your own account while logged in!",
                    ));
                }
                let mut auth = self.auth.lock().await;
                auth.remove_account(&username)
                    .map_err(|err| RpcError::new("AuthManagerError", err.to_string()))?;
                Ok(Value::Bool(true))
            }

            // ------------------------------------------------------- the rest
            other => {
                // The contract knows this method exists, so the gap is here
                // rather than in the caller. Saying which one is missing is
                // what makes the gap closable.
                tracing::warn!(method = other, "method not implemented yet");
                Err(RpcError::new(
                    "NotImplementedError",
                    format!("{other} is not implemented in this daemon yet"),
                ))
            }
        }
    }
}

/// Fetches a URL, for the two methods that need one.
///
/// Bounded on purpose: an unbounded download from a URL a client chose is a way
/// to fill the daemon's memory from the outside.
async fn fetch(url: &str) -> Result<Vec<u8>, RpcError> {
    const MAX: usize = 16 * 1024 * 1024;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent(concat!("redeluge/", "2.2.1"))
        .build()
        .map_err(|err| RpcError::invalid_argument(err.to_string()))?;

    let response = client
        .get(url)
        .send()
        .await
        .map_err(|err| RpcError::new("HTTPError", err.to_string()))?;

    if !response.status().is_success() {
        return Err(RpcError::new(
            "HTTPError",
            format!("{} returned {}", url, response.status()),
        ));
    }
    if let Some(length) = response.content_length() {
        if length as usize > MAX {
            return Err(RpcError::new(
                "HTTPError",
                format!("{url} is larger than {MAX} bytes"),
            ));
        }
    }

    let body = response
        .bytes()
        .await
        .map_err(|err| RpcError::new("HTTPError", err.to_string()))?;
    if body.len() > MAX {
        return Err(RpcError::new(
            "HTTPError",
            format!("{url} is larger than {MAX} bytes"),
        ));
    }
    Ok(body.to_vec())
}

fn filename_from_url(url: &str) -> String {
    url.rsplit('/')
        .next()
        .map(|name| name.split(['?', '#']).next().unwrap_or(name).to_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "downloaded.torrent".to_owned())
}

/// Directory entries beginning with a path, for the path chooser.
///
/// Directories only: the chooser is picking somewhere to save, and offering
/// files would be offering something that cannot be chosen.
fn glob_directory(pattern: &str) -> Vec<Value> {
    let path = std::path::Path::new(pattern);
    let (dir, prefix) = if pattern.ends_with('/') {
        (path.to_path_buf(), String::new())
    } else {
        (
            path.parent()
                .unwrap_or(std::path::Path::new("/"))
                .to_path_buf(),
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default(),
        )
    };

    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut out: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            // Hidden directories are not offered, matching the chooser's own
            // default. Someone who wants one can type it.
            if name.starts_with('.') || !name.starts_with(&prefix) {
                return None;
            }
            Some(entry.path().display().to_string())
        })
        .collect();
    out.sort();
    out.into_iter().map(Value::Str).collect()
}

/// Bytes free on the filesystem holding a path.
fn free_space(path: &str) -> i64 {
    #[cfg(unix)]
    {
        use std::ffi::CString;
        let Ok(c_path) = CString::new(path) else {
            return -1;
        };
        // SAFETY: statvfs writes into a struct we own and reads a NUL-terminated
        // path we own. Both outlive the call.
        unsafe {
            let mut stat: libc_statvfs = std::mem::zeroed();
            if statvfs(c_path.as_ptr(), &mut stat) != 0 {
                return -1;
            }
            (stat.f_bavail as i64).saturating_mul(stat.f_frsize as i64)
        }
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        -1
    }
}

/// Total size of a file, or of everything under a directory.
fn path_size(path: &str) -> i64 {
    let path = std::path::Path::new(path);
    let Ok(meta) = std::fs::metadata(path) else {
        return -1;
    };
    if meta.is_file() {
        return meta.len() as i64;
    }

    let mut total = 0i64;
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                total += meta.len() as i64;
            }
        }
    }
    total
}

#[cfg(unix)]
#[repr(C)]
#[allow(non_camel_case_types)]
struct libc_statvfs {
    f_bsize: u64,
    f_frsize: u64,
    f_blocks: u64,
    f_bfree: u64,
    f_bavail: u64,
    f_files: u64,
    f_ffree: u64,
    f_favail: u64,
    f_fsid: u64,
    f_flag: u64,
    f_namemax: u64,
    f_spare: [i32; 6],
}

#[cfg(unix)]
extern "C" {
    #[link_name = "statvfs64"]
    fn statvfs(path: *const std::ffi::c_char, buf: *mut libc_statvfs) -> i32;
}

/// Applies the per-torrent limits and flags an added torrent carries.
///
/// libtorrent takes the flags in the add request but not the limits, so
/// without this a torrent added with a speed cap ran uncapped until something
/// set the option a second time.
fn apply_limits(state: &mut crate::manager::SessionState, id: &str, options: &TorrentOptions) {
    let _ = state
        .session
        .set_max_connections(id, options.max_connections as i32);
    let _ = state
        .session
        .set_max_uploads(id, options.max_upload_slots as i32);
    let _ = state
        .session
        .set_download_limit(id, kib_to_bytes(options.max_download_speed));
    let _ = state
        .session
        .set_upload_limit(id, kib_to_bytes(options.max_upload_speed));

    let change = FlagChange::new()
        .set_to(flags::SEQUENTIAL_DOWNLOAD, options.sequential_download)
        .set_to(flags::SUPER_SEEDING, options.super_seeding);
    let _ = state.session.set_flags(id, change);

    if options.prioritize_first_last {
        apply_first_last_priority(state, id, true);
    }
}

/// Raises the priority of the pieces at each end of each file.
///
/// This is what "prioritise first and last pieces" means: a media player can
/// read a file's header and its index before the middle has arrived. The
/// setting was stored and never acted on, by `core.set_torrent_options` as
/// well as on add.
///
/// Only the boundary pieces are touched, and a file the user skipped is left
/// alone: writing a whole priority array would quietly un-skip it.
fn apply_first_last_priority(state: &mut crate::manager::SessionState, id: &str, on: bool) {
    let Ok(status) = state.session.torrent_status(id) else {
        return;
    };
    if status.piece_length <= 0 || status.num_pieces <= 0 {
        // No metadata yet. A magnet gets this applied when the torrent is next
        // given options; there is nothing to prioritise before then.
        return;
    }

    let Ok(files) = state.session.files(id) else {
        return;
    };
    let Ok(priorities) = state.session.file_priorities(id) else {
        return;
    };
    let Ok(mut pieces) = state.session.piece_priorities(id) else {
        return;
    };

    let length = i64::from(status.piece_length);
    let last_piece = pieces.len().saturating_sub(1);

    for file in &files {
        let own = priorities.get(file.index as usize).copied().unwrap_or(4);
        if own == 0 || file.size <= 0 {
            continue;
        }
        let first = (file.offset / length) as usize;
        let last = ((file.offset + file.size - 1) / length) as usize;
        for piece in [first.min(last_piece), last.min(last_piece)] {
            if let Some(slot) = pieces.get_mut(piece) {
                *slot = if on { 7 } else { own };
            }
        }
    }

    let _ = state.session.prioritize_pieces(id, &pieces);
}

/// Keeps the user's own copy of a `.torrent`, if they asked for one.
///
/// `copy_torrent_file` and `torrentfiles_location` are Deluge settings that
/// this daemon stored and never acted on. The copy is named after the file it
/// was added from where there is one, and after the torrent otherwise, which
/// is what a magnet gives you.
fn copy_torrent_file(directory: &str, filename: &str, id: &str, bytes: &[u8]) {
    if directory.is_empty() {
        return;
    }
    let name = if filename.is_empty() {
        format!("{id}.torrent")
    } else {
        // Only the last component, and nothing that climbs out of the
        // directory: the name came from a client.
        let base = filename.rsplit(['/', '\\']).next().unwrap_or(filename);
        let base = base.trim_matches('.');
        if base.is_empty() {
            format!("{id}.torrent")
        } else {
            base.to_owned()
        }
    };

    let path = std::path::Path::new(directory).join(name);
    if let Err(err) = std::fs::create_dir_all(directory).and_then(|()| std::fs::write(&path, bytes))
    {
        tracing::warn!(torrent = %id, path = %path.display(), error = %err,
            "could not keep a copy of the torrent file");
    }
}

/// The options a torrent is added with: the configured defaults, with
/// whatever the client sent on top.
///
/// `defaults` used to be `TorrentOptions::default()`, a constant, so every
/// preference under "Add Torrent Options", the per-torrent bandwidth limits
/// and the seeding rules were stored and never read. A client that sends a key
/// still wins, which is Deluge's order.
fn options_from(value: Option<&Value>, defaults: TorrentOptions) -> TorrentOptions {
    let mut options = defaults;
    let Some(Value::Dict(entries)) = value else {
        return options;
    };

    for (key, value) in entries {
        let Some(key) = key.as_str() else { continue };
        match key {
            "download_location" | "save_path" => {
                options.save_path = value.as_str().map(str::to_owned)
            }
            "name" => options.name = value.as_str().map(str::to_owned),
            "add_paused" => options.paused = value.as_bool().unwrap_or(false),
            "auto_managed" => options.auto_managed = value.as_bool().unwrap_or(true),
            "sequential_download" => options.sequential_download = value.as_bool().unwrap_or(false),
            "pre_allocate_storage" => {
                options.storage_mode = if value.as_bool().unwrap_or(false) {
                    "allocate".to_owned()
                } else {
                    "sparse".to_owned()
                };
            }
            "prioritize_first_last_pieces" => {
                options.prioritize_first_last = value.as_bool().unwrap_or(false)
            }
            "max_connections" => options.max_connections = value.as_i64().unwrap_or(-1),
            "max_upload_slots" => options.max_upload_slots = value.as_i64().unwrap_or(-1),
            "move_completed" => options.move_completed = value.as_bool().unwrap_or(false),
            "move_completed_path" => {
                options.move_completed_path = value.as_str().map(str::to_owned)
            }
            "label" => options.label = normalise_label(value.as_str().unwrap_or_default()),
            "owner" => options.owner = value.as_str().unwrap_or_default().to_owned(),
            "shared" => options.shared = value.as_bool().unwrap_or(false),
            "super_seeding" => options.super_seeding = value.as_bool().unwrap_or(false),
            "stop_at_ratio" => options.stop_at_ratio = value.as_bool().unwrap_or(false),
            "remove_at_ratio" => options.remove_at_ratio = value.as_bool().unwrap_or(false),
            "file_priorities" => {
                if let Value::List(items) = value {
                    options.file_priorities = items
                        .iter()
                        .filter_map(|item| item.as_i64())
                        .map(|priority| priority.clamp(0, 7) as u8)
                        .collect();
                }
            }
            _ => {}
        }
    }
    options
}

fn json_to_value(json: &serde_json::Value) -> Value {
    match json {
        serde_json::Value::Null => Value::None,
        serde_json::Value::Bool(value) => Value::Bool(*value),
        serde_json::Value::Number(number) => match number.as_i64() {
            Some(integer) => Value::Int(integer),
            None => Value::Float64(number.as_f64().unwrap_or(0.0)),
        },
        serde_json::Value::String(text) => Value::Str(text.clone()),
        serde_json::Value::Array(items) => Value::List(items.iter().map(json_to_value).collect()),
        serde_json::Value::Object(map) => json_map_to_value(map),
    }
}

fn json_map_to_value(map: &serde_json::Map<String, serde_json::Value>) -> Value {
    Value::Dict(
        map.iter()
            .map(|(key, value)| (Value::Str(key.clone()), json_to_value(value)))
            .collect(),
    )
}

fn value_to_json(value: &Value) -> serde_json::Value {
    match value {
        Value::None => serde_json::Value::Null,
        Value::Bool(value) => serde_json::Value::Bool(*value),
        Value::Int(value) => serde_json::Value::Number((*value).into()),
        Value::BigInt(text) => serde_json::Value::String(text.clone()),
        Value::Float32(value) => serde_json::Number::from_f64(f64::from(*value))
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Value::Float64(value) => serde_json::Number::from_f64(*value)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        Value::Str(text) => serde_json::Value::String(text.clone()),
        Value::Bytes(raw) => serde_json::Value::String(String::from_utf8_lossy(raw).into_owned()),
        Value::List(items) => serde_json::Value::Array(items.iter().map(value_to_json).collect()),
        Value::Dict(entries) => serde_json::Value::Object(
            entries
                .iter()
                .filter_map(|(key, value)| {
                    key.as_str()
                        .map(|key| (key.to_owned(), value_to_json(value)))
                })
                .collect(),
        ),
    }
}

impl Core {
    /// Adds a magnet, waits for its metadata, then removes it again.
    ///
    /// What the add dialog uses to show the file list before the user commits.
    /// The torrent is added paused so it never starts downloading content, and
    /// it is always removed, including when the wait times out.
    async fn prefetch_metadata(&self, uri: &str, seconds: u64) -> Result<Value, RpcError> {
        let request = AddTorrent::from_magnet(uri.to_owned(), "/tmp".to_owned()).paused(true);
        let id = self
            .manager
            .with(move |state| state.session.add_torrent(&request))
            .await
            .map_err(|err| RpcError::invalid_argument(err.to_string()))?
            .map_err(|err| RpcError::invalid_argument(err.to_string()))?;

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(seconds);
        let mut metadata = None;

        while std::time::Instant::now() < deadline {
            let wanted = id.clone();
            let ready = self
                .manager
                .with(move |state| {
                    let status = state.session.torrent_status(&wanted).ok()?;
                    status
                        .has_metadata
                        .then(|| state.session.torrent_file(&wanted).ok())
                        .flatten()
                })
                .await
                .ok()
                .flatten();

            if let Some(bytes) = ready {
                metadata = Some(bytes);
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }

        // Removed either way: leaving it behind would put a torrent nobody
        // asked for in the session and in the saved state.
        let cleanup = id.clone();
        let _ = self
            .manager
            .with(move |state| {
                let _ = state.session.remove_torrent(&cleanup, false);
            })
            .await;

        Ok(Value::List(vec![
            Value::Str(id),
            metadata.map(Value::Bytes).unwrap_or(Value::None),
        ]))
    }

    /// One torrent's status. `peers` costs a second call into libtorrent and a
    /// country lookup each, so it is only paid for when a client asks.
    async fn status_of_with(
        &self,
        id: &str,
        peers: bool,
        files: bool,
    ) -> Result<BTreeMap<String, Value>, RpcError> {
        let grace = self.idle_grace().await;
        let wanted = id.to_owned();
        let status = self
            .manager
            .with(move |state| {
                let torrent = state.torrents.get(&wanted)?.clone();
                let status = state.session.torrent_status(&wanted).ok()?;
                let trackers = state.session.trackers(&wanted).unwrap_or_default();
                let file_list = if files {
                    Some((
                        state.session.files(&wanted).unwrap_or_default(),
                        state.session.file_progress(&wanted).unwrap_or_default(),
                        state.session.file_priorities(&wanted).unwrap_or_default(),
                    ))
                } else {
                    None
                };
                let peers = if peers {
                    state
                        .session
                        .peers(&wanted)
                        .unwrap_or_default()
                        .into_iter()
                        .map(|peer| {
                            let country = state.country_of(&peer.ip);
                            (peer, country)
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                let idle_since = state.idle_since.get(&wanted).copied().unwrap_or_default();
                let mut out = torrent.status_with_peers(
                    &status,
                    state.session_paused,
                    &trackers,
                    &peers,
                    idle_since,
                    grace,
                );
                if let Some((entries, progress, priorities)) = file_list {
                    crate::torrent::put_files(&mut out, &entries, &progress, &priorities);
                }
                Some(out)
            })
            .await
            .map_err(|err| RpcError::invalid_argument(err.to_string()))?;

        status.ok_or_else(|| RpcError::new("InvalidTorrentError", format!("no such torrent: {id}")))
    }

    async fn all_status(
        &self,
        filter: Option<Value>,
        keys: Option<Vec<String>>,
    ) -> Result<Value, RpcError> {
        let grace = self.idle_grace().await;
        let all = self
            .manager
            .with(move |state| {
                let session_paused = state.session_paused;
                let mut out: Vec<(String, BTreeMap<String, Value>)> = Vec::new();
                for status in state.session.all_torrent_status() {
                    let Some(torrent) = state.torrents.get(&status.info_hash) else {
                        continue;
                    };
                    let trackers = state
                        .session
                        .trackers(&status.info_hash)
                        .unwrap_or_default();
                    let idle_since = state
                        .idle_since
                        .get(&status.info_hash)
                        .copied()
                        .unwrap_or_default();
                    out.push((
                        status.info_hash.clone(),
                        torrent.status_with_peers(
                            &status,
                            session_paused,
                            &trackers,
                            &[],
                            idle_since,
                            grace,
                        ),
                    ));
                }
                out
            })
            .await
            .map_err(|err| RpcError::invalid_argument(err.to_string()))?;

        let wanted = filter_pairs(filter.as_ref());
        Ok(Value::Dict(
            all.into_iter()
                .filter(|(_, status)| matches_filter(status, &wanted))
                .map(|(id, status)| (Value::Str(id), Value::Dict(filtered(status, &keys))))
                .collect(),
        ))
    }

    async fn session_status(&self, wanted: Vec<String>) -> Result<Value, RpcError> {
        let (counters, rates) = self
            .manager
            .with(|state| {
                state.session.post_session_stats();
                let mut download = 0i64;
                let mut upload = 0i64;
                for status in state.session.all_torrent_status() {
                    download += i64::from(status.download_payload_rate);
                    upload += i64::from(status.upload_payload_rate);
                }
                (state.counters.clone(), (download, upload))
            })
            .await
            .map_err(|err| RpcError::invalid_argument(err.to_string()))?;

        let names = redeluge_libtorrent::Session::stat_names();
        let mut out: BTreeMap<String, Value> = BTreeMap::new();
        for (index, name) in names.iter().enumerate() {
            let value = counters.get(index).copied().unwrap_or(0);
            out.insert(name.clone(), Value::Int(value));
        }

        // These two are the sum over torrents rather than a counter, and every
        // client reads them.
        out.insert("payload_download_rate".into(), Value::Int(rates.0));
        out.insert("payload_upload_rate".into(), Value::Int(rates.1));
        out.entry("download_rate".into())
            .or_insert(Value::Int(rates.0));
        out.entry("upload_rate".into())
            .or_insert(Value::Int(rates.1));

        let entries: Vec<(Value, Value)> = if wanted.is_empty() {
            out.into_iter()
                .map(|(key, value)| (Value::Str(key), value))
                .collect()
        } else {
            wanted
                .into_iter()
                .map(|key| {
                    let value = out.get(&key).cloned().unwrap_or(Value::Int(0));
                    (Value::Str(key), value)
                })
                .collect()
        };
        Ok(Value::Dict(entries))
    }

    /// The counts every client draws its sidebar from.
    async fn filter_tree(&self) -> Result<Value, RpcError> {
        let statuses = self
            .manager
            .with(|state| {
                let session_paused = state.session_paused;
                state
                    .session
                    .all_torrent_status()
                    .into_iter()
                    .filter_map(|status| {
                        let torrent = state.torrents.get(&status.info_hash)?;
                        // The trackers as well as the announced one, because
                        // the rule falls back to the first when nothing has
                        // been announced to yet, and the status applies the
                        // same rule.
                        let trackers = state
                            .session
                            .trackers(&status.info_hash)
                            .unwrap_or_default();
                        Some((
                            torrent.state(&status, session_paused),
                            crate::torrent::current_tracker(&status.current_tracker, &trackers),
                            torrent.options.owner.clone(),
                            torrent.options.label.clone(),
                            status.download_payload_rate > 0 || status.upload_payload_rate > 0,
                        ))
                    })
                    .collect::<Vec<_>>()
            })
            .await
            .map_err(|err| RpcError::invalid_argument(err.to_string()))?;

        let total = statuses.len() as i64;
        let mut by_state: BTreeMap<TorrentState, i64> = BTreeMap::new();
        let mut by_tracker: BTreeMap<String, i64> = BTreeMap::new();
        let mut by_owner: BTreeMap<String, i64> = BTreeMap::new();
        let mut by_label: BTreeMap<String, i64> = BTreeMap::new();
        let mut active = 0i64;

        for (state, tracker, owner, label, transferring) in statuses {
            *by_state.entry(state).or_insert(0) += 1;
            // Active means moving bytes, not "not paused". A seeding torrent
            // nobody is downloading from is idle, and counting it here put
            // every finished torrent in a category meant for the ones worth
            // watching. This is Deluge's own rule, and the filter below uses
            // the same one, so the count and the list agree.
            if transferring {
                active += 1;
            }
            // The same function the status uses, and that is the whole point.
            // There were two, and they disagreed: this one kept the subdomain
            // and the status dropped it, so the sidebar listed
            // `tracker.example.com` while every torrent was recorded under
            // `example.com`, and clicking the row filtered to nothing. A
            // filter value has to be the value it is compared against.
            let host = crate::torrent::tracker_host(&tracker);
            *by_tracker.entry(host).or_insert(0) += 1;
            *by_owner.entry(owner).or_insert(0) += 1;
            *by_label.entry(label).or_insert(0) += 1;
        }

        let pair = |label: &str, count: i64| {
            Value::List(vec![Value::Str(label.to_owned()), Value::Int(count)])
        };

        let mut states = vec![pair("All", total), pair("Active", active)];
        for state in TorrentState::ALL {
            states.push(pair(
                state.as_str(),
                by_state.get(&state).copied().unwrap_or(0),
            ));
        }

        let mut trackers = vec![pair("All", total)];
        trackers.extend(
            by_tracker
                .into_iter()
                .map(|(host, count)| pair(&host, count)),
        );

        let owners: Vec<Value> = by_owner
            .into_iter()
            .map(|(owner, count)| pair(&owner, count))
            .collect();

        // The Label plugin contributed this category in Deluge, which is why
        // clients already know how to draw it. An unlabelled torrent counts
        // under the empty string, the same key it filters on.
        let mut labels = vec![pair("All", total)];
        labels.extend(by_label.into_iter().map(|(name, count)| pair(&name, count)));

        Ok(Value::Dict(vec![
            (Value::Str("state".into()), Value::List(states)),
            (Value::Str("tracker_host".into()), Value::List(trackers)),
            (Value::Str("owner".into()), Value::List(owners)),
            (Value::Str("label".into()), Value::List(labels)),
        ]))
    }

    async fn apply_torrent_options(
        &self,
        ids: Vec<String>,
        options: Value,
    ) -> Result<Value, RpcError> {
        let Value::Dict(entries) = options else {
            return Err(RpcError::invalid_argument("a dictionary is required"));
        };

        self.manager
            .with(move |state| {
                for id in &ids {
                    let Some(torrent) = state.torrents.get_mut(id) else {
                        continue;
                    };
                    let mut change = FlagChange::new();
                    let mut first_last = None;

                    for (key, value) in &entries {
                        let Some(key) = key.as_str() else { continue };
                        match key {
                            "max_connections" => {
                                let limit = value.as_i64().unwrap_or(-1);
                                torrent.options.max_connections = limit;
                                let _ = state.session.set_max_connections(id, limit as i32);
                            }
                            "max_upload_slots" => {
                                let limit = value.as_i64().unwrap_or(-1);
                                torrent.options.max_upload_slots = limit;
                                let _ = state.session.set_max_uploads(id, limit as i32);
                            }
                            "max_download_speed" => {
                                let limit = as_f64(value).unwrap_or(-1.0);
                                torrent.options.max_download_speed = limit;
                                let _ = state.session.set_download_limit(id, kib_to_bytes(limit));
                            }
                            "max_upload_speed" => {
                                let limit = as_f64(value).unwrap_or(-1.0);
                                torrent.options.max_upload_speed = limit;
                                let _ = state.session.set_upload_limit(id, kib_to_bytes(limit));
                            }
                            "sequential_download" => {
                                let on = value.as_bool().unwrap_or(false);
                                torrent.options.sequential_download = on;
                                change = change.set_to(flags::SEQUENTIAL_DOWNLOAD, on);
                            }
                            "super_seeding" => {
                                let on = value.as_bool().unwrap_or(false);
                                torrent.options.super_seeding = on;
                                change = change.set_to(flags::SUPER_SEEDING, on);
                            }
                            "auto_managed" => {
                                let on = value.as_bool().unwrap_or(true);
                                torrent.options.auto_managed = on;
                                change = change.set_to(flags::AUTO_MANAGED, on);
                            }
                            "file_priorities" => {
                                if let Value::List(items) = value {
                                    let priorities: Vec<u8> = items
                                        .iter()
                                        .filter_map(|item| item.as_i64())
                                        .map(|p| p.clamp(0, 7) as u8)
                                        .collect();
                                    torrent.options.file_priorities = priorities.clone();
                                    let _ = state.session.prioritize_files(id, &priorities);
                                }
                            }
                            "stop_at_ratio" => {
                                torrent.options.stop_at_ratio = value.as_bool().unwrap_or(false)
                            }
                            "stop_ratio" => {
                                torrent.options.stop_ratio = as_f64(value).unwrap_or(2.0)
                            }
                            "remove_at_ratio" => {
                                torrent.options.remove_at_ratio = value.as_bool().unwrap_or(false)
                            }
                            "move_completed" => {
                                torrent.options.move_completed = value.as_bool().unwrap_or(false)
                            }
                            "move_completed_path" => {
                                torrent.options.move_completed_path =
                                    value.as_str().map(str::to_owned)
                            }
                            "label" => {
                                torrent.options.label =
                                    normalise_label(value.as_str().unwrap_or_default())
                            }
                            "owner" => {
                                torrent.options.owner =
                                    value.as_str().unwrap_or_default().to_owned()
                            }
                            "prioritize_first_last_pieces" => {
                                let on = value.as_bool().unwrap_or(false);
                                torrent.options.prioritize_first_last = on;
                                // Applied after this loop: it reads the whole
                                // session, and the torrent is borrowed here.
                                first_last = Some(on);
                            }
                            "shared" => torrent.options.shared = value.as_bool().unwrap_or(false),
                            "name" => torrent.options.name = value.as_str().map(str::to_owned),
                            _ => {}
                        }
                    }

                    if !change.is_empty() {
                        let _ = state.session.set_flags(id, change);
                    }
                    if let Some(on) = first_last {
                        apply_first_last_priority(state, id, on);
                    }
                }
                state.mark_dirty();
            })
            .await
            .map_err(|err| RpcError::invalid_argument(err.to_string()))?;
        Ok(Value::None)
    }
}

/// A label as the Label plugin would have stored it.
///
/// Lower case, and only the characters its own validator allowed. Deluge
/// refused anything else outright; refusing here would mean a torrent silently
/// keeping its old label because one character was wrong, so the value is
/// cleaned instead and what survives is what a client sees back.
pub fn normalise_label(raw: &str) -> String {
    raw.trim()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        .collect()
}

fn as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Int(number) => Some(*number as f64),
        Value::Float32(number) => Some(f64::from(*number)),
        Value::Float64(number) => Some(*number),
        _ => None,
    }
}

/// Deluge's speed settings are in KiB/s and libtorrent's in bytes per second.
fn kib_to_bytes(kib: f64) -> i32 {
    if kib < 0.0 {
        return -1;
    }
    (kib * 1024.0).min(f64::from(i32::MAX)) as i32
}

/// The filter a client sent, as key/value pairs.
fn filter_pairs(filter: Option<&Value>) -> Vec<(String, Vec<String>)> {
    let Some(Value::Dict(entries)) = filter else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|(key, value)| {
            let key = key.as_str()?.to_owned();
            let values = match value {
                Value::List(items) => items
                    .iter()
                    .filter_map(|item| item.as_str().map(str::to_owned))
                    .collect(),
                other => vec![other.as_str()?.to_owned()],
            };
            Some((key, values))
        })
        .collect()
}

fn matches_filter(status: &BTreeMap<String, Value>, filter: &[(String, Vec<String>)]) -> bool {
    filter.iter().all(|(key, wanted)| {
        // "All" is how every client spells "no filter on this field".
        if wanted.iter().any(|value| value == "All") {
            return true;
        }
        // "Active" is not a state, it is a question about right now: is this
        // torrent moving any bytes? A seeding torrent with no peers is not,
        // however finished it is. Deluge asks it this way too.
        if key == "state" && wanted.iter().any(|value| value == "Active") {
            let rate = |name: &str| status.get(name).and_then(Value::as_i64).unwrap_or_default();
            return rate("download_payload_rate") > 0 || rate("upload_payload_rate") > 0;
        }
        // The quick search. Deluge looks in more than the name, so that
        // "error" or the name of a tracker finds what you meant, and every
        // term has to match.
        if key == "keyword" {
            return wanted
                .iter()
                .flat_map(|value| value.split(','))
                .map(str::trim)
                .filter(|term| !term.is_empty())
                .all(|term| matches_keyword(status, &term.to_lowercase()));
        }
        // `name` is the other free-text filter, and it is a substring rather
        // than an exact match: nothing would ever match a whole torrent name
        // typed by hand.
        if key == "name" {
            let Some(name) = status.get("name").and_then(Value::as_str) else {
                return false;
            };
            let name = name.to_lowercase();
            return wanted
                .iter()
                .any(|value| name.contains(&value.to_lowercase()));
        }
        match status.get(key).and_then(Value::as_str) {
            Some(actual) => wanted.iter().any(|value| value == actual),
            None => false,
        }
    })
}

/// One search term against the fields Deluge searched.
///
/// Name, state, tracker, tracker message, label and the infohash. The file
/// list is the one field upstream searched that this does not: it is not in
/// the status, and fetching every torrent's files to answer a keystroke would
/// be a great deal of work for a search box.
fn matches_keyword(status: &BTreeMap<String, Value>, term: &str) -> bool {
    const SEARCHED: &[&str] = &[
        "name",
        "state",
        "tracker_host",
        "tracker",
        "tracker_status",
        "label",
        "hash",
    ];

    SEARCHED.iter().any(|key| {
        status
            .get(*key)
            .and_then(Value::as_str)
            .is_some_and(|value| value.to_lowercase().contains(term))
    })
}

#[cfg(test)]
mod filter_tests {
    use super::*;

    fn status(state: &str, down: i64, up: i64) -> BTreeMap<String, Value> {
        let mut out = BTreeMap::new();
        out.insert("state".to_owned(), Value::Str(state.to_owned()));
        out.insert("download_payload_rate".to_owned(), Value::Int(down));
        out.insert("upload_payload_rate".to_owned(), Value::Int(up));
        out.insert("name".to_owned(), Value::Str("a torrent".to_owned()));
        out
    }

    fn by_state(value: &str) -> Vec<(String, Vec<String>)> {
        vec![("state".to_owned(), vec![value.to_owned()])]
    }

    #[test]
    fn active_means_moving_bytes_rather_than_being_unpaused() {
        // A finished torrent that nobody is downloading from is not active,
        // however much of it is seeded. Counting it put every completed
        // torrent in the one category meant for what is worth watching.
        assert!(!matches_filter(
            &status("Seeding", 0, 0),
            &by_state("Active")
        ));
        assert!(matches_filter(
            &status("Seeding", 0, 4096),
            &by_state("Active")
        ));
        assert!(matches_filter(
            &status("Downloading", 8192, 0),
            &by_state("Active")
        ));
        // Paused cannot be active whatever the rates say, and they will be
        // zero, but the rule does not need a special case for it.
        assert!(!matches_filter(
            &status("Paused", 0, 0),
            &by_state("Active")
        ));
    }

    #[test]
    fn a_state_filter_is_still_the_state() {
        assert!(matches_filter(
            &status("Seeding", 0, 0),
            &by_state("Seeding")
        ));
        assert!(!matches_filter(
            &status("Seeding", 0, 0),
            &by_state("Downloading")
        ));
    }

    #[test]
    fn all_means_no_filter_on_that_field() {
        assert!(matches_filter(&status("Paused", 0, 0), &by_state("All")));
    }
}
