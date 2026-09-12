// SPDX-License-Identifier: GPL-3.0-or-later
//! An SSL torrent, end to end.
//!
//! An SSL torrent carries a certificate authority in its `info` dictionary and
//! will not start until the client has been given a certificate signed by that
//! authority. libtorrent says so by raising `torrent_need_cert_alert`, and
//! that alert is the observable thing worth testing: it proves the torrent was
//! recognised as an SSL one rather than added as an ordinary torrent whose
//! extra field was ignored.
//!
//! Everything here is generated in the test. There is no fixture, because a
//! certificate in a fixture expires.

use std::time::Duration;

use redeluge_libtorrent::{AddTorrent, AlertKind, Session, SessionSettings};

/// A certificate authority, and a certificate it signed.
struct Authority {
    ca_pem: String,
    leaf_pem: String,
    leaf_key_pem: String,
}

fn authority() -> Authority {
    use rcgen::{CertificateParams, DnType, IsCa, KeyPair};

    let mut ca_params = CertificateParams::new(vec!["redeluge-test-ca".to_owned()])
        .expect("valid subject alt name");
    ca_params.is_ca = IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    ca_params
        .distinguished_name
        .push(DnType::CommonName, "redeluge test CA");
    let ca_key = KeyPair::generate().expect("a key pair");
    let ca = ca_params.self_signed(&ca_key).expect("a self-signed CA");

    let leaf_params =
        CertificateParams::new(vec!["redeluge-test-peer".to_owned()]).expect("valid name");
    let leaf_key = KeyPair::generate().expect("a key pair");
    let leaf = leaf_params
        .signed_by(&leaf_key, &ca, &ca_key)
        .expect("a signed certificate");

    Authority {
        ca_pem: ca.pem(),
        leaf_pem: leaf.pem(),
        leaf_key_pem: leaf_key.serialize_pem(),
    }
}

/// A `.torrent` for a single twelve-byte file, optionally an SSL one.
///
/// Written out rather than built with `create_torrent`, because the bridge's
/// builder has no way to set the certificate authority and adding one to its
/// signature for a test would be the tail wagging the dog.
fn torrent_file(name: &str, ssl_cert: Option<&str>) -> Vec<u8> {
    fn bytes(value: &[u8]) -> Vec<u8> {
        let mut out = format!("{}:", value.len()).into_bytes();
        out.extend_from_slice(value);
        out
    }

    // One piece, whose hash is never checked because no data is ever read.
    let pieces = [0u8; 20];

    let mut info = b"d".to_vec();
    info.extend_from_slice(&bytes(b"length"));
    info.extend_from_slice(b"i12e");
    info.extend_from_slice(&bytes(b"name"));
    info.extend_from_slice(&bytes(name.as_bytes()));
    info.extend_from_slice(&bytes(b"piece length"));
    info.extend_from_slice(b"i16384e");
    info.extend_from_slice(&bytes(b"pieces"));
    info.extend_from_slice(&bytes(&pieces));
    if let Some(certificate) = ssl_cert {
        // The key libtorrent looks for. Its presence is what makes this an SSL
        // torrent, and it has to sort after `pieces` for the dictionary to be
        // well-formed bencode.
        info.extend_from_slice(&bytes(b"ssl-cert"));
        info.extend_from_slice(&bytes(certificate.as_bytes()));
    }
    info.push(b'e');

    let mut out = b"d".to_vec();
    out.extend_from_slice(&bytes(b"info"));
    out.extend_from_slice(&info);
    out.push(b'e');
    out
}

fn session() -> Session {
    Session::new(&SessionSettings::offline()).expect("offline session should start")
}

/// Waits for one kind of alert, or gives up.
fn wait_for(session: &mut Session, kind: AlertKind, attempts: u32) -> bool {
    for _ in 0..attempts {
        session.wait_for_alert(Duration::from_millis(200));
        for alert in session.pop_alerts() {
            if alert.kind == kind {
                return true;
            }
        }
    }
    false
}

#[test]
fn an_ssl_torrent_asks_for_a_certificate() {
    // The alert is the whole point: without it the torrent was added as an
    // ordinary one and the certificate field was quietly ignored.
    let authority = authority();
    let mut session = session();

    let dump = torrent_file("ssl-test", Some(&authority.ca_pem));
    let id = session
        .add_torrent(&AddTorrent::from_file(dump, "/tmp".to_owned()))
        .expect("an SSL torrent should add");

    assert_eq!(id.len(), 40);
    assert!(
        wait_for(&mut session, AlertKind::TorrentNeedCert, 25),
        "libtorrent should ask for a certificate"
    );
}

#[test]
fn an_ordinary_torrent_never_asks_for_one() {
    let mut session = session();
    let dump = torrent_file("plain-test", None);
    session
        .add_torrent(&AddTorrent::from_file(dump, "/tmp".to_owned()))
        .expect("an ordinary torrent should add");

    assert!(
        !wait_for(&mut session, AlertKind::TorrentNeedCert, 10),
        "a torrent with no certificate authority is not an SSL torrent"
    );
}

#[test]
fn a_certificate_signed_by_the_torrents_authority_is_accepted() {
    let authority = authority();
    let mut session = session();

    let dump = torrent_file("ssl-test", Some(&authority.ca_pem));
    let id = session
        .add_torrent(&AddTorrent::from_file(dump, "/tmp".to_owned()))
        .expect("an SSL torrent should add");
    assert!(wait_for(&mut session, AlertKind::TorrentNeedCert, 25));

    session
        .set_ssl_certificate(
            &id,
            authority.leaf_pem.as_bytes(),
            authority.leaf_key_pem.as_bytes(),
            b"",
            "",
        )
        .expect("a certificate signed by the torrent's own authority");
}

#[test]
fn setting_a_certificate_on_an_unknown_torrent_is_an_error() {
    let mut session = session();
    assert!(session
        .set_ssl_certificate("0".repeat(40).as_str(), b"", b"", b"", "")
        .is_err());
}

#[test]
fn rubbish_in_place_of_a_certificate_does_not_crash_the_session() {
    // The certificate comes from a client over the RPC, so it is not
    // necessarily a certificate at all. libtorrent may accept the call and
    // fail later; what must not happen is a crash here.
    let authority = authority();
    let mut session = session();

    let dump = torrent_file("ssl-test", Some(&authority.ca_pem));
    let id = session
        .add_torrent(&AddTorrent::from_file(dump, "/tmp".to_owned()))
        .expect("an SSL torrent should add");

    let _ = session.set_ssl_certificate(&id, b"not a certificate", b"not a key", b"", "");
    assert!(session.is_valid(&id), "the session survived it");
}
