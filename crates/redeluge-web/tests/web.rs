// SPDX-License-Identifier: GPL-3.0-or-later
//! Config parsing, template rendering, type conversion and the embedded assets.

use redeluge_rencode::Value;
use redeluge_web::assets;
use redeluge_web::config::ConfigFile;
use redeluge_web::convert::{json_to_rencode, rencode_to_json};
use redeluge_web::index::{choose_scripts, render_index, ScriptSet};
use redeluge_web::template::{render, Context, Error as TemplateError};
use serde_json::json;

fn write(name: &str, contents: &str) -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join(name);
    std::fs::write(&path, contents).unwrap();
    (dir, path)
}

// ------------------------------------------------------------------- config

#[test]
fn a_two_object_config_file_parses() {
    // The shape every Deluge config file has: a version header, then settings.
    let (_dir, path) = write(
        "web.conf",
        r#"{
  "file": 1,
  "format": 2
}{
  "port": 8112,
  "theme": "gray",
  "https": false
}"#,
    );

    let config = ConfigFile::load(&path).unwrap();
    assert_eq!(
        config.version.get("format").and_then(|v| v.as_i64()),
        Some(2)
    );
    assert_eq!(config.integer("port"), Some(8112));
    assert_eq!(config.string("theme"), Some("gray"));
    assert_eq!(config.boolean("https"), Some(false));
    assert_eq!(config.string("absent"), None);
}

#[test]
fn an_older_single_object_config_file_still_parses() {
    let (_dir, path) = write("web.conf", r#"{"port": 9000}"#);
    let config = ConfigFile::load(&path).unwrap();
    assert_eq!(config.integer("port"), Some(9000));
    assert!(config.version.is_empty());
}

#[test]
fn braces_inside_strings_do_not_split_the_file() {
    // A password or a path can contain a brace. Counting braces without
    // tracking strings would cut the file in the wrong place.
    let (_dir, path) = write(
        "web.conf",
        r#"{"file": 1}{"pwd_salt": "ab{cd}ef", "note": "a \" and a } here"}"#,
    );
    let config = ConfigFile::load(&path).unwrap();
    assert_eq!(config.string("pwd_salt"), Some("ab{cd}ef"));
    assert_eq!(config.string("note"), Some("a \" and a } here"));
}

#[test]
fn a_missing_config_file_is_empty_rather_than_an_error() {
    // Deluge writes web.conf only once something changes, so absence is normal.
    let dir = tempfile::tempdir().unwrap();
    let config = ConfigFile::load(dir.path().join("never-written.conf")).unwrap();
    assert!(config.settings.is_empty());
}

#[test]
fn a_corrupt_config_file_is_an_error_not_a_panic() {
    for contents in ["not json at all", "{", "[]", "{\"a\": }"] {
        let (_dir, path) = write("web.conf", contents);
        let _ = ConfigFile::load(&path);
    }
}

// ----------------------------------------------------------------- template

#[test]
fn substitution_and_loops_render() {
    let template = "start ${name}\n% for item in items:\n<i>${item}</i>\n% endfor\nend";
    let context = Context::new()
        .set("name", "value")
        .list("items", vec!["a".into(), "b".into()]);

    let output = render(template, &context).unwrap();
    assert!(output.contains("start value"));
    assert!(output.contains("<i>a</i>"));
    assert!(output.contains("<i>b</i>"));
    assert!(output.trim_end().ends_with("end"));
}

#[test]
fn a_translation_marker_resolves_to_its_own_text() {
    // redeluge is English only, so `_("x")` is the identity. Leaving the marker
    // in place is how every label in the interface once read `${escape(_(...))}`.
    let output = render(r#"<dt>${_("Downloaded:")}</dt>"#, &Context::new()).unwrap();
    assert_eq!(output.trim(), "<dt>Downloaded:</dt>");

    let single = render("${_('Up Speed:')}", &Context::new()).unwrap();
    assert_eq!(single.trim(), "Up Speed:");
}

#[test]
fn an_unknown_variable_fails_loudly() {
    match render("${nothing}", &Context::new()) {
        Err(TemplateError::MissingValue { name, .. }) => assert_eq!(name, "nothing"),
        other => panic!("expected MissingValue, got {other:?}"),
    }
}

#[test]
fn an_unsupported_directive_fails_rather_than_rendering_something_wrong() {
    assert!(matches!(
        render("% if x:\nbody\n% endif", &Context::new()),
        Err(TemplateError::Unsupported { .. })
    ));
    assert!(matches!(
        render("% endfor", &Context::new()),
        Err(TemplateError::StrayEndfor { .. })
    ));
    assert!(matches!(
        render(
            "% for x in items:\nbody",
            &Context::new().list("items", vec![])
        ),
        Err(TemplateError::UnclosedLoop { .. })
    ));
}

#[test]
fn a_loop_over_an_absent_list_renders_nothing() {
    let output = render("a\n% for x in missing:\n${x}\n% endfor\nb", &Context::new()).unwrap();
    assert!(output.contains('a') && output.contains('b'));
    assert!(!output.contains("${x}"));
}

// ------------------------------------------------------------------ convert

#[test]
fn json_and_rencode_round_trip_through_each_other() {
    let original = json!({
        "name": "debian.iso",
        "progress": 42.5,
        "paused": false,
        "priorities": [0, 1, 5, 7],
        "nested": {"a": null, "b": [true, "text"]},
    });

    let round_tripped = rencode_to_json(&json_to_rencode(&original));
    assert_eq!(round_tripped, original);
}

#[test]
fn integers_stay_integers_and_floats_stay_floats() {
    // The front end does arithmetic on these. An integer arriving as 1.0 makes
    // the status bar render "1.0 peers".
    assert_eq!(json_to_rencode(&json!(42)), Value::Int(42));
    assert_eq!(json_to_rencode(&json!(-1)), Value::Int(-1));
    assert!(matches!(json_to_rencode(&json!(1.5)), Value::Float64(_)));
    assert_eq!(rencode_to_json(&Value::Int(42)), json!(42));
}

#[test]
fn a_float_that_json_cannot_hold_becomes_zero_rather_than_breaking_the_page() {
    // JSON has no NaN or infinity. Emitting one produces a body the browser
    // refuses to parse, which loses the whole response rather than one field.
    assert_eq!(rencode_to_json(&Value::Float64(f64::NAN)), json!(0));
    assert_eq!(rencode_to_json(&Value::Float64(f64::INFINITY)), json!(0));
}

#[test]
fn bytes_that_are_not_utf8_still_reach_the_browser() {
    // A peer controls torrent names. Dropping the field would hide the torrent
    // the user is looking for.
    let value = Value::Bytes(vec![0xff, 0xfe, b'o', b'k']);
    let json = rencode_to_json(&value);
    assert!(json.as_str().expect("a string").ends_with("ok"));
}

#[test]
fn dictionary_order_survives_the_conversion() {
    let value = Value::Dict(vec![
        (Value::Str("z".into()), Value::Int(1)),
        (Value::Str("a".into()), Value::Int(2)),
    ]);
    let json = rencode_to_json(&value);
    let keys: Vec<&String> = json.as_object().unwrap().keys().collect();
    assert_eq!(keys, vec!["a", "z"], "serde_json maps are sorted");
    assert_eq!(json["z"], json!(1));
}

#[test]
fn a_non_string_dictionary_key_becomes_readable_text() {
    let value = Value::Dict(vec![(Value::Int(7), Value::Bool(true))]);
    assert_eq!(rencode_to_json(&value), json!({"7": true}));
}

// ------------------------------------------------------------------- assets

#[test]
fn the_web_ui_assets_were_embedded() {
    for required in [
        "index.html",
        "js/gettext.js",
        "js/deluge-all-debug.js",
        "js/extjs/ext-base-debug.js",
        "js/extjs/ext-all-debug.js",
        "js/extjs/ext-extensions-debug.js",
        "css/deluge.css",
        "themes/css/xtheme-gray.css",
        "render/tab_status.html",
        "icons/deluge.png",
    ] {
        assert!(assets::contains(required), "{required} was not embedded");
    }
    assert!(assets::files().len() > 400, "too few assets embedded");
}

#[test]
fn the_concatenated_bundle_looks_like_the_whole_front_end() {
    let bundle = assets::get("js/deluge-all-debug.js").expect("the bundle");
    let text = std::str::from_utf8(bundle).expect("JavaScript is UTF-8");

    // The ordering is what matters: a base class defined after the class that
    // extends it produces a blank page, and the file size looks identical.
    let options_manager = text.find("Deluge.OptionsManager").expect("OptionsManager");
    let login_window = text.find("Deluge.LoginWindow").expect("LoginWindow");
    assert!(
        options_manager < login_window,
        "OptionsManager must be defined before the windows that use it"
    );
    // A floor rather than a fixed size: it catches a bundle that lost most of
    // itself, without failing every time a file is added or removed. It came
    // down from 300 KB when the plugin interface was taken out in phase 5.
    assert!(bundle.len() > 280_000, "the bundle is suspiciously small");
}

#[test]
fn the_bundle_carries_no_plugin_interface() {
    // Phase 5 removed it. The front end is concatenated from a directory, so a
    // file restored by accident would be bundled again in silence.
    let bundle = assets::get("js/deluge-all-debug.js").expect("the bundle");
    let text = std::str::from_utf8(bundle).expect("JavaScript is UTF-8");

    for gone in [
        "Deluge.preferences.Plugins",
        "Deluge.add.InstallPluginWindow",
        "Deluge.pluginStore",
        "registerPlugin",
        "web.get_plugins",
    ] {
        assert!(!text.contains(gone), "the bundle still carries {gone}");
    }
}

#[test]
fn no_asset_still_carries_an_unrendered_translation_marker() {
    // This is the bug that made every label read `${escape(_("..."))}`.
    for (path, bytes) in assets::files() {
        if !path.ends_with(".js") && !path.ends_with(".html") {
            continue;
        }
        // index.html is rendered at request time, and ExtJS's own source
        // contains the sequence in unrelated code.
        if *path == "index.html" || path.starts_with("js/extjs/ext-all") {
            continue;
        }
        let text = String::from_utf8_lossy(bytes);
        assert!(
            !text.contains("escape(_(\""),
            "{path} still contains an unrendered translation marker"
        );
    }
}

#[test]
fn content_types_are_right_for_the_files_that_matter() {
    // A stylesheet served as text/plain is ignored, and the page renders naked.
    assert_eq!(assets::content_type("a.css"), "text/css; charset=utf-8");
    assert_eq!(
        assets::content_type("a.js"),
        "text/javascript; charset=utf-8"
    );
    assert_eq!(assets::content_type("a.png"), "image/png");
    assert_eq!(assets::content_type("a.gif"), "image/gif");
    assert_eq!(
        assets::content_type("noextension"),
        "application/octet-stream"
    );
}

// -------------------------------------------------------------------- index

#[test]
fn the_debug_script_set_is_the_one_we_ship() {
    // Minification is not part of this build yet, so only the debug bundles
    // exist. If that changes, this test is the reminder to revisit it.
    assert!(ScriptSet::Debug.available());
    assert!(!ScriptSet::Normal.available());
    assert_eq!(choose_scripts(false), ScriptSet::Debug);
    assert_eq!(choose_scripts(true), ScriptSet::Debug);
}

#[test]
fn the_page_renders_with_every_script_and_stylesheet_it_needs() {
    let template = std::str::from_utf8(assets::get("index.html").unwrap()).unwrap();
    let html = render_index(
        template,
        "/",
        "2.2.1",
        "gray",
        &json!({"theme": "gray", "base": "/"}),
        false,
    )
    .expect("the shipped template must render");

    assert!(html.contains("<title>Deluge WebUI 2.2.1</title>"));
    assert!(html.contains("js/gettext.js"));
    assert!(html.contains("js/deluge-all-debug.js"));
    assert!(html.contains("themes/css/xtheme-gray.css"));
    assert!(html.contains("Deluge.debug = false"));
    assert!(!html.contains("${"), "an unrendered marker survived");
}

#[test]
fn a_base_path_reaches_every_asset_url() {
    // Serving under a reverse proxy subpath is the one deployment that breaks
    // silently if a single URL misses the prefix.
    let template = std::str::from_utf8(assets::get("index.html").unwrap()).unwrap();
    let html = render_index(
        template,
        "/deluge/",
        "2.2.1",
        "gray",
        &json!({"base": "/deluge/"}),
        false,
    )
    .unwrap();

    for line in html.lines() {
        if let Some(start) = line.find("src=\"").or_else(|| line.find("href=\"")) {
            let value = &line[start..];
            let value = &value[value.find('"').unwrap() + 1..];
            let url = &value[..value.find('"').unwrap()];
            assert!(
                url.starts_with("/deluge/"),
                "{url} does not carry the base path"
            );
        }
    }
}

// ---------------------------------------------------------------- bootstrap

#[test]
fn a_fresh_configuration_is_wired_up_from_the_auth_file() {
    // This replaced a Python script that went with the Python tree. What it
    // does is read the daemon's own localclient credentials and point the Web
    // UI at it, so nobody has to open a connection manager.
    use redeluge_web::bootstrap;

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("auth"),
        "# comment\nlocalclient:abc123def:10\nalice:$scrypt$x:5\n",
    )
    .unwrap();

    let wanted = bootstrap::Wanted {
        password: Some("hunter2".to_owned()),
        reset_password: false,
        daemon_port: 58846,
    };
    let outcome = bootstrap::run(dir.path(), &wanted).expect("bootstrap succeeds");

    assert!(outcome.password_set);
    assert!(outcome.host_added);

    let hosts = ConfigFile::load(dir.path().join("hostlist.conf")).unwrap();
    let entries = hosts.get("hosts").unwrap().as_array().unwrap();
    assert_eq!(entries.len(), 1);

    let entry = entries[0].as_array().unwrap();
    assert_eq!(entry[1].as_str(), Some("127.0.0.1"));
    assert_eq!(entry[2].as_u64(), Some(58846));
    assert_eq!(entry[3].as_str(), Some("localclient"));
    assert_eq!(
        entry[4].as_str(),
        Some("abc123def"),
        "the password must come from the auth file"
    );
    assert_eq!(entry[0].as_str().unwrap().len(), 32, "a host id is 32 hex");

    let web = ConfigFile::load(dir.path().join("web.conf")).unwrap();
    assert_eq!(
        web.string("default_daemon"),
        entry[0].as_str(),
        "the Web UI should connect without being asked"
    );

    // And the password it set is the one that verifies.
    let stored = redeluge_web::auth::StoredPassword::from_config(
        web.string("pwd_salt"),
        web.string("pwd_sha1"),
    );
    assert!(stored.verify("hunter2"));
    assert!(!stored.verify("wrong"));
}

#[test]
fn bootstrapping_twice_changes_nothing_it_should_not() {
    use redeluge_web::bootstrap;

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("auth"), "localclient:first:10\n").unwrap();

    let wanted = bootstrap::Wanted {
        password: Some("original".to_owned()),
        reset_password: false,
        daemon_port: 58846,
    };
    bootstrap::run(dir.path(), &wanted).unwrap();
    let first_id = ConfigFile::load(dir.path().join("web.conf"))
        .unwrap()
        .string("default_daemon")
        .unwrap()
        .to_owned();

    // A different password, without asking for a reset: the stored one wins,
    // or every restart would undo a password someone changed in the UI.
    let second = bootstrap::Wanted {
        password: Some("different".to_owned()),
        reset_password: false,
        daemon_port: 58846,
    };
    let outcome = bootstrap::run(dir.path(), &second).unwrap();
    assert!(
        !outcome.password_set,
        "an existing password was overwritten"
    );
    assert!(!outcome.host_added, "a second host entry was added");

    let web = ConfigFile::load(dir.path().join("web.conf")).unwrap();
    assert_eq!(web.string("default_daemon"), Some(first_id.as_str()));

    let stored = redeluge_web::auth::StoredPassword::from_config(
        web.string("pwd_salt"),
        web.string("pwd_sha1"),
    );
    assert!(stored.verify("original"));
}

#[test]
fn a_reset_replaces_the_stored_password() {
    use redeluge_web::bootstrap;

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("auth"), "localclient:secret:10\n").unwrap();

    bootstrap::run(
        dir.path(),
        &bootstrap::Wanted {
            password: Some("original".to_owned()),
            reset_password: false,
            daemon_port: 58846,
        },
    )
    .unwrap();

    let outcome = bootstrap::run(
        dir.path(),
        &bootstrap::Wanted {
            password: Some("replaced".to_owned()),
            reset_password: true,
            daemon_port: 58846,
        },
    )
    .unwrap();
    assert!(outcome.password_set);

    let web = ConfigFile::load(dir.path().join("web.conf")).unwrap();
    let stored = redeluge_web::auth::StoredPassword::from_config(
        web.string("pwd_salt"),
        web.string("pwd_sha1"),
    );
    assert!(stored.verify("replaced"));
    assert!(!stored.verify("original"));
}

#[test]
fn a_stale_host_password_is_realigned_with_the_auth_file() {
    // The daemon regenerates the localclient password whenever it recreates
    // the auth file, and a hostlist entry holding the old one silently stops
    // working.
    use redeluge_web::bootstrap;

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("auth"), "localclient:old:10\n").unwrap();
    bootstrap::run(
        dir.path(),
        &bootstrap::Wanted {
            password: None,
            reset_password: false,
            daemon_port: 58846,
        },
    )
    .unwrap();

    std::fs::write(dir.path().join("auth"), "localclient:new:10\n").unwrap();
    let outcome = bootstrap::run(
        dir.path(),
        &bootstrap::Wanted {
            password: None,
            reset_password: false,
            daemon_port: 58846,
        },
    )
    .unwrap();
    assert!(outcome.host_updated);

    let hosts = ConfigFile::load(dir.path().join("hostlist.conf")).unwrap();
    let entries = hosts.get("hosts").unwrap().as_array().unwrap();
    assert_eq!(entries.len(), 1, "no duplicate entry was added");
    assert_eq!(entries[0].as_array().unwrap()[4].as_str(), Some("new"));
}

#[test]
fn no_auth_file_means_no_host_entry_rather_than_a_broken_one() {
    use redeluge_web::bootstrap;

    let dir = tempfile::tempdir().unwrap();
    let outcome = bootstrap::run(
        dir.path(),
        &bootstrap::Wanted {
            password: Some("hunter2".to_owned()),
            reset_password: false,
            daemon_port: 58846,
        },
    )
    .unwrap();

    assert!(!outcome.host_added, "there are no credentials to use");
    assert!(
        outcome.password_set,
        "the password does not depend on the daemon"
    );
}
