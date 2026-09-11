// SPDX-License-Identifier: GPL-3.0-or-later
//! The daemon's account file.
//!
//! One account per line, `username:password:level`. The file is read on start
//! and re-read when its modification time changes, so an operator can add an
//! account without restarting, which is what the Python daemon does.
//!
//! # What changes from the Python implementation
//!
//! `deluge/core/authmanager.py` compares passwords in plaintext for any hash it
//! does not recognise, using `==`. Here every comparison is constant time, and
//! a plaintext entry is rewritten as scrypt the first time it is used, so an
//! old file converges on something safe by being used.
//!
//! # Why `localclient` is the exception
//!
//! That account exists so a tool on the same machine can log in without anyone
//! typing anything: it reads the password straight out of this file. Hashing it
//! would make the account unusable, which is not a hypothetical, it is how the
//! first run of this daemon locked itself out.
//!
//! So `localclient` keeps a plaintext password and is never upgraded. What
//! makes that acceptable is the rest of the arrangement: the file is 0600, the
//! password is twenty random bytes nobody chose, and the account is only
//! reachable over loopback unless `allow_remote` is turned on. Anyone who can
//! read the file can already read everything else the daemon owns.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use ring::rand::SecureRandom;

/// What a caller is allowed to do. The numbers are part of the wire contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AuthLevel {
    None,
    ReadOnly,
    Normal,
    Admin,
}

impl AuthLevel {
    pub fn as_i64(self) -> i64 {
        match self {
            Self::None => 0,
            Self::ReadOnly => 1,
            Self::Normal => 5,
            Self::Admin => 10,
        }
    }

    pub fn from_i64(value: i64) -> Option<Self> {
        match value {
            0 => Some(Self::None),
            1 => Some(Self::ReadOnly),
            5 => Some(Self::Normal),
            10 => Some(Self::Admin),
            _ => None,
        }
    }

    /// The names the RPC uses, which clients display and send back.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "NONE",
            Self::ReadOnly => "READONLY",
            Self::Normal => "NORMAL",
            Self::Admin => "ADMIN",
        }
    }

    /// Parses the name the wire uses. Not `FromStr`: a bad name here is a
    /// missing case rather than a failure worth an error type.
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "NONE" => Some(Self::None),
            "READONLY" => Some(Self::ReadOnly),
            "DEFAULT" | "NORMAL" => Some(Self::Normal),
            "ADMIN" => Some(Self::Admin),
            _ => None,
        }
    }
}

/// The level an account gets when its line does not say.
pub const DEFAULT_LEVEL: AuthLevel = AuthLevel::Normal;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no such account: {0}")]
    UnknownAccount(String),
    #[error("the password does not match")]
    BadPassword,
    #[error("an account named {0} already exists")]
    AccountExists(String),
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
    #[error("could not generate a password: {0}")]
    Random(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;

const SCRYPT_LOG_N: u8 = 15;
const SCRYPT_R: u32 = 8;
const SCRYPT_P: u32 = 1;
const SCRYPT_LEN: usize = 64;
const SALT_CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

#[derive(Debug, Clone)]
struct Account {
    /// The stored password, in whatever form the file held it.
    password: StoredPassword,
    level: AuthLevel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StoredPassword {
    /// `$scrypt$n$r$p$salt$hex`, the format `deluge/security.py` writes.
    Scrypt(String),
    /// A password the file holds in the clear, from an older install.
    Plaintext(String),
}

impl StoredPassword {
    fn parse(text: &str) -> Self {
        if text.starts_with("$scrypt$") {
            Self::Scrypt(text.to_owned())
        } else {
            Self::Plaintext(text.to_owned())
        }
    }

    fn as_str(&self) -> &str {
        match self {
            Self::Scrypt(text) | Self::Plaintext(text) => text,
        }
    }

    fn verify(&self, candidate: &str) -> bool {
        match self {
            Self::Scrypt(encoded) => verify_scrypt(encoded, candidate),
            Self::Plaintext(stored) => constant_time_eq(stored.as_bytes(), candidate.as_bytes()),
        }
    }

    fn needs_upgrade(&self, username: &str) -> bool {
        // localclient is deliberately left in plaintext; see the module notes.
        username != "localclient" && matches!(self, Self::Plaintext(_))
    }
}

/// The accounts the daemon knows about.
#[derive(Debug)]
pub struct AuthManager {
    path: PathBuf,
    accounts: HashMap<String, Account>,
    /// Last modification time seen, so an unchanged file is not re-parsed.
    seen: Option<SystemTime>,
}

impl AuthManager {
    /// Opens the auth file, creating it with a `localclient` account when it is
    /// missing. That account is what the daemon's own tools log in with.
    pub fn open(config_dir: &Path) -> Result<Self> {
        let path = config_dir.join("auth");
        let mut manager = Self {
            path,
            accounts: HashMap::new(),
            seen: None,
        };

        if !manager.path.exists() {
            let password = random_password()?;
            manager.accounts.insert(
                "localclient".to_owned(),
                Account {
                    // Plaintext on purpose: a local tool reads it from here.
                    password: StoredPassword::Plaintext(password),
                    level: AuthLevel::Admin,
                },
            );
            manager.save()?;
            tracing::info!(path = %manager.path.display(), "created an auth file");
        } else {
            manager.reload()?;
        }
        Ok(manager)
    }

    /// Re-reads the file when it has changed on disk.
    ///
    /// Returns whether anything was read. The daemon calls this on a timer so
    /// an operator can add an account without a restart.
    pub fn reload_if_changed(&mut self) -> Result<bool> {
        let modified = std::fs::metadata(&self.path)
            .and_then(|meta| meta.modified())
            .ok();
        if modified.is_some() && modified == self.seen {
            return Ok(false);
        }
        self.reload()?;
        Ok(true)
    }

    fn reload(&mut self) -> Result<()> {
        let text = read_with_backup(&self.path)?;
        self.seen = std::fs::metadata(&self.path)
            .and_then(|meta| meta.modified())
            .ok();

        let mut accounts = HashMap::new();
        for (number, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            let fields: Vec<&str> = line.split(':').collect();
            let (username, password, level) = match fields.as_slice() {
                [username, password] => {
                    // An old file with no level. localclient is the daemon's
                    // own account and has always been an administrator.
                    let level = if username.trim() == "localclient" {
                        AuthLevel::Admin
                    } else {
                        DEFAULT_LEVEL
                    };
                    (*username, *password, level)
                }
                [username, password, level] => {
                    let parsed = level
                        .trim()
                        .parse::<i64>()
                        .ok()
                        .and_then(AuthLevel::from_i64)
                        .or_else(|| AuthLevel::from_name(level.trim()));
                    match parsed {
                        Some(level) => (*username, *password, level),
                        None => {
                            tracing::warn!(
                                line = number + 1,
                                "unreadable auth level, using the default"
                            );
                            (*username, *password, DEFAULT_LEVEL)
                        }
                    }
                }
                _ => {
                    // A password containing a colon produces this. Skipping the
                    // line is better than guessing which colon separates what.
                    tracing::error!(line = number + 1, "malformed auth entry, skipped");
                    continue;
                }
            };

            accounts.insert(
                username.trim().to_owned(),
                Account {
                    password: StoredPassword::parse(password.trim()),
                    level,
                },
            );
        }

        self.accounts = accounts;
        Ok(())
    }

    /// Checks a username and password, returning the level it grants.
    ///
    /// A plaintext password that verifies is rewritten as scrypt, so a file
    /// from an older install converges on something safe by being used.
    pub fn authorize(&mut self, username: &str, password: &str) -> Result<AuthLevel> {
        let Some(account) = self.accounts.get(username) else {
            // The file may have gained the account since it was last read.
            self.reload_if_changed()?;
            let account = self
                .accounts
                .get(username)
                .ok_or_else(|| Error::UnknownAccount(username.to_owned()))?;
            return Self::check(account, password);
        };

        let level = Self::check(account, password)?;

        if account.password.needs_upgrade(username) {
            if let Ok(upgraded) = hash(password) {
                if let Some(entry) = self.accounts.get_mut(username) {
                    entry.password = upgraded;
                }
                let _ = self.save();
                tracing::info!(username, "stored password upgraded to scrypt");
            }
        }
        Ok(level)
    }

    fn check(account: &Account, password: &str) -> Result<AuthLevel> {
        if account.password.verify(password) {
            Ok(account.level)
        } else {
            Err(Error::BadPassword)
        }
    }

    pub fn has_account(&self, username: &str) -> bool {
        self.accounts.contains_key(username)
    }

    /// Username and level of every account, for `core.get_known_accounts`.
    pub fn accounts(&self) -> Vec<(String, AuthLevel)> {
        let mut out: Vec<(String, AuthLevel)> = self
            .accounts
            .iter()
            .map(|(name, account)| (name.clone(), account.level))
            .collect();
        out.sort_by(|left, right| left.0.cmp(&right.0));
        out
    }

    pub fn create_account(
        &mut self,
        username: &str,
        password: &str,
        level: AuthLevel,
    ) -> Result<()> {
        if username.is_empty() {
            return Err(Error::UnknownAccount(String::new()));
        }
        if self.accounts.contains_key(username) {
            return Err(Error::AccountExists(username.to_owned()));
        }
        self.accounts.insert(
            username.to_owned(),
            Account {
                password: hash(password)?,
                level,
            },
        );
        self.save()
    }

    pub fn update_account(
        &mut self,
        username: &str,
        password: &str,
        level: AuthLevel,
    ) -> Result<()> {
        if !self.accounts.contains_key(username) {
            return Err(Error::UnknownAccount(username.to_owned()));
        }
        self.accounts.insert(
            username.to_owned(),
            Account {
                password: hash(password)?,
                level,
            },
        );
        self.save()
    }

    pub fn remove_account(&mut self, username: &str) -> Result<()> {
        if self.accounts.remove(username).is_none() {
            return Err(Error::UnknownAccount(username.to_owned()));
        }
        self.save()
    }

    /// The credentials a local client logs in with.
    pub fn localclient(&self) -> Option<(&str, &str)> {
        self.accounts
            .get_key_value("localclient")
            .map(|(name, account)| (name.as_str(), account.password.as_str()))
    }

    fn save(&mut self) -> Result<()> {
        let mut names: Vec<&String> = self.accounts.keys().collect();
        names.sort();

        let mut text = String::from(
            "# username:password:level\n\
             # Written by redeluged. Passwords are scrypt hashes.\n",
        );
        for name in names {
            let account = &self.accounts[name];
            text.push_str(&format!(
                "{name}:{}:{}\n",
                account.password.as_str(),
                account.level.as_i64()
            ));
        }

        write_atomically(&self.path, text.as_bytes())?;
        self.seen = std::fs::metadata(&self.path)
            .and_then(|meta| meta.modified())
            .ok();
        Ok(())
    }
}

fn read_with_backup(path: &Path) -> Result<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(text),
        Err(primary) => {
            // The Python daemon falls back to the backup, and so does this: a
            // truncated write must not lock everyone out.
            match std::fs::read_to_string(path.with_extension("bak")) {
                Ok(text) => {
                    tracing::warn!(path = %path.display(), "reading the auth backup");
                    Ok(text)
                }
                Err(_) => Err(Error::Read {
                    path: path.to_path_buf(),
                    source: primary,
                }),
            }
        }
    }
}

fn write_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    let temporary = path.with_extension("tmp");
    std::fs::write(&temporary, bytes).map_err(|source| Error::Write {
        path: temporary.clone(),
        source,
    })?;

    // The auth file holds the only copy of every password, so the previous one
    // is kept until the new one is in place.
    if path.exists() {
        let _ = std::fs::rename(path, path.with_extension("bak"));
    }
    std::fs::rename(&temporary, path).map_err(|source| Error::Write {
        path: path.to_path_buf(),
        source,
    })?;

    // Nobody but the owner has any business reading this.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn hash(password: &str) -> Result<StoredPassword> {
    let salt = random_string(16)?;
    let params = scrypt::Params::new(SCRYPT_LOG_N, SCRYPT_R, SCRYPT_P, SCRYPT_LEN)
        .map_err(|_| Error::Random("scrypt parameters"))?;

    let mut out = vec![0u8; SCRYPT_LEN];
    scrypt::scrypt(password.as_bytes(), salt.as_bytes(), &params, &mut out)
        .map_err(|_| Error::Random("scrypt"))?;

    Ok(StoredPassword::Scrypt(format!(
        "$scrypt${}${SCRYPT_R}${SCRYPT_P}${salt}${}",
        1u32 << SCRYPT_LOG_N,
        hex::encode(out)
    )))
}

fn verify_scrypt(encoded: &str, candidate: &str) -> bool {
    let mut parts = encoded.trim_start_matches('$').split('$');
    if parts.next() != Some("scrypt") {
        return false;
    }

    let Some(n) = parts.next().and_then(|v| v.parse::<u32>().ok()) else {
        return false;
    };
    let Some(r) = parts.next().and_then(|v| v.parse::<u32>().ok()) else {
        return false;
    };
    let Some(p) = parts.next().and_then(|v| v.parse::<u32>().ok()) else {
        return false;
    };
    let (Some(salt), Some(expected)) = (parts.next(), parts.next()) else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }
    let Ok(expected) = hex::decode(expected) else {
        return false;
    };

    // The parameters come from a file. A work factor nobody chose would hang
    // the daemon on the next login.
    if !n.is_power_of_two() || !(2..=(1 << 20)).contains(&n) {
        return false;
    }
    let Ok(params) = scrypt::Params::new(n.trailing_zeros() as u8, r, p, expected.len()) else {
        return false;
    };

    let mut derived = vec![0u8; expected.len()];
    if scrypt::scrypt(candidate.as_bytes(), salt.as_bytes(), &params, &mut derived).is_err() {
        return false;
    }
    constant_time_eq(&derived, &expected)
}

fn random_string(length: usize) -> Result<String> {
    let rng = ring::rand::SystemRandom::new();
    let mut raw = vec![0u8; length];
    rng.fill(&mut raw)
        .map_err(|_| Error::Random("system rng"))?;
    Ok(raw
        .into_iter()
        .map(|byte| SALT_CHARS[usize::from(byte) % SALT_CHARS.len()] as char)
        .collect())
}

/// A password for the `localclient` account, which nobody types.
fn random_password() -> Result<String> {
    let rng = ring::rand::SystemRandom::new();
    let mut raw = [0u8; 20];
    rng.fill(&mut raw)
        .map_err(|_| Error::Random("system rng"))?;
    Ok(hex::encode(raw))
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}
