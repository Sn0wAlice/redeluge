// SPDX-License-Identifier: GPL-3.0-or-later
//! The HTTP routes: the page, the rendered fragments, and the assets.
//!
//! In the library rather than the binary so a test can boot the same
//! application the binary serves. A route that only exists in `main.rs` is a
//! route nothing can exercise except by hand.

use actix_web::http::header;
use actix_web::{web, HttpRequest, HttpResponse};

use crate::state::SharedState;
use crate::{assets, index, json_api, upload};

/// The theme every install has, and what an unknown one falls back to.
pub const DEFAULT_THEME: &str = "dark";

/// Registers every route. The binary and the tests both call this.
pub fn configure(cfg: &mut web::ServiceConfig) {
    cfg.route("/json", web::post().to(json_api::handle))
        .route("/upload", web::post().to(upload::handle))
        .route("/", web::get().to(serve_index))
        .route("/render/{name}", web::get().to(serve_render))
        .route("/flag/{code}", web::get().to(serve_flag))
        // Anything else is an embedded asset, or the page again so the front
        // end's own routing works on a reload.
        .default_service(web::get().to(serve_asset));
}

/// Renders the page. Everything the browser does after this is a JSON call.
async fn serve_index(request: HttpRequest, state: web::Data<SharedState>) -> HttpResponse {
    let state = state.get_ref();

    let Some(raw) = assets::get("index.html") else {
        return HttpResponse::InternalServerError().body("Web UI assets are missing");
    };
    let template = String::from_utf8_lossy(raw);

    let debug = request
        .query_string()
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .any(|(key, value)| key == "debug" && matches!(value, "true" | "yes" | "on" | "1"));

    // A stored theme with no stylesheet renders the page unstyled with nothing
    // to point at. `web.set_theme` refuses one now, but a configuration
    // written before it did still holds one, so the page repairs itself.
    let theme = json_api::current_theme(state).await;

    // Only the keys the page's inline script reads; the rest arrive from
    // web.get_config once ExtJS is running.
    let js_config = serde_json::json!({
        "theme": theme,
        "sidebar_show_zero": false,
        "sidebar_multiple_filters": true,
        "show_session_speed": false,
        "base": state.settings.base,
        // The fork's own version, which since 1.6.0 is also what the daemon
        // reports to clients. Shown in the About window.
        "redeluge_version": env!("CARGO_PKG_VERSION"),
        "first_login": false,
        // The poll loop reads this before `web.get_config` has answered, so it
        // has to be in the page rather than only in that call.
        "poll_interval": json_api::poll_interval(state).await,
    });

    match index::render_index(
        &template,
        &state.settings.base,
        &state.settings.version,
        &theme,
        &js_config,
        debug,
    ) {
        Ok(html) => HttpResponse::Ok()
            .content_type("text/html; charset=utf-8")
            .body(html),
        Err(err) => {
            tracing::error!(error = %err, "could not render the index template");
            HttpResponse::InternalServerError().body("Could not render the Web UI")
        }
    }
}

/// Serves a `render/` template.
///
/// These are HTML fragments the front end fetches and drops into the page. They
/// carry `${_("...")}` markers, which the Python server resolved through Mako
/// and gettext on every request; English only makes that substitution the
/// identity, but it still has to happen or the markers reach the browser.
async fn serve_render(path: web::Path<String>, state: web::Data<SharedState>) -> HttpResponse {
    let name = path.into_inner();
    // No path traversal: the name indexes the embedded set and nothing else.
    let known = assets::contains(&format!("render/{name}"));
    let (file, status) = if known {
        (format!("render/{name}"), actix_web::http::StatusCode::OK)
    } else {
        (
            "render/404.html".to_owned(),
            actix_web::http::StatusCode::NOT_FOUND,
        )
    };

    let Some(raw) = assets::get(&file) else {
        return HttpResponse::NotFound().body("not found");
    };

    let context =
        crate::template::Context::new().set("version", state.get_ref().settings.version.clone());

    match crate::template::render(&String::from_utf8_lossy(raw), &context) {
        Ok(html) => HttpResponse::build(status)
            .insert_header((header::CONTENT_TYPE, "text/html; charset=utf-8"))
            .body(html),
        Err(err) => {
            tracing::error!(file, error = %err, "could not render a template");
            HttpResponse::InternalServerError().body("template error")
        }
    }
}

/// Serves the flag for a peer's country.
///
/// The peers tab asks for `flag/<code>` per row, which is Deluge's own URL, so
/// a client written against the Python server keeps working. Without this
/// route every peer with a country rendered a broken image, because the
/// renderer emits an `<img>` rather than a background.
///
/// The code is validated rather than pasted into a path: it arrives from the
/// daemon, which reads it out of a GeoIP database, and a path segment built
/// from data is how a traversal starts. Two ASCII letters is the whole of
/// ISO 3166-1 alpha-2.
async fn serve_flag(path: web::Path<String>) -> HttpResponse {
    let code = path.into_inner().to_ascii_lowercase();
    let valid = code.len() == 2 && code.bytes().all(|byte| byte.is_ascii_lowercase());

    let bytes = if valid {
        assets::get(&format!("flags/{code}.png"))
    } else {
        None
    };

    match bytes {
        Some(bytes) => HttpResponse::Ok()
            .insert_header((header::CONTENT_TYPE, "image/png"))
            .insert_header((header::CACHE_CONTROL, "public, max-age=86400"))
            .body(bytes),
        // A country with no flag is normal: the database knows codes the set
        // does not cover. Answering with the page would put HTML in an `<img>`.
        None => HttpResponse::NotFound()
            .insert_header((header::CONTENT_TYPE, "text/plain; charset=utf-8"))
            .body("no flag"),
    }
}

/// Serves one embedded asset, or the page when the path names no asset.
async fn serve_asset(request: HttpRequest, state: web::Data<SharedState>) -> HttpResponse {
    let base = &state.get_ref().settings.base;
    let path = request.path();
    // Strip the base prefix so the same asset resolves under a subpath.
    let relative = path
        .strip_prefix(base.as_str())
        .unwrap_or_else(|| path.trim_start_matches('/'));

    match assets::get(relative) {
        Some(bytes) => HttpResponse::Ok()
            .insert_header((header::CONTENT_TYPE, assets::content_type(relative)))
            // The assets are versioned with the binary, so a long cache is safe
            // and saves the browser a few hundred requests per page load.
            .insert_header((header::CACHE_CONTROL, "public, max-age=3600"))
            .body(bytes),
        // A path that looks like a file and names no asset is a 404. Answering
        // it with the page means the browser is handed HTML where it asked for
        // a stylesheet or a script, which it reports as a parse error rather
        // than as a missing file, and the page renders unstyled with no
        // obvious cause.
        None if looks_like_a_file(relative) => HttpResponse::NotFound()
            .insert_header((header::CONTENT_TYPE, "text/plain; charset=utf-8"))
            .body("not found"),
        // Anything else is a front-end route, and reloading one has to work.
        None => serve_index(request, state).await,
    }
}

/// Whether a path is asking for a file rather than for a front-end route.
///
/// The last segment having an extension this server knows how to serve is the
/// test. `/torrents` is a route; `/themes/css/xtheme-nope.css` is a file that
/// is not there.
fn looks_like_a_file(path: &str) -> bool {
    let last = path.rsplit('/').next().unwrap_or("");
    match last.rsplit_once('.') {
        Some((_, extension)) => !extension.is_empty() && extension.len() <= 5,
        None => false,
    }
}
