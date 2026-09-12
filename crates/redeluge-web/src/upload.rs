// SPDX-License-Identifier: GPL-3.0-or-later
//! `POST /upload`: the add-by-file dialog.
//!
//! The only part of the interface that is not a JSON-RPC call. The dialog
//! posts a multipart form, this writes each part into the staging directory
//! and answers with the paths, and the dialog then asks
//! `web.get_torrent_info` about each one and finally `web.add_torrents`.
//!
//! Three things are deliberate. The answer is always HTTP 200 with a `success`
//! field, because ExtJS's form submit treats any other status as a transport
//! failure and shows its own message instead of ours. The files land under the
//! configuration directory rather than in `/tmp`: the daemon is another
//! process, possibly in another namespace, and it reads these by path.
//!
//! And the content type is `text/html`, not `application/json`, which looks
//! wrong and is not. A form with `fileUpload: true` is submitted through a
//! hidden iframe, because XMLHttpRequest could not send a file when ExtJS 3
//! was written. The browser parses the response to build that iframe's
//! document, and only `text/html` makes it insert the body unchanged where
//! ExtJS can read it back. Answering `application/json` makes every upload
//! fail with "Failed to upload torrent" and nothing in the log. The Python
//! server did the same thing for the same reason.

use actix_multipart::Multipart;
use actix_web::{web, HttpResponse};
use futures_util::StreamExt;
use serde_json::json;

use crate::state::SharedState;
use crate::torrentfile;

/// The largest torrent this will accept, per file.
///
/// A torrent of a large collection is a few megabytes; ten is generous. The
/// cap matters because this runs before anything has looked at the content.
const MAX_TORRENT: usize = 10 * 1024 * 1024;

/// How many files one post may carry.
const MAX_FILES: usize = 64;

/// The answer, in the shape and content type the iframe upload needs.
fn answer(body: serde_json::Value) -> HttpResponse {
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(body.to_string())
}

pub async fn handle(
    request: actix_web::HttpRequest,
    mut payload: Multipart,
    state: web::Data<SharedState>,
) -> HttpResponse {
    let state = state.get_ref().clone();

    // The same session cookie as every other call. Without this an unattended
    // Web UI would accept file writes from anyone who could reach the port.
    if !crate::json_api::is_authenticated(&request, &state).await {
        return answer(json!({
            "success": false,
            "error": "not authenticated",
        }));
    }

    let staging = torrentfile::staging_dir(&state.settings.config_dir);
    if let Err(err) = tokio::fs::create_dir_all(&staging).await {
        tracing::error!(error = %err, path = %staging.display(),
            "could not create the upload directory");
        return answer(json!({"success": false, "error": "no staging directory"}));
    }

    let mut written = Vec::new();
    while let Some(field) = payload.next().await {
        let mut field = match field {
            Ok(field) => field,
            Err(err) => {
                tracing::warn!(error = %err, "malformed upload");
                return answer(json!({"success": false, "error": "malformed upload"}));
            }
        };

        if written.len() >= MAX_FILES {
            return answer(json!({"success": false, "error": "too many files"}));
        }

        let name = field
            .content_disposition()
            .and_then(|disposition| disposition.get_filename())
            .map(torrentfile::safe_name)
            .unwrap_or_else(|| "upload.torrent".to_owned());

        let mut bytes: Vec<u8> = Vec::new();
        while let Some(chunk) = field.next().await {
            let chunk = match chunk {
                Ok(chunk) => chunk,
                Err(err) => {
                    tracing::warn!(error = %err, "upload stopped early");
                    return answer(json!({"success": false, "error": "upload failed"}));
                }
            };
            if bytes.len() + chunk.len() > MAX_TORRENT {
                return answer(json!({"success": false, "error": "torrent file too large"}));
            }
            bytes.extend_from_slice(&chunk);
        }

        if bytes.is_empty() {
            continue;
        }
        // Parsed before it is written, so a file that is not a torrent is
        // refused here rather than at the next step with a worse message.
        if let Err(err) = torrentfile::parse(&bytes) {
            tracing::warn!(name, error = %err, "refused an upload that is not a torrent");
            return answer(json!({"success": false, "error": "not a torrent file"}));
        }

        let path = unique_path(&staging, &name);
        if let Err(err) = tokio::fs::write(&path, &bytes).await {
            tracing::error!(error = %err, path = %path.display(), "could not write an upload");
            return answer(json!({"success": false, "error": "could not store the file"}));
        }
        written.push(path.display().to_string());
    }

    if written.is_empty() {
        return answer(json!({"success": false, "error": "no files"}));
    }

    tracing::info!(count = written.len(), "torrent files uploaded");
    answer(json!({"success": true, "files": written}))
}

/// A path that does not already exist, so two uploads of the same name do not
/// overwrite one another while the dialog still holds both.
fn unique_path(staging: &std::path::Path, name: &str) -> std::path::PathBuf {
    let candidate = staging.join(name);
    if !candidate.exists() {
        return candidate;
    }
    for suffix in 1..1000 {
        let candidate = staging.join(format!("{suffix}-{name}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    staging.join(format!("{}-{name}", std::process::id()))
}

/// Deletes staged files that nothing came back for.
///
/// The dialog leaves a file behind whenever it is cancelled after the upload,
/// and nothing else ever removes them.
pub async fn sweep_staging(config_dir: &std::path::Path, older_than: std::time::Duration) -> usize {
    let staging = torrentfile::staging_dir(config_dir);
    let Ok(mut entries) = tokio::fs::read_dir(&staging).await else {
        return 0;
    };

    let mut removed = 0;
    while let Ok(Some(entry)) = entries.next_entry().await {
        let Ok(metadata) = entry.metadata().await else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let stale = metadata
            .modified()
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .map(|age| age > older_than)
            .unwrap_or(false);

        if stale && tokio::fs::remove_file(entry.path()).await.is_ok() {
            removed += 1;
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_second_upload_of_the_same_name_gets_its_own_path() {
        // Both are in the dialog at once, so the second must not overwrite the
        // first before either has been added.
        let dir = tempfile::tempdir().unwrap();
        let first = unique_path(dir.path(), "a.torrent");
        std::fs::write(&first, b"x").unwrap();

        let second = unique_path(dir.path(), "a.torrent");
        assert_ne!(first, second);
        assert!(second.display().to_string().ends_with("1-a.torrent"));
    }

    #[tokio::test]
    async fn sweeping_removes_what_was_left_behind_and_keeps_what_is_fresh() {
        let dir = tempfile::tempdir().unwrap();
        let staging = torrentfile::staging_dir(dir.path());
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("fresh.torrent"), b"x").unwrap();

        assert_eq!(
            sweep_staging(dir.path(), Duration::from_secs(3600)).await,
            0
        );
        assert!(staging.join("fresh.torrent").exists());

        assert_eq!(sweep_staging(dir.path(), Duration::ZERO).await, 1);
        assert!(!staging.join("fresh.torrent").exists());
    }

    #[tokio::test]
    async fn sweeping_a_directory_that_is_not_there_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(sweep_staging(dir.path(), Duration::ZERO).await, 0);
    }
}
