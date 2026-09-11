// SPDX-License-Identifier: GPL-3.0-or-later
//! Connects to a running Deluge daemon and checks the frozen contract against it.
//!
//! This is the point of replacing `deluge-web` before `deluged`: the codec gets
//! checked against a live reference implementation rather than against itself.
//!
//! ```text
//! daemon_probe <host> <port> <username> <password>
//! ```

use std::collections::BTreeSet;

use redeluge_rpc::{Client, ClientSettings};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [host, port, username, password] = args.as_slice() else {
        eprintln!("usage: daemon_probe <host> <port> <username> <password>");
        std::process::exit(2);
    };

    let client = Client::connect(host, port.parse()?, ClientSettings::default()).await?;

    // daemon.info answers before authentication, which is how a client learns
    // whether it can speak to this daemon at all.
    println!("daemon version: {}", client.info().await?);

    let level = client.login(username, password).await?;
    println!("logged in at auth level {level}");

    let live: BTreeSet<String> = client.method_list().await?.into_iter().collect();
    println!("daemon exposes {} methods", live.len());

    // The contract says which of those the Rust daemon will have to answer.
    let contract = redeluge_contract::Contract::get();
    let expected: BTreeSet<String> = contract
        .methods_for(redeluge_contract::Transport::Daemon)
        .map(|method| method.name.clone())
        .collect();

    let missing: Vec<&String> = expected.difference(&live).collect();
    let extra: Vec<&String> = live.difference(&expected).collect();

    println!("contract expects {} daemon methods", expected.len());
    if !missing.is_empty() {
        println!("in the contract but not on this daemon: {missing:?}");
    }
    if !extra.is_empty() {
        println!("on this daemon but not in the contract: {extra:?}");
    }

    // A real call with a real answer, so the round trip is proved end to end.
    let free = client.call("core.get_free_space", vec![]).await?;
    println!("core.get_free_space -> {free}");

    let status = client
        .call(
            "core.get_session_status",
            vec![redeluge_rencode::Value::List(vec![])],
        )
        .await?;
    println!("core.get_session_status returned {}", status.type_name());

    // And a call that must fail, to prove errors arrive as values.
    match client.call("core.no_such_method", vec![]).await {
        Err(err) => println!("unknown method correctly refused: {err}"),
        Ok(value) => println!("unexpected success: {value}"),
    }

    if missing.is_empty() {
        println!("\nOK: every daemon method in the contract exists on this daemon");
        Ok(())
    } else {
        Err(format!("{} contract methods are missing", missing.len()).into())
    }
}
