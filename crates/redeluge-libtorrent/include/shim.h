// SPDX-License-Identifier: GPL-3.0-or-later
// C++ side of the redeluge FFI boundary.
//
// The one rule here: nothing from libtorrent's type system crosses into Rust.
// Alerts are flattened, handles are looked up by hex infohash, containers are
// copied into rust::Vec, and errors are thrown as std::runtime_error which cxx
// turns into Result::Err.
//
// The implementation is split in three: shim.cc owns the session and its
// settings, shim_torrent.cc everything addressed by infohash, and shim_alert.cc
// the alert flattening.

#pragma once

#include <map>
#include <memory>
#include <string>

#include <libtorrent/session.hpp>
#include <libtorrent/torrent_handle.hpp>

#include "rust/cxx.h"

namespace redeluge {
// Declared before the generated header is pulled in: cxx emits
// `using Session = ::redeluge::Session;`, and that header includes this one,
// so the name has to exist by then. A declaration is enough for the alias.
class Session;
}  // namespace redeluge

#include "redeluge-libtorrent/src/bridge.rs.h"

namespace redeluge {

/// Turns one libtorrent alert into the flat representation. Defined in
/// shim_alert.cc.
FlatAlert flatten_alert(lt::alert const* alert);

/// Hex of a v1 infohash, or of a truncated v2 one for a v2-only torrent.
std::string hex_of(lt::sha1_hash const& hash);
std::string hex_of(lt::info_hash_t const& hashes);

std::string to_string(rust::Str value);

class Session {
 public:
  explicit Session(SessionConfig const& config);
  ~Session();

  Session(Session const&) = delete;
  Session& operator=(Session const&) = delete;

  // ------------------------------------------------------------------ session

  void apply_settings(rust::Slice<SettingValue const> settings);
  int64_t setting_int(rust::Str name) const;
  bool setting_bool(rust::Str name) const;
  rust::String setting_str(rust::Str name) const;

  void post_session_stats();

  bool is_listening() const;

  void set_ip_filter(rust::Slice<IpRange const> rules);
  int64_t ip_filter_ranges() const;
  uint16_t listen_port() const;

  rust::String add_torrent(AddTorrentRequest const& request);
  rust::String add_magnet(rust::Str uri, rust::Str save_path);
  rust::String add_torrent_from_resume(rust::Slice<uint8_t const> resume_data,
                                       rust::Str save_path);

  bool wait_for_alert(int32_t millis);
  rust::Vec<FlatAlert> pop_alerts();

  // ----------------------------------------------------------------- torrents

  TorrentStatus torrent_status(rust::Str info_hash) const;
  rust::Vec<TorrentStatus> all_torrent_status() const;
  rust::Vec<rust::String> torrent_hashes() const;
  bool is_valid(rust::Str info_hash) const;

  void pause_torrent(rust::Str info_hash);
  void resume_torrent(rust::Str info_hash);
  void remove_torrent(rust::Str info_hash, bool with_data);
  void force_recheck(rust::Str info_hash);
  void force_reannounce(rust::Str info_hash, int32_t seconds);
  void scrape_tracker(rust::Str info_hash);
  void clear_error(rust::Str info_hash);
  void move_storage(rust::Str info_hash, rust::Str destination);
  void set_flags(rust::Str info_hash, uint64_t set, uint64_t unset);

  void set_max_connections(rust::Str info_hash, int32_t limit);
  void set_max_uploads(rust::Str info_hash, int32_t limit);
  void set_download_limit(rust::Str info_hash, int32_t bytes_per_second);
  void set_upload_limit(rust::Str info_hash, int32_t bytes_per_second);

  int32_t queue_position(rust::Str info_hash) const;
  void queue_top(rust::Str info_hash);
  void queue_up(rust::Str info_hash);
  void queue_down(rust::Str info_hash);
  void queue_bottom(rust::Str info_hash);

  // -------------------------------------------------------------------- files

  rust::Vec<FileEntry> files(rust::Str info_hash) const;
  rust::Vec<int64_t> file_progress(rust::Str info_hash) const;
  rust::Vec<uint8_t> file_priorities(rust::Str info_hash) const;
  void prioritize_files(rust::Str info_hash, rust::Slice<uint8_t const> priorities);
  void rename_file(rust::Str info_hash, int32_t index, rust::Str new_name);

  rust::Vec<uint8_t> piece_priorities(rust::Str info_hash) const;
  void prioritize_pieces(rust::Str info_hash, rust::Slice<uint8_t const> priorities);
  rust::Vec<int32_t> piece_availability(rust::Str info_hash) const;

  // ----------------------------------------------------------- trackers, peers

  rust::Vec<TrackerEntry> trackers(rust::Str info_hash) const;
  void replace_trackers(rust::Str info_hash, rust::Slice<rust::String const> urls,
                        rust::Slice<uint8_t const> tiers);
  void add_tracker(rust::Str info_hash, rust::Str url, uint8_t tier);

  rust::Vec<PeerInfo> peers(rust::Str info_hash) const;
  void connect_peer(rust::Str info_hash, rust::Str ip, uint16_t port);

  // ------------------------------------------------------------ resume, meta

  void save_resume_data(rust::Str info_hash, bool flush_disk_cache);
  bool needs_resume_save(rust::Str info_hash) const;
  rust::Vec<uint8_t> torrent_file(rust::Str info_hash) const;

  void set_ssl_certificate(rust::Str info_hash, rust::Slice<uint8_t const> certificate,
                           rust::Slice<uint8_t const> private_key,
                           rust::Slice<uint8_t const> dh_params, rust::Str passphrase);

 private:
  lt::torrent_handle const& require(std::string const& info_hash) const;
  /// Non-const because every mutating operation needs a mutable handle, and the
  /// table is the only thing that owns them.
  lt::torrent_handle& require_mut(std::string const& info_hash);

  std::unique_ptr<lt::session> session_;
  // Handles are cheap value types and stay valid across calls, so the table is
  // the cheapest way to answer "which torrent is this hex string".
  std::map<std::string, lt::torrent_handle> handles_;
};

std::unique_ptr<Session> new_session(SessionConfig const& config);
rust::String libtorrent_version();
rust::Vec<rust::String> session_stat_names();
rust::String torrent_file_info_hash(rust::Slice<uint8_t const> torrent_file);
rust::Vec<uint8_t> create_torrent(rust::Str path, int32_t piece_length, rust::Str comment,
                                  rust::Str creator, bool private_torrent,
                                  rust::Slice<rust::String const> trackers,
                                  rust::Slice<rust::String const> web_seeds,
                                  HashProgress& progress);

}  // namespace redeluge
