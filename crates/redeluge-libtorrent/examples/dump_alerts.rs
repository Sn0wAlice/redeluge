// SPDX-License-Identifier: GPL-3.0-or-later
//! Diagnostic: add a magnet, pause it, print every alert libtorrent raises.
use std::time::Duration;

use redeluge_libtorrent::{Session, SessionSettings};

fn main() -> Result<(), redeluge_libtorrent::Error> {
    let mut session = Session::new(&SessionSettings::offline())?;
    let hash = session.add_magnet(
        "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=redeluge-spike",
        "/tmp",
    )?;
    println!("added {hash}");

    for round in 0..30 {
        if round == 10 {
            println!("-- pausing --");
            session.pause_torrent(&hash)?;
        }
        session.wait_for_alert(Duration::from_millis(200));
        for alert in session.pop_alerts() {
            println!(
                "kind={:<3} what={:<24} hash={:<8} msg={}",
                alert.kind as u16,
                alert.what,
                alert.info_hash.as_deref().unwrap_or("-"),
                alert.message
            );
        }
    }
    Ok(())
}
