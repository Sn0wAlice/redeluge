// SPDX-License-Identifier: GPL-3.0-or-later
//! The daemon's certificate.
//!
//! Deluge generates a self-signed certificate on first run and keeps it in
//! `ssl/daemon.cert` and `ssl/daemon.pkey`. Clients do not verify it, so it
//! proves nothing about who is listening; what it does is encrypt the
//! connection, which matters because the login travels over it.
//!
//! An existing pair is reused rather than replaced, so a client that pinned the
//! fingerprint keeps working across a restart.
//!
//! With one exception the Python daemon forces: the certificate it generates is
//! X.509 version 1, and rustls will not use one. Refusing to start over it would
//! strand every existing installation for a certificate nothing verifies, so an
//! unusable pair is moved aside and a new one generated. The old files are kept
//! rather than deleted, and the log says what happened.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rustls::ServerConfig;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};

#[derive(Debug, thiserror::Error)]
pub enum Error {
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
    #[error("could not generate a certificate: {0}")]
    Generate(String),
    #[error("the stored certificate is unusable: {0}")]
    Unusable(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// Loads the daemon's certificate, generating one when there is none.
pub fn server_config(config_dir: &Path) -> Result<ServerConfig> {
    let ssl_dir = config_dir.join("ssl");
    let cert_path = ssl_dir.join("daemon.cert");
    let key_path = ssl_dir.join("daemon.pkey");

    // Reuse the stored pair, but only if rustls will actually take it.
    if let (Ok(cert), Ok(key)) = (
        std::fs::read_to_string(&cert_path),
        std::fs::read_to_string(&key_path),
    ) {
        match build(&cert, &key) {
            Ok(config) => return Ok(config),
            Err(err) => {
                // Almost always the Python daemon's certificate, which is
                // X.509 version 1. Refusing to start over a certificate that
                // nothing verifies would strand every existing installation.
                tracing::warn!(
                    error = %err,
                    "the stored daemon certificate cannot be used, generating a new one"
                );
                for path in [&cert_path, &key_path] {
                    // Appended, not substituted: with_extension would turn
                    // both daemon.cert and daemon.pkey into daemon.unusable,
                    // and the second would overwrite the first.
                    let mut aside = path.clone().into_os_string();
                    aside.push(".unusable");
                    let aside = std::path::PathBuf::from(aside);
                    if std::fs::rename(path, &aside).is_ok() {
                        tracing::info!(kept = %aside.display(), "kept the old file");
                    }
                }
            }
        }
    }

    let (cert_pem, key_pem) = generate()?;
    std::fs::create_dir_all(&ssl_dir).map_err(|source| Error::Write {
        path: ssl_dir.clone(),
        source,
    })?;
    write_private(&key_path, key_pem.as_bytes())?;
    std::fs::write(&cert_path, cert_pem.as_bytes()).map_err(|source| Error::Write {
        path: cert_path.clone(),
        source,
    })?;
    tracing::info!(path = %cert_path.display(), "generated a daemon certificate");

    build(&cert_pem, &key_pem)
}

fn build(cert_pem: &str, key_pem: &str) -> Result<ServerConfig> {
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cert_pem.as_bytes())
        .collect::<std::result::Result<_, _>>()
        .map_err(|err| Error::Unusable(err.to_string()))?;
    if certs.is_empty() {
        return Err(Error::Unusable("no certificate in the file".to_owned()));
    }

    let key: PrivateKeyDer<'static> = rustls_pemfile::private_key(&mut key_pem.as_bytes())
        .map_err(|err| Error::Unusable(err.to_string()))?
        .ok_or_else(|| Error::Unusable("no private key in the file".to_owned()))?;

    ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|err| Error::Unusable(err.to_string()))
}

/// Generates a self-signed certificate for the daemon.
fn generate() -> Result<(String, String)> {
    // The name is cosmetic: no client checks it, and the daemon is reached by
    // address rather than by name.
    let mut params = rcgen::CertificateParams::new(vec!["redeluge-daemon".to_owned()])
        .map_err(|err| Error::Generate(err.to_string()))?;
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "redeluge daemon");

    let key = rcgen::KeyPair::generate().map_err(|err| Error::Generate(err.to_string()))?;
    let certificate = params
        .self_signed(&key)
        .map_err(|err| Error::Generate(err.to_string()))?;

    Ok((certificate.pem(), key.serialize_pem()))
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    std::fs::write(path, bytes).map_err(|source| Error::Write {
        path: path.to_path_buf(),
        source,
    })?;

    // A world-readable private key is the same as no key at all.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// The SHA-256 fingerprint of the daemon's certificate.
///
/// What a thin client pins when it connects across a network.
pub fn fingerprint(config_dir: &Path) -> Result<String> {
    let path = config_dir.join("ssl").join("daemon.cert");
    let pem = std::fs::read_to_string(&path).map_err(|source| Error::Read {
        path: path.clone(),
        source,
    })?;
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut pem.as_bytes())
        .collect::<std::result::Result<_, _>>()
        .map_err(|err| Error::Unusable(err.to_string()))?;
    let first = certs
        .first()
        .ok_or_else(|| Error::Unusable("no certificate in the file".to_owned()))?;

    let digest = ring::digest::digest(&ring::digest::SHA256, first.as_ref());
    Ok(hex::encode(digest.as_ref()))
}

/// Wraps a config for the listener.
pub fn into_acceptor(config: ServerConfig) -> tokio_rustls::TlsAcceptor {
    tokio_rustls::TlsAcceptor::from(Arc::new(config))
}
