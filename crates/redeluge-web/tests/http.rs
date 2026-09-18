// SPDX-License-Identifier: GPL-3.0-or-later
//! The HTTP surface, against a booted server.
//!
//! Everything else in this crate's tests is pure logic. This boots the same
//! application the binary serves, through the same `routes::configure`, and
//! drives it over HTTP: the JSON dispatch, the session cookie, the asset
//! routes, the upload endpoint and the login throttle.
//!
//! No daemon is running, which is deliberate. Every call that needs one comes
//! back as an error rather than hanging, and that is worth pinning: a Web UI
//! whose daemon is down has to stay answerable.

use std::sync::Arc;
use std::time::Duration;

use actix_web::{test, web, App};
use redeluge_web::auth::{hash_password, Sessions};
use redeluge_web::state::{AppState, Settings, SharedState};
use redeluge_web::throttle::Throttle;
use serde_json::{json, Value as Json};
use tokio::sync::{Mutex, RwLock};

const PASSWORD: &str = "a-test-password";

fn state(config_dir: &std::path::Path) -> SharedState {
    Arc::new(AppState {
        settings: Settings {
            config_dir: config_dir.to_path_buf(),
            interface: "127.0.0.1".to_owned(),
            port: 0,
            base: "/".to_owned(),
            session_timeout: Duration::from_secs(3600),
            theme: "dark".to_owned(),
            default_daemon: None,
            version: "1.6.0".to_owned(),
        },
        password: RwLock::new(hash_password(PASSWORD).expect("hashable")),
        sessions: Mutex::new(Sessions::new()),
        login_throttle: Mutex::new(Throttle::new()),
        hosts: RwLock::new(Vec::new()),
        daemon: RwLock::new(None),
        client_settings: Default::default(),
        events: Mutex::new(Default::default()),
        events_ready: tokio::sync::Notify::new(),
        web_config: RwLock::new(redeluge_web::config::ConfigFile {
            path: config_dir.join("web.conf"),
            version: serde_json::Map::new(),
            settings: serde_json::Map::new(),
        }),
        slow_stats: Mutex::new(Default::default()),
    })
}

macro_rules! booted {
    ($state:expr) => {
        test::init_service(
            App::new()
                .app_data(web::Data::new($state.clone()))
                .app_data(web::JsonConfig::default().limit(8 * 1024 * 1024))
                .configure(redeluge_web::routes::configure),
        )
        .await
    };
}

/// One JSON-RPC call, with an optional session cookie. Returns the body and
/// any cookie the server set.
async fn call(
    app: &impl actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse,
        Error = actix_web::Error,
    >,
    method: &str,
    params: Json,
    cookie: Option<&str>,
) -> (Json, Option<String>) {
    let mut request = test::TestRequest::post()
        .uri("/json")
        .set_json(json!({"method": method, "params": params, "id": 1}));
    if let Some(cookie) = cookie {
        request = request.insert_header(("Cookie", format!("_session_id={cookie}")));
    }

    let response = test::call_service(app, request.to_request()).await;
    let set_cookie = response
        .headers()
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .and_then(|pair| pair.split_once('='))
        .map(|(_, value)| value.to_owned());

    let body: Json = test::read_body_json(response).await;
    (body, set_cookie)
}

async fn logged_in(
    app: &impl actix_web::dev::Service<
        actix_http::Request,
        Response = actix_web::dev::ServiceResponse,
        Error = actix_web::Error,
    >,
) -> String {
    let (body, cookie) = call(app, "auth.login", json!([PASSWORD]), None).await;
    assert_eq!(body["result"], json!(true), "login should succeed");
    cookie.expect("login sets a session cookie")
}

// -------------------------------------------------------------------- basics

#[actix_web::test]
async fn the_page_is_served_at_the_root() {
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));

    let response = test::call_service(&app, test::TestRequest::get().uri("/").to_request()).await;
    assert!(response.status().is_success());

    let body = test::read_body(response).await;
    let html = String::from_utf8_lossy(&body);
    assert!(html.contains("<title>"), "the page has a title");
    assert!(
        !html.contains("${"),
        "no unrendered template markers reach the browser"
    );
}

#[actix_web::test]
async fn an_embedded_asset_is_served_with_its_own_content_type() {
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));

    let response = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/js/deluge-all-debug.js")
            .to_request(),
    )
    .await;

    assert!(response.status().is_success());
    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    assert!(
        content_type.starts_with("text/javascript"),
        "{content_type}"
    );

    let body = test::read_body(response).await;
    assert!(body.len() > 100_000, "the bundle is there, not a stub");
}

#[actix_web::test]
async fn a_path_that_names_no_asset_is_answered_with_the_page() {
    // The front end routes on the fragment, so a reload of a deep link has to
    // come back with the page rather than a 404.
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));

    let response = test::call_service(
        &app,
        test::TestRequest::get().uri("/no/such/thing").to_request(),
    )
    .await;
    assert!(response.status().is_success());

    let body = test::read_body(response).await;
    assert!(String::from_utf8_lossy(&body).contains("<title>"));
}

#[actix_web::test]
async fn a_render_fragment_comes_back_rendered() {
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));

    let response = test::call_service(
        &app,
        test::TestRequest::get()
            .uri("/render/tab_status.html")
            .to_request(),
    )
    .await;

    assert!(response.status().is_success());
    let body = test::read_body(response).await;
    let html = String::from_utf8_lossy(&body);
    assert!(!html.contains("${_("), "the markers were substituted");
}

// --------------------------------------------------------------- the session

#[actix_web::test]
async fn a_call_without_a_session_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));

    let (body, _) = call(&app, "web.get_config", json!([]), None).await;
    assert_eq!(body["result"], Json::Null);
    assert_eq!(body["error"]["code"], json!(1), "not authenticated");
}

#[actix_web::test]
async fn three_methods_answer_before_a_session_exists() {
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));

    for method in ["auth.check_session", "system.listMethods"] {
        let (body, _) = call(&app, method, json!([]), None).await;
        assert_eq!(body["error"], Json::Null, "{method} should answer");
    }
    // The third is auth.login itself, covered by every other test here.
}

#[actix_web::test]
async fn the_wrong_password_is_false_rather_than_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));

    let (body, _) = call(&app, "auth.login", json!(["wrong"]), None).await;
    assert_eq!(body["result"], json!(false));
    assert_eq!(body["error"], Json::Null);
}

#[actix_web::test]
async fn logging_in_gives_a_session_that_later_calls_accept() {
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));
    let session = logged_in(&app).await;

    let (body, _) = call(&app, "auth.check_session", json!([]), Some(&session)).await;
    assert_eq!(body["result"], json!(true));

    let (body, _) = call(&app, "web.get_config", json!([]), Some(&session)).await;
    assert_eq!(body["error"], Json::Null);
    assert!(body["result"].is_object());
}

#[actix_web::test]
async fn a_session_cookie_that_was_tampered_with_is_refused() {
    // The cookie carries a checksum. It is not a security control, the id
    // behind it is, but a mangled cookie must not be treated as a session.
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));
    let session = logged_in(&app).await;

    let mut tampered = session.clone();
    tampered.insert(0, 'x');
    let (body, _) = call(&app, "web.get_config", json!([]), Some(&tampered)).await;
    assert_eq!(body["error"]["code"], json!(1));
}

#[actix_web::test]
async fn deleting_a_session_ends_it() {
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));
    let session = logged_in(&app).await;

    let (body, _) = call(&app, "auth.delete_session", json!([]), Some(&session)).await;
    assert_eq!(body["result"], json!(true));

    let (body, _) = call(&app, "web.get_config", json!([]), Some(&session)).await;
    assert_eq!(body["error"]["code"], json!(1), "the session is gone");
}

#[actix_web::test]
async fn guessing_is_slowed_down_after_a_burst() {
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));

    // Verifying a password costs real time here, so the assertion is on the
    // property rather than on which attempt is the one refused: a run of wrong
    // passwords has to get refused before it has run for long.
    let mut refused = 0;
    for _ in 0..redeluge_web::throttle::BURST + 3 {
        let (body, _) = call(&app, "auth.login", json!(["wrong"]), None).await;
        if body["result"] == Json::Null {
            assert!(
                body["error"]["message"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("too many"),
                "{body}"
            );
            refused += 1;
        } else {
            assert_eq!(body["result"], json!(false));
        }
    }
    assert!(refused > 0, "guessing was never slowed down");
}

#[actix_web::test]
async fn the_right_password_is_never_held_up() {
    // Only failures spend the budget, so opening several tabs at once must not
    // lock someone out of their own interface.
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));

    for _ in 0..redeluge_web::throttle::BURST + 3 {
        let (body, _) = call(&app, "auth.login", json!([PASSWORD]), None).await;
        assert_eq!(body["result"], json!(true), "{body}");
    }
}

// -------------------------------------------------------------- the dispatch

#[actix_web::test]
async fn an_unknown_method_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));
    let session = logged_in(&app).await;

    let (body, _) = call(&app, "web.no_such_method", json!([]), Some(&session)).await;
    assert_eq!(body["error"]["code"], json!(2));
}

#[actix_web::test]
async fn every_method_the_front_end_calls_is_answered() {
    // Not that each does the right thing, only that none of them is missing.
    // Ten of these were absent once, which made adding a torrent impossible
    // from the interface and nothing failed until someone clicked.
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));
    let session = logged_in(&app).await;

    const CALLED_BY_THE_FRONT_END: &[&str] = &[
        "web.add_host",
        "web.add_torrents",
        "web.connect",
        "web.connected",
        "web.disconnect",
        "web.download_torrent_from_url",
        "web.edit_host",
        "web.get_config",
        "web.get_events",
        "web.get_host_status",
        "web.get_hosts",
        "web.get_magnet_info",
        "web.get_torrent_files",
        "web.get_torrent_info",
        "web.get_torrent_status",
        "web.register_event_listener",
        "web.remove_host",
        "web.set_config",
        "web.set_theme",
        "web.start_daemon",
        "web.stop_daemon",
        "web.update_ui",
    ];

    for method in CALLED_BY_THE_FRONT_END {
        let (body, _) = call(&app, method, json!([]), Some(&session)).await;
        let code = body["error"]["code"].as_i64();
        assert_ne!(code, Some(2), "{method} is not answered at all");
    }
}

#[actix_web::test]
async fn a_call_that_needs_the_daemon_errors_rather_than_hanging() {
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));
    let session = logged_in(&app).await;

    let (body, _) = call(&app, "core.get_session_status", json!([[]]), Some(&session)).await;
    assert_eq!(body["error"]["code"], json!(4), "the daemon is not there");

    let (body, _) = call(&app, "web.connected", json!([]), Some(&session)).await;
    assert_eq!(body["result"], json!(false));
}

#[actix_web::test]
async fn the_advertised_method_list_is_what_the_server_answers() {
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));
    let session = logged_in(&app).await;

    let (body, _) = call(&app, "system.listMethods", json!([]), Some(&session)).await;
    let listed: Vec<String> = serde_json::from_value(body["result"].clone()).unwrap();

    assert!(listed.contains(&"web.add_torrents".to_owned()));
    assert!(listed.contains(&"auth.login".to_owned()));
    assert!(listed.iter().all(|name| name.contains('.')));

    // Everything it advertises must actually dispatch.
    for method in &listed {
        if method.starts_with("core.") || method.starts_with("daemon.") {
            continue; // Forwarded; no daemon here.
        }
        let (body, _) = call(&app, method, json!([]), Some(&session)).await;
        assert_ne!(
            body["error"]["code"].as_i64(),
            Some(2),
            "{method} is advertised but unknown"
        );
    }
}

// -------------------------------------------------------------------- themes

#[actix_web::test]
async fn the_theme_list_is_pairs_not_names() {
    // The interface loads these into a combo whose store has two fields. A
    // flat list of strings makes ExtJS read each string as a row and take its
    // first character as the value, so choosing "dark" set the theme to "d".
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));
    let session = logged_in(&app).await;

    let (body, _) = call(&app, "web.get_themes", json!([]), Some(&session)).await;
    let themes = body["result"].as_array().expect("a list");
    assert!(!themes.is_empty());

    for theme in themes {
        let pair = theme.as_array().expect("a pair, not a name");
        assert_eq!(pair.len(), 2, "{theme}");
        assert!(pair[0].as_str().is_some_and(|name| !name.is_empty()));
        assert!(pair[1].as_str().is_some_and(|label| !label.is_empty()));
    }

    let names: Vec<&str> = themes
        .iter()
        .filter_map(|theme| theme[0].as_str())
        .collect();
    // Two, named for what they are.
    assert_eq!(names, vec!["dark", "white"], "{names:?}");
}

#[actix_web::test]
async fn the_label_of_a_theme_is_its_name_capitalised() {
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));
    let session = logged_in(&app).await;

    let (body, _) = call(&app, "web.get_themes", json!([]), Some(&session)).await;
    let dark = body["result"]
        .as_array()
        .unwrap()
        .iter()
        .find(|theme| theme[0] == json!("dark"))
        .expect("the dark theme");
    assert_eq!(dark[1], json!("Dark"));
}

#[actix_web::test]
async fn webutils_get_themes_answers_the_same_thing() {
    // The interface page calls it under this name, not the web.* one.
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));
    let session = logged_in(&app).await;

    let (web, _) = call(&app, "web.get_themes", json!([]), Some(&session)).await;
    let (utils, _) = call(&app, "webutils.get_themes", json!([]), Some(&session)).await;
    assert_eq!(web["result"], utils["result"]);
}

#[actix_web::test]
async fn a_theme_that_exists_is_accepted_and_kept() {
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));
    let session = logged_in(&app).await;

    let (body, _) = call(&app, "web.set_theme", json!(["white"]), Some(&session)).await;
    assert_eq!(body["error"], Json::Null, "{body}");

    let (body, _) = call(&app, "web.get_config", json!([]), Some(&session)).await;
    assert_eq!(body["result"]["theme"], json!("white"));
}

#[actix_web::test]
async fn the_themes_this_fork_used_to_ship_still_answer_to_their_old_names() {
    // Two light themes and a dark one became one of each. A client that
    // remembers `gray`, or a `web.conf` written before the rename, must not
    // land on a stylesheet that is gone: that would change the colour of
    // somebody's interface on an upgrade, including turning a dark one light.
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));
    let session = logged_in(&app).await;

    for (old, new) in [("gray", "white"), ("blue", "white"), ("access", "dark")] {
        let (body, _) = call(&app, "web.set_theme", json!([old]), Some(&session)).await;
        assert_eq!(body["error"], Json::Null, "{old}: {body}");

        let (body, _) = call(&app, "web.get_config", json!([]), Some(&session)).await;
        assert_eq!(
            body["result"]["theme"],
            json!(new),
            "{old} should become {new}"
        );
    }
}

#[actix_web::test]
async fn a_theme_with_no_stylesheet_is_refused() {
    // Storing it would leave the page asking for a file that is not there,
    // which is how the interface ended up unstyled with no obvious cause.
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));
    let session = logged_in(&app).await;

    let (body, _) = call(&app, "web.set_theme", json!(["nope"]), Some(&session)).await;
    assert!(body["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .contains("no such theme"));

    let (body, _) = call(&app, "web.get_config", json!([]), Some(&session)).await;
    assert_ne!(body["result"]["theme"], json!("nope"));
}

#[actix_web::test]
async fn every_theme_offered_has_a_stylesheet_that_is_served() {
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));
    let session = logged_in(&app).await;

    let (body, _) = call(&app, "web.get_themes", json!([]), Some(&session)).await;
    for theme in body["result"].as_array().unwrap() {
        let name = theme[0].as_str().unwrap();
        let response = test::call_service(
            &app,
            test::TestRequest::get()
                .uri(&format!("/themes/css/xtheme-{name}.css"))
                .to_request(),
        )
        .await;
        assert!(response.status().is_success(), "{name} has no stylesheet");

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned();
        assert!(
            content_type.starts_with("text/css"),
            "{name}: {content_type}"
        );
    }
}

#[actix_web::test]
async fn a_stylesheet_that_is_not_there_is_a_404_not_the_page() {
    // Answering with the page hands the browser HTML where it asked for CSS.
    // It reports that as a parse error, not as a missing file, and the page
    // renders unstyled with nothing to point at.
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));

    for path in [
        "/themes/css/xtheme-nope.css",
        "/js/does-not-exist.js",
        "/icons/missing.png",
    ] {
        let response =
            test::call_service(&app, test::TestRequest::get().uri(path).to_request()).await;
        assert_eq!(response.status(), 404, "{path}");
    }
}

#[actix_web::test]
async fn a_front_end_route_is_still_answered_with_the_page() {
    // The other half of the same rule: reloading a deep link has to work.
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));

    let response = test::call_service(
        &app,
        test::TestRequest::get().uri("/some/route").to_request(),
    )
    .await;
    assert!(response.status().is_success());
    let body = test::read_body(response).await;
    assert!(String::from_utf8_lossy(&body).contains("<title>"));
}

// ------------------------------------------------------------------- magnets

#[actix_web::test]
async fn a_magnet_is_described_without_a_daemon() {
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));
    let session = logged_in(&app).await;

    let uri = "magnet:?xt=urn:btih:0123456789abcdef0123456789abcdef01234567&dn=Example";
    let (body, _) = call(&app, "web.get_magnet_info", json!([uri]), Some(&session)).await;

    assert_eq!(body["error"], Json::Null);
    assert_eq!(body["result"]["name"], json!("Example"));
    assert_eq!(
        body["result"]["info_hash"],
        json!("0123456789abcdef0123456789abcdef01234567")
    );
}

// -------------------------------------------------------------------- upload

/// A multipart body carrying one file.
fn multipart(name: &str, contents: &[u8]) -> (String, Vec<u8>) {
    let boundary = "----redelugetestboundary";
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(
        format!("Content-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\n")
            .as_bytes(),
    );
    body.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
    body.extend_from_slice(contents);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

/// The smallest thing that parses as a torrent.
const TORRENT: &[u8] = b"d4:infod6:lengthi12e4:name8:test.txtee";

#[actix_web::test]
async fn an_upload_without_a_session_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));
    let (content_type, body) = multipart("a.torrent", TORRENT);

    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/upload")
            .insert_header(("content-type", content_type))
            .set_payload(body)
            .to_request(),
    )
    .await;

    // Parsed from the bytes rather than with `read_body_json`, which insists
    // on an `application/json` content type this endpoint deliberately does
    // not use.
    let answer: Json = serde_json::from_slice(&test::read_body(response).await).unwrap();
    assert_eq!(answer["success"], json!(false));
}

#[actix_web::test]
async fn an_uploaded_torrent_can_then_be_asked_about() {
    // The whole add-by-file path, minus the daemon: upload, then
    // get_torrent_info on the path that came back.
    let dir = tempfile::tempdir().unwrap();
    let state = state(dir.path());
    let app = booted!(state);
    let session = logged_in(&app).await;

    let (content_type, body) = multipart("a.torrent", TORRENT);
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/upload")
            .insert_header(("content-type", content_type))
            .insert_header(("Cookie", format!("_session_id={session}")))
            .set_payload(body)
            .to_request(),
    )
    .await;

    let answer: Json = serde_json::from_slice(&test::read_body(response).await).unwrap();
    assert_eq!(answer["success"], json!(true), "{answer}");
    let path = answer["files"][0].as_str().expect("a path").to_owned();

    let (body, _) = call(&app, "web.get_torrent_info", json!([path]), Some(&session)).await;
    assert_eq!(body["error"], Json::Null, "{body}");
    assert_eq!(body["result"]["name"], json!("test.txt"));
    assert_eq!(
        body["result"]["files_tree"]["contents"]["test.txt"]["length"],
        json!(12)
    );
}

#[actix_web::test]
async fn the_upload_answers_as_html_because_extjs_reads_it_from_an_iframe() {
    // A form with `fileUpload: true` posts through a hidden iframe, and the
    // browser parses the response to build that iframe's document. Only
    // `text/html` makes it insert the body unchanged where ExtJS can read it
    // back. `application/json` makes every upload fail with "Failed to upload
    // torrent" and nothing in the log, which is exactly what happened.
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));
    let session = logged_in(&app).await;

    let (content_type, body) = multipart("a.torrent", TORRENT);
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/upload")
            .insert_header(("content-type", content_type))
            .insert_header(("Cookie", format!("_session_id={session}")))
            .set_payload(body)
            .to_request(),
    )
    .await;

    assert_eq!(response.status(), 200);
    let answered = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    assert!(answered.starts_with("text/html"), "{answered}");

    // And the body is still the JSON the dialog parses out of it.
    let raw = test::read_body(response).await;
    let parsed: Json = serde_json::from_slice(&raw).expect("the body is JSON");
    assert_eq!(parsed["success"], json!(true));
}

#[actix_web::test]
async fn a_refused_upload_also_answers_as_html() {
    // The failure path goes through the same iframe, so it needs the same
    // content type or the dialog shows its own message instead of ours.
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));

    let (content_type, body) = multipart("a.torrent", TORRENT);
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/upload")
            .insert_header(("content-type", content_type))
            .set_payload(body)
            .to_request(),
    )
    .await;

    let answered = response
        .headers()
        .get("content-type")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    assert!(answered.starts_with("text/html"), "{answered}");

    let raw = test::read_body(response).await;
    let parsed: Json = serde_json::from_slice(&raw).unwrap();
    assert_eq!(parsed["success"], json!(false));
}

#[actix_web::test]
async fn an_upload_that_is_not_a_torrent_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));
    let session = logged_in(&app).await;

    let (content_type, body) = multipart("a.torrent", b"this is not a torrent");
    let response = test::call_service(
        &app,
        test::TestRequest::post()
            .uri("/upload")
            .insert_header(("content-type", content_type))
            .insert_header(("Cookie", format!("_session_id={session}")))
            .set_payload(body)
            .to_request(),
    )
    .await;

    let answer: Json = serde_json::from_slice(&test::read_body(response).await).unwrap();
    assert_eq!(answer["success"], json!(false));
    assert_eq!(answer["error"], json!("not a torrent file"));
}

#[actix_web::test]
async fn a_file_outside_the_staging_area_cannot_be_read_back() {
    // The path comes from the browser, so without the check an authenticated
    // client could read anything the server can.
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));
    let session = logged_in(&app).await;

    let secret = dir.path().join("secret");
    std::fs::write(&secret, TORRENT).unwrap();

    for path in [secret.display().to_string(), "/etc/hostname".to_owned()] {
        let (body, _) = call(&app, "web.get_torrent_info", json!([path]), Some(&session)).await;
        assert_eq!(
            body["error"]["message"],
            json!("no such uploaded torrent"),
            "{path} should not be readable"
        );
    }
}

// -------------------------------------------------------------------- hosts

#[actix_web::test]
async fn a_host_can_be_added_edited_and_removed() {
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));
    let session = logged_in(&app).await;

    let (body, _) = call(
        &app,
        "web.add_host",
        json!(["127.0.0.1", 58846, "localclient", "secret"]),
        Some(&session),
    )
    .await;
    assert_eq!(body["error"], Json::Null, "{body}");
    assert_eq!(body["result"][0], json!(true));
    let id = body["result"][1].as_str().expect("an id").to_owned();

    let (body, _) = call(&app, "web.get_hosts", json!([]), Some(&session)).await;
    assert_eq!(body["result"].as_array().unwrap().len(), 1);

    let (body, _) = call(
        &app,
        "web.edit_host",
        json!([id, "10.0.0.1", 58847, "other", "pw"]),
        Some(&session),
    )
    .await;
    assert_eq!(body["result"], json!(true));

    let (body, _) = call(&app, "web.get_hosts", json!([]), Some(&session)).await;
    assert_eq!(body["result"][0][1], json!("10.0.0.1"));
    assert_eq!(body["result"][0][2], json!(58847));

    let (body, _) = call(&app, "web.remove_host", json!([id]), Some(&session)).await;
    assert_eq!(body["result"], json!(true));

    let (body, _) = call(&app, "web.get_hosts", json!([]), Some(&session)).await;
    assert!(body["result"].as_array().unwrap().is_empty());
}

#[actix_web::test]
async fn the_same_host_is_not_added_twice() {
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));
    let session = logged_in(&app).await;

    let params = json!(["127.0.0.1", 58846, "localclient", "secret"]);
    let (first, _) = call(&app, "web.add_host", params.clone(), Some(&session)).await;
    assert_eq!(first["error"], Json::Null);

    let (second, _) = call(&app, "web.add_host", params, Some(&session)).await;
    assert!(second["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .contains("already"));
}

#[actix_web::test]
async fn editing_a_host_that_is_not_there_is_an_error() {
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));
    let session = logged_in(&app).await;

    let (body, _) = call(
        &app,
        "web.edit_host",
        json!(["nope", "127.0.0.1", 58846, "", ""]),
        Some(&session),
    )
    .await;
    assert_eq!(body["error"]["message"], json!("no such host"));
}

#[actix_web::test]
async fn a_host_added_through_the_api_is_written_to_the_file() {
    // It has to survive a restart, which means the file and not just memory.
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));
    let session = logged_in(&app).await;

    call(
        &app,
        "web.add_host",
        json!(["127.0.0.1", 58846, "localclient", "secret"]),
        Some(&session),
    )
    .await;

    let written = std::fs::read_to_string(dir.path().join("hostlist.conf")).unwrap();
    assert!(written.contains("127.0.0.1"), "{written}");
    assert!(written.contains("58846"));
}

#[actix_web::test]
async fn the_daemon_is_not_started_by_the_web_ui() {
    // It is a service of its own under systemd or the container, so spawning
    // an unsupervised child here would be wrong. Refused with a reason.
    let dir = tempfile::tempdir().unwrap();
    let app = booted!(state(dir.path()));
    let session = logged_in(&app).await;

    let (body, _) = call(&app, "web.start_daemon", json!([58846]), Some(&session)).await;
    assert!(body["error"]["message"]
        .as_str()
        .unwrap_or_default()
        .contains("service of its own"));
}

// --------------------------------------------------------------------- flags

#[actix_web::test]
async fn a_peer_country_resolves_to_a_flag() {
    // The peers tab emits an `<img src="flag/xx">` per row. Without this route
    // every one of them was a broken image, which is what a missing asset
    // looks like when the renderer uses an element rather than a background.
    let dir = tempfile::tempdir().unwrap();
    let state = state(dir.path());
    let app = booted!(state);

    for path in ["/flag/fr", "/flag/FR"] {
        let response =
            test::call_service(&app, test::TestRequest::get().uri(path).to_request()).await;
        assert!(response.status().is_success(), "{path} should have a flag");
        assert_eq!(
            response
                .headers()
                .get("content-type")
                .and_then(|value| value.to_str().ok()),
            Some("image/png"),
            "{path} should answer as an image"
        );
    }
}

#[actix_web::test]
async fn a_country_code_cannot_name_another_file() {
    // The code arrives from the daemon, which read it out of a GeoIP database,
    // so it is data and is validated rather than pasted into a path.
    let dir = tempfile::tempdir().unwrap();
    let state = state(dir.path());
    let app = booted!(state);

    for path in [
        "/flag/..%2f..%2findex.html",
        "/flag/fra",
        "/flag/f",
        "/flag/zz",
        "/flag/%2e%2e",
    ] {
        let response =
            test::call_service(&app, test::TestRequest::get().uri(path).to_request()).await;
        assert_eq!(
            response.status(),
            actix_web::http::StatusCode::NOT_FOUND,
            "{path} should not resolve to anything"
        );
    }
}

// ------------------------------------------------------- the web configuration

#[actix_web::test]
async fn the_poll_interval_can_be_changed_through_the_api() {
    // The Interface page writes it here. It is clamped rather than trusted:
    // zero would spin the browser and an hour would look like the interface
    // had stopped.
    let dir = tempfile::tempdir().unwrap();
    let state = state(dir.path());
    let app = booted!(state);
    let session = logged_in(&app).await;

    for (asked, expected) in [(5000, 5000), (10, 500), (600_000, 60000)] {
        call(
            &app,
            "web.set_config",
            json!([{"poll_interval": asked}]),
            Some(&session),
        )
        .await;

        let (body, _) = call(&app, "web.get_config", json!([]), Some(&session)).await;
        assert_eq!(
            body["result"]["poll_interval"],
            json!(expected),
            "asking for {asked} should give {expected}"
        );
    }
}

#[actix_web::test]
async fn a_daemon_certificate_can_be_pinned_through_the_api() {
    // The connection manager's Edit window writes this. Without it a pin could
    // only be set by stopping the server and editing web.conf.
    let dir = tempfile::tempdir().unwrap();
    let state = state(dir.path());
    let app = booted!(state);
    let session = logged_in(&app).await;

    let (body, _) = call(&app, "web.get_config", json!([]), Some(&session)).await;
    assert_eq!(
        body["result"]["daemon_fingerprints"],
        json!({}),
        "a fresh configuration pins nothing"
    );

    let pin = "c91c7c0ee3f1ff89d5c7e1eaa5c76efcd95290edd7a84276bfa841921908010a";
    call(
        &app,
        "web.set_config",
        json!([{"daemon_fingerprints": {"abc123": pin}}]),
        Some(&session),
    )
    .await;

    let (body, _) = call(&app, "web.get_config", json!([]), Some(&session)).await;
    assert_eq!(body["result"]["daemon_fingerprints"]["abc123"], json!(pin));
}

#[actix_web::test]
async fn the_server_binding_still_cannot_be_changed_from_the_browser() {
    // The Interface page no longer offers these, and the server refuses them
    // in any case: a browser that could move the listener can lock everyone
    // out of it.
    let dir = tempfile::tempdir().unwrap();
    let state = state(dir.path());
    let app = booted!(state);
    let session = logged_in(&app).await;

    call(
        &app,
        "web.set_config",
        json!([{"port": 9999, "interface": "0.0.0.0", "https": true}]),
        Some(&session),
    )
    .await;

    let (body, _) = call(&app, "web.get_config", json!([]), Some(&session)).await;
    assert_ne!(body["result"]["port"], json!(9999));
    assert_eq!(body["result"]["https"], json!(false));
}

// ------------------------------------------------------------------- events

#[actix_web::test]
async fn asking_for_events_waits_rather_than_answering_empty_at_once() {
    // The front end asks again the instant it is answered. Answering empty
    // straight away turned that into about twenty requests a second for as
    // long as a tab was open; holding the answer is what makes the same loop
    // correct.
    let dir = tempfile::tempdir().unwrap();
    let state = state(dir.path());
    let app = booted!(state);
    let session = logged_in(&app).await;

    let started = std::time::Instant::now();
    // The wait is twenty-five seconds, so this is expected to time out. What
    // is being asserted is that it did not come back at once.
    let answered = tokio::time::timeout(
        Duration::from_secs(3),
        call(&app, "web.get_events", json!([]), Some(&session)),
    )
    .await
    .is_ok();

    assert!(
        !answered && started.elapsed() >= Duration::from_secs(2),
        "the poll came back after {:?}, which is a busy loop",
        started.elapsed()
    );
}

#[actix_web::test]
async fn an_event_ends_the_wait_instead_of_sitting_in_the_queue() {
    // Holding the answer must not delay an event: the whole point is that the
    // browser hears about one as soon as the daemon reports it.
    let dir = tempfile::tempdir().unwrap();
    let state = state(dir.path());
    let app = booted!(state);
    let session = logged_in(&app).await;

    call(
        &app,
        "web.register_event_listener",
        json!(["TorrentAddedEvent"]),
        Some(&session),
    )
    .await;

    let pusher = state.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        pusher
            .events
            .lock()
            .await
            .push("TorrentAddedEvent".to_owned(), vec![json!("abc")]);
        pusher.events_ready.notify_waiters();
    });

    let started = std::time::Instant::now();
    let (body, _) = call(&app, "web.get_events", json!([]), Some(&session)).await;

    assert_eq!(
        body["result"],
        json!([["TorrentAddedEvent", ["abc"]]]),
        "the event should have come back"
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the event waited {:?} instead of ending the poll",
        started.elapsed()
    );
}

// -------------------------------------------------------------- the Label plugin

#[actix_web::test]
async fn the_label_namespace_reaches_the_daemon() {
    // Radarr, Sonarr and the rest talk to this server, not to the daemon's own
    // port. A namespace the Web UI does not forward is a namespace they cannot
    // call, whatever the daemon answers.
    let dir = tempfile::tempdir().unwrap();
    let state = state(dir.path());
    let app = booted!(state);
    let session = logged_in(&app).await;

    for method in [
        "label.get_labels",
        "label.add",
        "label.remove",
        "label.set_torrent",
        "label.get_config",
        "label.set_config",
        "label.get_options",
        "label.set_options",
    ] {
        let (body, _) = call(&app, method, json!([]), Some(&session)).await;
        // No daemon is connected in this test, so the answer is a daemon
        // error. What must not happen is "Unknown method", which is the Web UI
        // refusing to pass it on.
        let message = body["error"]["message"].as_str().unwrap_or_default();
        assert!(
            !message.contains("Unknown method"),
            "{method} was not forwarded: {message}"
        );
    }
}

#[actix_web::test]
async fn the_web_ui_agrees_that_the_label_plugin_is_enabled() {
    // A client that asks what is enabled and then calls `label.*` has to get
    // an answer that agrees with what happens next.
    let dir = tempfile::tempdir().unwrap();
    let state = state(dir.path());
    let app = booted!(state);
    let session = logged_in(&app).await;

    let (body, _) = call(&app, "web.get_plugins", json!([]), Some(&session)).await;
    assert_eq!(body["result"]["enabled_plugins"], json!(["Label"]));
    assert_eq!(body["result"]["available_plugins"], json!(["Label"]));
}
