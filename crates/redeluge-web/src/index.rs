// SPDX-License-Identifier: GPL-3.0-or-later
//! The page itself.
//!
//! Which script bundle to serve is the one piece of logic here, and it is the
//! Python server's: a version containing `dev` asks for unbundled sources, a
//! `?debug=true` query asks for the unminified bundle, and otherwise the
//! minified bundle is used. Falling back when files are missing matters,
//! because an install without the build step has only the debug bundle.

use crate::assets;
use crate::template::{render, Context, Error};

/// Which set of scripts the page should load.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptSet {
    /// Minified bundles. What a release install serves.
    Normal,
    /// Concatenated but unminified. The fallback, and what `?debug=true` asks for.
    Debug,
}

impl ScriptSet {
    fn scripts(self) -> &'static [&'static str] {
        match self {
            // Real paths under the web root, so a plain static handler
            // resolves them. The Python server maps logical names onto these,
            // which buys nothing here.
            Self::Normal => &[
                "js/extjs/ext-base.js",
                "js/extjs/ext-all.js",
                "js/extjs/ext-extensions.js",
                "js/deluge-all.js",
            ],
            Self::Debug => &[
                "js/extjs/ext-base-debug.js",
                "js/extjs/ext-all-debug.js",
                "js/extjs/ext-extensions-debug.js",
                "js/deluge-all-debug.js",
            ],
        }
    }

    /// Whether every file this set needs was embedded.
    ///
    /// The minified bundles are produced by a step this build does not run yet,
    /// so in practice only the debug set is complete. Checking rather than
    /// assuming means adding minification later changes nothing here.
    pub fn available(self) -> bool {
        self.scripts().iter().all(|path| assets::contains(path))
    }
}

/// What makes one build's asset URLs different from another's.
///
/// The reported version is the same `2.2.1` for every build, so it cannot be
/// the key on its own. A digest over every embedded asset is, and it has to be
/// every one: this used to be the size of the JavaScript bundle alone, and the
/// same key then went on the URL of the stylesheets, the icons and the other
/// script bundle too. A fix confined to any of those left every URL unchanged,
/// so browsers kept serving the previous build's copy for the hour the cache
/// allows, and the fix appeared not to work.
fn cache_key(version: &str) -> String {
    use std::sync::OnceLock;

    static DIGEST: OnceLock<String> = OnceLock::new();
    let digest = DIGEST.get_or_init(|| {
        // Order matters, and `assets::files()` is a hash map, so the names are
        // sorted first: the same build must produce the same key every time.
        let mut names: Vec<&&str> = assets::files().keys().collect();
        names.sort_unstable();

        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for name in names {
            let bytes = assets::get(name).unwrap_or_default();
            for byte in name.as_bytes().iter().chain(bytes) {
                hash ^= u64::from(*byte);
                hash = hash.wrapping_mul(0x100_0000_01b3);
            }
        }
        format!("{hash:016x}")
    });

    format!("{version}-{digest}")
}

/// Picks the best script set that was actually embedded.
pub fn choose_scripts(debug_requested: bool) -> ScriptSet {
    let wanted = if debug_requested {
        ScriptSet::Debug
    } else {
        ScriptSet::Normal
    };
    if wanted.available() {
        return wanted;
    }

    let other = match wanted {
        ScriptSet::Normal => ScriptSet::Debug,
        ScriptSet::Debug => ScriptSet::Normal,
    };
    if other.available() {
        tracing::debug!(?wanted, ?other, "using the other script set");
        return other;
    }

    tracing::error!("no complete script set was embedded; the Web UI will not load");
    wanted
}

const STYLESHEETS: &[&str] = &[
    "css/ext-all-notheme.css",
    "css/ext-extensions.css",
    "css/deluge.css",
];

/// Renders `index.html` with the values the page asks for.
pub fn render_index(
    template: &str,
    base: &str,
    version: &str,
    theme: &str,
    js_config: &serde_json::Value,
    debug_requested: bool,
) -> Result<String, Error> {
    let scripts = choose_scripts(debug_requested);

    let mut stylesheets: Vec<String> = STYLESHEETS.iter().map(|name| (*name).to_owned()).collect();
    // The theme sheet is ordered last so it overrides the others, which is what
    // the Python server does and what the ExtJS themes assume.
    stylesheets.push(format!("themes/css/xtheme-{theme}.css"));

    let mut script_list = vec!["js/gettext.js".to_owned()];
    script_list.extend(scripts.scripts().iter().map(|name| (*name).to_owned()));

    // Every asset URL carries the build's version. The assets are compiled
    // into the binary and are cached for an hour, so without this an upgraded
    // server serves new JavaScript that browsers ignore until the cache
    // expires: the page runs the old front end against the new API and fails
    // in ways that look nothing like a caching problem.
    let stamp = cache_key(version);
    let stylesheets: Vec<String> = stylesheets
        .into_iter()
        .map(|path| format!("{path}?v={stamp}"))
        .collect();
    let script_list: Vec<String> = script_list
        .into_iter()
        .map(|path| format!("{path}?v={stamp}"))
        .collect();

    let context = Context::new()
        .set("version", version)
        .set("base", base)
        .set("js_config", js_config.to_string())
        .set("debug", if debug_requested { "true" } else { "false" })
        .list("stylesheets", stylesheets)
        .list("scripts", script_list);

    render(template, &context)
}
