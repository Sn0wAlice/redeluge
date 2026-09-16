// SPDX-License-Identifier: GPL-3.0-or-later
// Flattening libtorrent alerts.
//
// libtorrent alerts are a class hierarchy read with alert_cast, which does not
// translate into Rust. Each one is flattened here into a plain struct with a
// discriminant, a message, an optional infohash and a few payload slots. The
// discriminants are the order contract/alerts.json lists them in, and a Rust
// test fails if the two drift.

#include "shim.h"

#include <stdexcept>
#include <vector>

#include <libtorrent/alert_types.hpp>
#include <libtorrent/write_resume_data.hpp>

namespace redeluge {
namespace {
// Discriminants for FlatAlert::kind. The order is the alphabetical order of
// contract/alerts.json, and a Rust test asserts the two stay in step. Adding an
// alert means appending here and in the contract, never renumbering.
enum Kind : uint16_t {
  kUnknown = 0,
  kAddTorrent = 1,
  kExternalIp = 2,
  kFastresumeRejected = 3,
  kFileCompleted = 4,
  kFileError = 5,
  kFileRenamed = 6,
  kMetadataReceived = 7,
  kPerformance = 8,
  kSaveResumeData = 9,
  kSaveResumeDataFailed = 10,
  kSessionStats = 11,
  kStateChanged = 12,
  kStateUpdate = 13,
  kStorageMoved = 14,
  kStorageMovedFailed = 15,
  kTorrentChecked = 16,
  kTorrentFinished = 17,
  kTorrentNeedCert = 18,
  kTorrentPaused = 19,
  kTorrentResumed = 20,
  kTrackerAnnounce = 21,
  kTrackerError = 22,
  kTrackerReply = 23,
  kTrackerWarning = 24,
};

// dynamic_cast, not alert_cast: alert_cast compares the concrete alert type id
// and so never matches a base class. Asking it for torrent_alert silently
// returns null for every torrent-scoped alert, which is how this was wrong the
// first time round.
std::string hash_of_alert(lt::alert const* alert) {
  if (auto const* scoped = dynamic_cast<lt::torrent_alert const*>(alert)) {
    if (scoped->handle.is_valid()) return hex_of(scoped->handle.info_hashes());
  }
  return std::string();
}

FlatAlert blank(lt::alert const* alert, Kind kind, char const* what) {
  FlatAlert flat{};
  flat.kind = static_cast<uint16_t>(kind);
  flat.what = rust::String(what);
  flat.message = rust::String(alert->message());
  flat.info_hash = rust::String(hash_of_alert(alert));
  flat.num_a = 0;
  flat.num_b = 0;
  flat.str_a = rust::String();
  flat.str_b = rust::String();
  flat.blob = rust::Vec<uint8_t>();
  flat.counters = rust::Vec<int64_t>();
  return flat;
}

// Maps one libtorrent alert onto the flat representation. Unrecognised alerts
// still cross with kind 0 and their rendered message, so an unexpected alert is
// visible in logs instead of silently dropped.
FlatAlert flatten(lt::alert const* alert) {
  if (auto const* a = lt::alert_cast<lt::add_torrent_alert>(alert)) {
    FlatAlert flat = blank(alert, kAddTorrent, "add_torrent");
    if (a->error) flat.str_a = rust::String(a->error.message());
    flat.num_a = a->error ? 1 : 0;
    return flat;
  }
  if (auto const* a = lt::alert_cast<lt::external_ip_alert>(alert)) {
    FlatAlert flat = blank(alert, kExternalIp, "external_ip");
    flat.str_a = rust::String(a->external_address.to_string());
    return flat;
  }
  if (lt::alert_cast<lt::fastresume_rejected_alert>(alert)) {
    return blank(alert, kFastresumeRejected, "fastresume_rejected");
  }
  if (auto const* a = lt::alert_cast<lt::file_completed_alert>(alert)) {
    FlatAlert flat = blank(alert, kFileCompleted, "file_completed");
    flat.num_a = static_cast<int64_t>(static_cast<int>(a->index));
    return flat;
  }
  if (auto const* a = lt::alert_cast<lt::file_error_alert>(alert)) {
    FlatAlert flat = blank(alert, kFileError, "file_error");
    flat.str_a = rust::String(a->filename());
    flat.str_b = rust::String(a->error.message());
    return flat;
  }
  if (auto const* a = lt::alert_cast<lt::file_renamed_alert>(alert)) {
    FlatAlert flat = blank(alert, kFileRenamed, "file_renamed");
    flat.num_a = static_cast<int64_t>(static_cast<int>(a->index));
    flat.str_a = rust::String(a->new_name());
    return flat;
  }
  if (lt::alert_cast<lt::metadata_received_alert>(alert)) {
    return blank(alert, kMetadataReceived, "metadata_received");
  }
  if (auto const* a = lt::alert_cast<lt::performance_alert>(alert)) {
    FlatAlert flat = blank(alert, kPerformance, "performance");
    flat.num_a = static_cast<int64_t>(a->warning_code);
    return flat;
  }
  if (auto const* a = lt::alert_cast<lt::save_resume_data_alert>(alert)) {
    FlatAlert flat = blank(alert, kSaveResumeData, "save_resume_data");
    // The daemon never looks inside this. libtorrent writes it, stores it, and
    // reads it back on restart, so it crosses as an opaque bencoded buffer and
    // Rust needs no bencode of its own.
    std::vector<char> const encoded = lt::write_resume_data_buf(a->params);
    flat.blob.reserve(encoded.size());
    for (char byte : encoded) flat.blob.push_back(static_cast<uint8_t>(byte));
    return flat;
  }
  if (auto const* a = lt::alert_cast<lt::save_resume_data_failed_alert>(alert)) {
    FlatAlert flat = blank(alert, kSaveResumeDataFailed, "save_resume_data_failed");
    flat.str_a = rust::String(a->error.message());
    return flat;
  }
  if (auto const* a = lt::alert_cast<lt::session_stats_alert>(alert)) {
    FlatAlert flat = blank(alert, kSessionStats, "session_stats");
    // The counters themselves, not just how many there are: this alert is the
    // only way to read them, and the Web UI's status bar is drawn from them.
    auto const counters = a->counters();
    flat.counters.reserve(counters.size());
    for (auto const value : counters) flat.counters.push_back(value);
    flat.num_a = static_cast<int64_t>(counters.size());
    return flat;
  }
  if (auto const* a = lt::alert_cast<lt::state_changed_alert>(alert)) {
    FlatAlert flat = blank(alert, kStateChanged, "state_changed");
    flat.num_a = static_cast<int64_t>(static_cast<int>(a->state));
    flat.num_b = static_cast<int64_t>(static_cast<int>(a->prev_state));
    return flat;
  }
  if (auto const* a = lt::alert_cast<lt::state_update_alert>(alert)) {
    FlatAlert flat = blank(alert, kStateUpdate, "state_update");
    flat.num_a = static_cast<int64_t>(a->status.size());
    return flat;
  }
  if (auto const* a = lt::alert_cast<lt::storage_moved_alert>(alert)) {
    FlatAlert flat = blank(alert, kStorageMoved, "storage_moved");
    flat.str_a = rust::String(a->storage_path());
    return flat;
  }
  if (auto const* a = lt::alert_cast<lt::storage_moved_failed_alert>(alert)) {
    FlatAlert flat = blank(alert, kStorageMovedFailed, "storage_moved_failed");
    flat.str_a = rust::String(a->error.message());
    return flat;
  }
  if (lt::alert_cast<lt::torrent_checked_alert>(alert)) {
    return blank(alert, kTorrentChecked, "torrent_checked");
  }
  if (lt::alert_cast<lt::torrent_finished_alert>(alert)) {
    return blank(alert, kTorrentFinished, "torrent_finished");
  }
  if (lt::alert_cast<lt::torrent_need_cert_alert>(alert)) {
    return blank(alert, kTorrentNeedCert, "torrent_need_cert");
  }
  if (lt::alert_cast<lt::torrent_paused_alert>(alert)) {
    return blank(alert, kTorrentPaused, "torrent_paused");
  }
  if (lt::alert_cast<lt::torrent_resumed_alert>(alert)) {
    return blank(alert, kTorrentResumed, "torrent_resumed");
  }
  if (auto const* a = lt::alert_cast<lt::tracker_announce_alert>(alert)) {
    FlatAlert flat = blank(alert, kTrackerAnnounce, "tracker_announce");
    flat.str_a = rust::String(a->tracker_url());
    return flat;
  }
  if (auto const* a = lt::alert_cast<lt::tracker_error_alert>(alert)) {
    FlatAlert flat = blank(alert, kTrackerError, "tracker_error");
    flat.str_a = rust::String(a->tracker_url());
    // The tracker's own words first, and the transport error only when it did
    // not answer at all. `error.message()` for a tracker that replied with a
    // failure reason is "tracker failure", which says nothing: the reason
    // itself is the difference between "come back later" and "I have never
    // heard of this torrent", and something has to be able to tell them apart.
    char const* reason = a->error_message();
    flat.str_b = rust::String((reason != nullptr && reason[0] != '\0')
                                  ? reason
                                  : a->error.message());
    flat.num_a = static_cast<int64_t>(a->times_in_row);
    return flat;
  }
  if (auto const* a = lt::alert_cast<lt::tracker_reply_alert>(alert)) {
    FlatAlert flat = blank(alert, kTrackerReply, "tracker_reply");
    flat.str_a = rust::String(a->tracker_url());
    flat.num_a = static_cast<int64_t>(a->num_peers);
    return flat;
  }
  if (auto const* a = lt::alert_cast<lt::tracker_warning_alert>(alert)) {
    FlatAlert flat = blank(alert, kTrackerWarning, "tracker_warning");
    flat.str_a = rust::String(a->tracker_url());
    // Carried for the same reason as the error above: some trackers say what
    // they think of a torrent here rather than in a failure reason.
    char const* warning = a->warning_message();
    flat.str_b = rust::String(warning != nullptr ? warning : "");
    return flat;
  }
  return blank(alert, kUnknown, alert->what());
}

}  // namespace

FlatAlert flatten_alert(lt::alert const* alert) { return flatten(alert); }

}  // namespace redeluge
