// SPDX-License-Identifier: GPL-3.0-or-later
//! Passwords and sessions for the Web UI.
//!
//! # What the Python implementation does, and what changes here
//!
//! `deluge/ui/web/auth.py` stores the password as one round of SHA-1 over a
//! salt, and compares the result with `==`. One round of SHA-1 is not a password
//! hash, and a non-constant-time comparison of a secret-derived value is a habit
//! worth not having.
//!
//! Existing passwords still work, because an installation must survive the
//! switch: a stored SHA-1 hash is verified as before, and then transparently
//! replaced with scrypt on the next successful login. New passwords are only
//! ever written as scrypt, using the same format as the daemon's own auth file
//! so the two halves of Deluge finally agree.
//!
//! Every comparison here is constant time.

use std::collections::HashMap;
use std::time::{Duration, SystemTime};

use ring::rand::SecureRandom;

/// Which scheme a stored password uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredPassword {
    /// `$scrypt$n$r$p$salt$hash`, the daemon's format.
    Scrypt {
        n: u32,
        r: u32,
        p: u32,
        salt: String,
        hash: Vec<u8>,
    },
    /// A salt and a single round of SHA-1, as the Python Web UI wrote it.
    LegacySha1 { salt: String, hash: String },
    /// Nothing has been set.
    Unset,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the stored password is not in a format this server understands")]
    UnknownFormat,
    #[error("scrypt parameters are out of range: n={n}, r={r}, p={p}")]
    BadParameters { n: u32, r: u32, p: u32 },
    #[error("could not read random bytes")]
    NoRandomness,
}

pub type Result<T> = std::result::Result<T, Error>;

/// scrypt cost, matching `deluge/security.py`.
const SCRYPT_N: u32 = 1 << 15;
const SCRYPT_R: u32 = 8;
const SCRYPT_P: u32 = 1;
const SCRYPT_LEN: usize = 64;
const SALT_CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

impl StoredPassword {
    /// Reads whichever scheme the configuration holds.
    pub fn from_config(pwd_salt: Option<&str>, pwd_sha1: Option<&str>) -> Self {
        match (pwd_salt, pwd_sha1) {
            (Some(salt), Some(hash)) if hash.starts_with("$scrypt$") => {
                // The daemon's format keeps everything in one field; the Web UI
                // has two, so a migrated password lives entirely in pwd_sha1.
                let _ = salt;
                parse_scrypt(hash).unwrap_or(Self::Unset)
            }
            (_, Some(hash)) if hash.starts_with("$scrypt$") => {
                parse_scrypt(hash).unwrap_or(Self::Unset)
            }
            (Some(salt), Some(hash)) if !salt.is_empty() && !hash.is_empty() => Self::LegacySha1 {
                salt: salt.to_owned(),
                hash: hash.to_owned(),
            },
            _ => Self::Unset,
        }
    }

    /// Whether this password matches. Always constant time.
    pub fn verify(&self, candidate: &str) -> bool {
        match self {
            Self::Unset => false,
            Self::LegacySha1 { salt, hash } => {
                let mut input = salt.as_bytes().to_vec();
                input.extend_from_slice(candidate.as_bytes());
                let digest = ring::digest::digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, &input);
                constant_time_eq(hex::encode(digest.as_ref()).as_bytes(), hash.as_bytes())
            }
            Self::Scrypt {
                n,
                r,
                p,
                salt,
                hash,
            } => match scrypt(candidate, salt, *n, *r, *p) {
                Ok(derived) => constant_time_eq(&derived, hash),
                Err(_) => false,
            },
        }
    }

    /// Whether a successful login should rewrite this in the current scheme.
    pub fn needs_upgrade(&self) -> bool {
        matches!(self, Self::LegacySha1 { .. })
    }

    /// The single configuration string to store, in the daemon's format.
    pub fn to_config_string(&self) -> Option<String> {
        match self {
            Self::Scrypt {
                n,
                r,
                p,
                salt,
                hash,
            } => Some(format!("$scrypt${n}${r}${p}${salt}${}", hex::encode(hash))),
            _ => None,
        }
    }
}

/// Hashes a new password with scrypt.
pub fn hash_password(password: &str) -> Result<StoredPassword> {
    let salt = random_salt(16)?;
    let hash = scrypt(password, &salt, SCRYPT_N, SCRYPT_R, SCRYPT_P)?;
    Ok(StoredPassword::Scrypt {
        n: SCRYPT_N,
        r: SCRYPT_R,
        p: SCRYPT_P,
        salt,
        hash,
    })
}

fn parse_scrypt(text: &str) -> Option<StoredPassword> {
    // $scrypt$n$r$p$salt$hexhash
    let mut parts = text.trim_start_matches('$').split('$');
    if parts.next()? != "scrypt" {
        return None;
    }
    let n = parts.next()?.parse().ok()?;
    let r = parts.next()?.parse().ok()?;
    let p = parts.next()?.parse().ok()?;
    let salt = parts.next()?.to_owned();
    let hash = hex::decode(parts.next()?).ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(StoredPassword::Scrypt {
        n,
        r,
        p,
        salt,
        hash,
    })
}

fn scrypt(password: &str, salt: &str, n: u32, r: u32, p: u32) -> Result<Vec<u8>> {
    // Guard the parameters before handing them to the cost function. They come
    // out of a configuration file, and a hostile one could otherwise ask for a
    // work factor that never returns.
    let sane = n.is_power_of_two()
        && (2..=(1 << 20)).contains(&n)
        && (1..=32).contains(&r)
        && (1..=16).contains(&p);
    if !sane {
        return Err(Error::BadParameters { n, r, p });
    }

    let params = scrypt::Params::new(n.trailing_zeros() as u8, r, p, SCRYPT_LEN)
        .map_err(|_| Error::BadParameters { n, r, p })?;

    let mut out = vec![0u8; SCRYPT_LEN];
    scrypt::scrypt(password.as_bytes(), salt.as_bytes(), &params, &mut out)
        .map_err(|_| Error::BadParameters { n, r, p })?;
    Ok(out)
}

fn random_salt(length: usize) -> Result<String> {
    let rng = ring::rand::SystemRandom::new();
    let mut raw = vec![0u8; length];
    rng.fill(&mut raw).map_err(|_| Error::NoRandomness)?;
    Ok(raw
        .into_iter()
        .map(|byte| SALT_CHARS[usize::from(byte) % SALT_CHARS.len()] as char)
        .collect())
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

// --------------------------------------------------------------------- sessions

/// A signed-in browser.
#[derive(Debug, Clone)]
pub struct Session {
    pub login: String,
    pub level: i64,
    pub expires: SystemTime,
}

/// The session table.
///
/// Sessions live only as long as the process. The Python implementation writes
/// them into `web.conf`, which means every browser that has ever logged in is
/// on disk, and a stolen config file is a stolen session. Keeping them in
/// memory costs a re-login after a restart and removes that entirely.
#[derive(Debug, Default)]
pub struct Sessions {
    entries: HashMap<String, Session>,
}

impl Sessions {
    pub fn new() -> Self {
        Self::default()
    }

    /// Issues a session and returns its identifier.
    pub fn create(&mut self, login: &str, level: i64, timeout: Duration) -> Result<String> {
        let rng = ring::rand::SystemRandom::new();
        let mut raw = [0u8; 32];
        rng.fill(&mut raw).map_err(|_| Error::NoRandomness)?;
        let id = hex::encode(raw);

        self.entries.insert(
            id.clone(),
            Session {
                login: login.to_owned(),
                level,
                expires: SystemTime::now() + timeout,
            },
        );
        Ok(id)
    }

    /// Looks up a session and extends it, as the Python version does on each
    /// authenticated request.
    pub fn touch(&mut self, id: &str, timeout: Duration) -> Option<Session> {
        let now = SystemTime::now();
        let entry = self.entries.get_mut(id)?;
        if entry.expires <= now {
            self.entries.remove(id);
            return None;
        }
        entry.expires = now + timeout;
        Some(entry.clone())
    }

    pub fn remove(&mut self, id: &str) -> bool {
        self.entries.remove(id).is_some()
    }

    /// Drops everything that has expired. Called on a timer.
    pub fn sweep(&mut self) -> usize {
        let now = SystemTime::now();
        let before = self.entries.len();
        self.entries.retain(|_, session| session.expires > now);
        before - self.entries.len()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}
