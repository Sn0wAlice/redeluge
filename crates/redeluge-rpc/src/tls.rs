// SPDX-License-Identifier: GPL-3.0-or-later
//! TLS for the daemon connection.
//!
//! The daemon presents a certificate it generated for itself on first run, and
//! the Python client does not verify it: `deluge/ui/client.py` uses a context
//! factory with no verification at all. Refusing to connect to a daemon the
//! Python client connects to would make this a non-replacement, so the same
//! behaviour is available, but it is named for what it is and it is not the
//! default you get by accident.
//!
//! The transport is still encrypted; what is missing is proof of who is on the
//! other end. Over loopback, which is where `allow_remote` leaves the daemon,
//! that is a reasonable trade. Across a network it is not, which is why
//! [`TlsMode::Pinned`] exists.

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::{ClientConfig, DigitallySignedStruct, Error as TlsError, SignatureScheme};
use rustls_pki_types::{CertificateDer, ServerName, UnixTime};

/// How much the client insists on knowing about the daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TlsMode {
    /// Encrypt, but accept any certificate.
    ///
    /// What the Python client does. Appropriate for a daemon on loopback, and
    /// only there: on a network this accepts an interceptor as readily as the
    /// daemon.
    Insecure,
    /// Accept only a certificate with this SHA-256 fingerprint.
    ///
    /// Read the daemon's own `ssl/daemon.cert` to get it. This is what a thin
    /// client across a network should use.
    Pinned { sha256: [u8; 32] },
}

impl TlsMode {
    pub fn client_config(&self) -> ClientConfig {
        let verifier: Arc<dyn ServerCertVerifier> = match self {
            Self::Insecure => Arc::new(AcceptAnyCertificate),
            Self::Pinned { sha256 } => Arc::new(PinnedCertificate { sha256: *sha256 }),
        };

        ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(verifier)
            .with_no_client_auth()
    }
}

/// Signature schemes both ends can agree on. Kept in one place so the two
/// verifiers cannot drift apart.
fn supported_schemes() -> Vec<SignatureScheme> {
    vec![
        SignatureScheme::RSA_PKCS1_SHA256,
        SignatureScheme::RSA_PKCS1_SHA384,
        SignatureScheme::RSA_PKCS1_SHA512,
        SignatureScheme::ECDSA_NISTP256_SHA256,
        SignatureScheme::ECDSA_NISTP384_SHA384,
        SignatureScheme::RSA_PSS_SHA256,
        SignatureScheme::RSA_PSS_SHA384,
        SignatureScheme::RSA_PSS_SHA512,
        SignatureScheme::ED25519,
    ]
}

#[derive(Debug)]
struct AcceptAnyCertificate;

impl ServerCertVerifier for AcceptAnyCertificate {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        Ok(HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        supported_schemes()
    }
}

#[derive(Debug)]
struct PinnedCertificate {
    sha256: [u8; 32],
}

impl ServerCertVerifier for PinnedCertificate {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        let actual = sha256(end_entity.as_ref());
        // Constant time: the fingerprint is not a secret, but comparing it in
        // variable time is a habit worth not forming.
        let matched = actual
            .iter()
            .zip(self.sha256.iter())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0;

        if matched {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(TlsError::General(
                "daemon certificate does not match the pinned fingerprint".to_owned(),
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        supported_schemes()
    }
}

fn sha256(data: &[u8]) -> [u8; 32] {
    let digest = ring::digest::digest(&ring::digest::SHA256, data);
    let mut out = [0u8; 32];
    out.copy_from_slice(digest.as_ref());
    out
}
