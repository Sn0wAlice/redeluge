// SPDX-License-Identifier: GPL-3.0-or-later
// Everything addressed by infohash.
//
// Handles are looked up in the session's table rather than crossing the
// boundary, so Rust never holds a C++ object it could outlive, and a call for a
// torrent that has gone away is an error rather than undefined behaviour.

#include "shim.h"

#include <stdexcept>
#include <vector>

#include <libtorrent/create_torrent.hpp>
#include <libtorrent/peer_info.hpp>
#include <libtorrent/torrent_flags.hpp>
#include <libtorrent/torrent_info.hpp>
#include <libtorrent/torrent_status.hpp>

namespace redeluge {
namespace {

/// Status fields the daemon reads. `query_pieces` is not in the default set and
/// the piece map is what the progress bar is drawn from.
constexpr lt::status_flags_t kStatusFlags =
    lt::torrent_handle::query_distributed_copies |
    lt::torrent_handle::query_accurate_download_counters |
    lt::torrent_handle::query_last_seen_complete | lt::torrent_handle::query_pieces |
    lt::torrent_handle::query_verified_pieces | lt::torrent_handle::query_name |
    lt::torrent_handle::query_save_path | lt::torrent_handle::query_torrent_file;

int64_t seconds_of(lt::seconds const value) { return value.count(); }

int64_t timestamp_of(std::time_t const value) {
  return static_cast<int64_t>(value);
}

TorrentStatus convert(lt::torrent_status const& st) {
  TorrentStatus out{};
  out.info_hash = rust::String(hex_of(st.info_hashes));
  out.name = rust::String(st.name);
  out.save_path = rust::String(st.save_path);
  out.state = static_cast<uint8_t>(static_cast<int>(st.state));
  out.progress = st.progress;
  out.flags = static_cast<uint64_t>(static_cast<std::uint64_t>(st.flags));

  out.download_rate = st.download_rate;
  out.upload_rate = st.upload_rate;
  out.download_payload_rate = st.download_payload_rate;
  out.upload_payload_rate = st.upload_payload_rate;

  out.num_peers = st.num_peers;
  out.num_seeds = st.num_seeds;
  out.num_complete = st.num_complete;
  out.num_incomplete = st.num_incomplete;
  out.connect_candidates = st.connect_candidates;

  out.total_done = st.total_done;
  out.total_wanted = st.total_wanted;
  out.total_wanted_done = st.total_wanted_done;
  out.total_payload_download = st.total_payload_download;
  out.total_payload_upload = st.total_payload_upload;
  out.all_time_download = st.all_time_download;
  out.all_time_upload = st.all_time_upload;

  out.active_time = seconds_of(st.active_duration);
  out.seeding_time = seconds_of(st.seeding_duration);
  out.time_since_download = seconds_of(st.active_duration - st.finished_duration);
  out.time_since_upload = seconds_of(st.seeding_duration);
  out.added_time = timestamp_of(st.added_time);
  out.completed_time = timestamp_of(st.completed_time);
  out.finished_time = seconds_of(st.finished_duration);
  out.last_seen_complete = timestamp_of(st.last_seen_complete);
  out.next_announce = seconds_of(
      std::chrono::duration_cast<lt::seconds>(st.next_announce));

  out.distributed_copies = st.distributed_copies;
  out.queue_position = static_cast<int32_t>(static_cast<int>(st.queue_position));
  out.seed_rank = st.seed_rank;
  out.storage_mode = static_cast<uint8_t>(static_cast<int>(st.storage_mode));

  out.is_finished = st.is_finished;
  out.is_seeding = st.is_seeding;
  out.is_paused = (st.flags & lt::torrent_flags::paused) != lt::torrent_flags_t{};
  out.has_metadata = st.has_metadata;
  out.moving_storage = st.moving_storage;

  out.current_tracker = rust::String(st.current_tracker);
  if (st.errc) {
    out.error = rust::String(st.errc.message());
    // error_file names a file when the error is about one, and carries a
    // sentinel otherwise; a negative index is not a file.
    int const index = static_cast<int>(st.error_file);
    if (index >= 0) {
      if (auto const info = st.torrent_file.lock()) {
        if (index < info->num_files()) {
          out.error_file = rust::String(info->files().file_path(st.error_file));
        }
      }
    }
  }

  out.pieces.reserve(st.pieces.size());
  for (auto const piece : st.pieces.range()) {
    out.pieces.push_back(st.pieces[piece] ? 1 : 0);
  }

  if (auto const info = st.torrent_file.lock()) {
    out.num_pieces = info->num_pieces();
    out.piece_length = info->piece_length();
    out.total_size = info->total_size();
    out.num_files = info->num_files();
  }
  return out;
}

lt::download_priority_t priority_of(uint8_t value) {
  // libtorrent takes 0 to 7; anything else is a caller bug, and clamping would
  // hide it until someone wondered why a file never downloaded.
  if (value > 7) throw std::runtime_error("file priority must be 0 to 7");
  return lt::download_priority_t{value};
}

}  // namespace

lt::torrent_handle const& Session::require(std::string const& info_hash) const {
  auto found = handles_.find(info_hash);
  if (found == handles_.end() || !found->second.is_valid()) {
    throw std::runtime_error("no such torrent: " + info_hash);
  }
  return found->second;
}

lt::torrent_handle& Session::require_mut(std::string const& info_hash) {
  auto found = handles_.find(info_hash);
  if (found == handles_.end() || !found->second.is_valid()) {
    throw std::runtime_error("no such torrent: " + info_hash);
  }
  return found->second;
}

TorrentStatus Session::torrent_status(rust::Str info_hash) const {
  return convert(require(to_string(info_hash)).status(kStatusFlags));
}

rust::Vec<TorrentStatus> Session::all_torrent_status() const {
  rust::Vec<TorrentStatus> out;
  out.reserve(handles_.size());
  for (auto const& entry : handles_) {
    if (!entry.second.is_valid()) continue;
    out.push_back(convert(entry.second.status(kStatusFlags)));
  }
  return out;
}

rust::Vec<rust::String> Session::torrent_hashes() const {
  rust::Vec<rust::String> out;
  out.reserve(handles_.size());
  for (auto const& entry : handles_) out.push_back(rust::String(entry.first));
  return out;
}

bool Session::is_valid(rust::Str info_hash) const {
  auto found = handles_.find(to_string(info_hash));
  return found != handles_.end() && found->second.is_valid();
}

void Session::pause_torrent(rust::Str info_hash) {
  require_mut(to_string(info_hash)).pause();
}

void Session::resume_torrent(rust::Str info_hash) {
  require_mut(to_string(info_hash)).resume();
}

void Session::remove_torrent(rust::Str info_hash, bool with_data) {
  std::string const key = to_string(info_hash);
  lt::torrent_handle const& handle = require(key);
  session_->remove_torrent(
      handle, with_data ? lt::session::delete_files : lt::remove_flags_t{});
  handles_.erase(key);
}

void Session::force_recheck(rust::Str info_hash) {
  require_mut(to_string(info_hash)).force_recheck();
}

void Session::force_reannounce(rust::Str info_hash, int32_t seconds) {
  require_mut(to_string(info_hash)).force_reannounce(seconds);
}

void Session::scrape_tracker(rust::Str info_hash) {
  require_mut(to_string(info_hash)).scrape_tracker();
}

void Session::clear_error(rust::Str info_hash) {
  require_mut(to_string(info_hash)).clear_error();
}

void Session::move_storage(rust::Str info_hash, rust::Str destination) {
  std::string const path = to_string(destination);
  if (path.empty()) throw std::runtime_error("a destination is required");
  require_mut(to_string(info_hash)).move_storage(path);
}

void Session::set_flags(rust::Str info_hash, uint64_t set, uint64_t unset) {
  lt::torrent_handle& handle = require_mut(to_string(info_hash));
  lt::torrent_flags_t const mask = lt::torrent_flags_t(set) | lt::torrent_flags_t(unset);
  // One call with a mask, so there is no window where the torrent has neither
  // state. For the paused flag that window means it briefly starts.
  handle.set_flags(lt::torrent_flags_t(set), mask);
}

void Session::set_max_connections(rust::Str info_hash, int32_t limit) {
  require_mut(to_string(info_hash)).set_max_connections(limit);
}

void Session::set_max_uploads(rust::Str info_hash, int32_t limit) {
  require_mut(to_string(info_hash)).set_max_uploads(limit);
}

void Session::set_download_limit(rust::Str info_hash, int32_t bytes_per_second) {
  require_mut(to_string(info_hash)).set_download_limit(bytes_per_second);
}

void Session::set_upload_limit(rust::Str info_hash, int32_t bytes_per_second) {
  require_mut(to_string(info_hash)).set_upload_limit(bytes_per_second);
}

int32_t Session::queue_position(rust::Str info_hash) const {
  return static_cast<int32_t>(
      static_cast<int>(require(to_string(info_hash)).queue_position()));
}

void Session::queue_top(rust::Str info_hash) {
  require_mut(to_string(info_hash)).queue_position_top();
}

void Session::queue_up(rust::Str info_hash) {
  require_mut(to_string(info_hash)).queue_position_up();
}

void Session::queue_down(rust::Str info_hash) {
  require_mut(to_string(info_hash)).queue_position_down();
}

void Session::queue_bottom(rust::Str info_hash) {
  require_mut(to_string(info_hash)).queue_position_bottom();
}

rust::Vec<FileEntry> Session::files(rust::Str info_hash) const {
  auto const info = require(to_string(info_hash)).torrent_file();
  rust::Vec<FileEntry> out;
  if (!info) return out;  // No metadata yet; an empty list, not an error.

  lt::file_storage const& storage = info->files();
  out.reserve(storage.num_files());
  for (lt::file_index_t index : storage.file_range()) {
    FileEntry entry{};
    entry.index = static_cast<int32_t>(static_cast<int>(index));
    entry.path = rust::String(storage.file_path(index));
    entry.size = storage.file_size(index);
    entry.offset = storage.file_offset(index);
    out.push_back(std::move(entry));
  }
  return out;
}

rust::Vec<int64_t> Session::file_progress(rust::Str info_hash) const {
  std::vector<std::int64_t> progress;
  require(to_string(info_hash)).file_progress(progress);

  rust::Vec<int64_t> out;
  out.reserve(progress.size());
  for (std::int64_t value : progress) out.push_back(value);
  return out;
}

rust::Vec<uint8_t> Session::file_priorities(rust::Str info_hash) const {
  auto const priorities = require(to_string(info_hash)).get_file_priorities();
  rust::Vec<uint8_t> out;
  out.reserve(priorities.size());
  for (auto priority : priorities) {
    out.push_back(static_cast<uint8_t>(static_cast<std::uint8_t>(priority)));
  }
  return out;
}

void Session::prioritize_files(rust::Str info_hash,
                               rust::Slice<uint8_t const> priorities) {
  std::vector<lt::download_priority_t> converted;
  converted.reserve(priorities.size());
  for (uint8_t priority : priorities) converted.push_back(priority_of(priority));
  require_mut(to_string(info_hash)).prioritize_files(converted);
}

void Session::rename_file(rust::Str info_hash, int32_t index, rust::Str new_name) {
  std::string const name = to_string(new_name);
  if (name.empty()) throw std::runtime_error("a new name is required");
  if (index < 0) throw std::runtime_error("file index must not be negative");
  require_mut(to_string(info_hash))
      .rename_file(lt::file_index_t{index}, name);
}

rust::Vec<uint8_t> Session::piece_priorities(rust::Str info_hash) const {
  auto const priorities = require(to_string(info_hash)).get_piece_priorities();
  rust::Vec<uint8_t> out;
  out.reserve(priorities.size());
  for (auto priority : priorities) {
    out.push_back(static_cast<uint8_t>(static_cast<std::uint8_t>(priority)));
  }
  return out;
}

void Session::prioritize_pieces(rust::Str info_hash,
                                rust::Slice<uint8_t const> priorities) {
  std::vector<lt::download_priority_t> converted;
  converted.reserve(priorities.size());
  for (uint8_t priority : priorities) converted.push_back(priority_of(priority));
  require_mut(to_string(info_hash)).prioritize_pieces(converted);
}

rust::Vec<int32_t> Session::piece_availability(rust::Str info_hash) const {
  std::vector<int> availability;
  require(to_string(info_hash)).piece_availability(availability);

  rust::Vec<int32_t> out;
  out.reserve(availability.size());
  for (int value : availability) out.push_back(value);
  return out;
}

rust::Vec<TrackerEntry> Session::trackers(rust::Str info_hash) const {
  rust::Vec<TrackerEntry> out;
  for (lt::announce_entry const& announce :
       require(to_string(info_hash)).trackers()) {
    TrackerEntry entry{};
    entry.url = rust::String(announce.url);
    entry.tier = announce.tier;

    // In 2.0 the per-announce state lives on the endpoints rather than the
    // entry. The daemon shows one line per tracker, so this reports the worst
    // endpoint: a tracker is only healthy when every endpoint reaches it.
    for (auto const& endpoint : announce.endpoints) {
      for (auto const& info : endpoint.info_hashes) {
        if (info.fails > entry.fails) {
          entry.fails = info.fails;
          if (!info.message.empty()) entry.message = rust::String(info.message);
          if (info.last_error && entry.message.empty()) {
            entry.message = rust::String(info.last_error.message());
          }
        }
        entry.verified = entry.verified || info.fails == 0;
        entry.updating = entry.updating || info.updating;
      }
    }
    out.push_back(std::move(entry));
  }
  return out;
}

void Session::replace_trackers(rust::Str info_hash,
                               rust::Slice<rust::String const> urls,
                               rust::Slice<uint8_t const> tiers) {
  if (!tiers.empty() && tiers.size() != urls.size()) {
    throw std::runtime_error("one tier per tracker, or none at all");
  }

  std::vector<lt::announce_entry> entries;
  entries.reserve(urls.size());
  for (std::size_t index = 0; index < urls.size(); ++index) {
    lt::announce_entry entry(std::string(urls[index]));
    entry.tier = tiers.empty() ? 0 : tiers[index];
    entries.push_back(std::move(entry));
  }
  require_mut(to_string(info_hash)).replace_trackers(entries);
}

void Session::add_tracker(rust::Str info_hash, rust::Str url, uint8_t tier) {
  std::string const address = to_string(url);
  if (address.empty()) throw std::runtime_error("a tracker url is required");

  lt::announce_entry entry(address);
  entry.tier = tier;
  require_mut(to_string(info_hash)).add_tracker(entry);
}

rust::Vec<PeerInfo> Session::peers(rust::Str info_hash) const {
  std::vector<lt::peer_info> peers;
  require(to_string(info_hash)).get_peer_info(peers);

  rust::Vec<PeerInfo> out;
  out.reserve(peers.size());
  for (lt::peer_info const& peer : peers) {
    PeerInfo entry{};
    entry.ip = rust::String(peer.ip.address().to_string());
    entry.port = peer.ip.port();
    entry.client = rust::String(peer.client);
    entry.peer_id = rust::String(hex_of(lt::sha1_hash(peer.pid.data())));
    entry.down_speed = peer.down_speed;
    entry.up_speed = peer.up_speed;
    entry.progress = peer.progress;
    entry.seed = (peer.flags & lt::peer_info::seed) != decltype(peer.flags){};
    out.push_back(std::move(entry));
  }
  return out;
}

void Session::connect_peer(rust::Str info_hash, rust::Str ip, uint16_t port) {
  lt::error_code ec;
  auto const address = lt::make_address(to_string(ip), ec);
  if (ec) throw std::runtime_error("not an ip address: " + ec.message());
  require_mut(to_string(info_hash))
      .connect_peer(lt::tcp::endpoint(address, port));
}

void Session::save_resume_data(rust::Str info_hash, bool flush_disk_cache) {
  lt::resume_data_flags_t flags{};
  if (flush_disk_cache) flags |= lt::torrent_handle::flush_disk_cache;
  require_mut(to_string(info_hash)).save_resume_data(flags);
}

bool Session::needs_resume_save(rust::Str info_hash) const {
  return require(to_string(info_hash)).need_save_resume_data();
}

rust::Vec<uint8_t> Session::torrent_file(rust::Str info_hash) const {
  auto const info = require(to_string(info_hash)).torrent_file();
  if (!info) throw std::runtime_error("this torrent has no metadata yet");

  // libtorrent keeps the parsed form, not the original bytes, so the file is
  // rebuilt. It is equivalent, not identical: a torrent with unusual extra keys
  // will not round-trip byte for byte.
  lt::create_torrent builder(*info);
  std::vector<char> encoded;
  lt::bencode(std::back_inserter(encoded), builder.generate());

  rust::Vec<uint8_t> out;
  out.reserve(encoded.size());
  for (char byte : encoded) out.push_back(static_cast<uint8_t>(byte));
  return out;
}

void Session::set_ssl_certificate(rust::Str info_hash,
                                  rust::Slice<uint8_t const> certificate,
                                  rust::Slice<uint8_t const> private_key,
                                  rust::Slice<uint8_t const> dh_params,
                                  rust::Str passphrase) {
  auto as_string = [](rust::Slice<uint8_t const> bytes) {
    return std::string(reinterpret_cast<char const*>(bytes.data()), bytes.size());
  };
  require_mut(to_string(info_hash))
      .set_ssl_certificate_buffer(as_string(certificate), as_string(private_key),
                                  as_string(dh_params));
  (void)passphrase;  // The buffer form takes no passphrase.
}

}  // namespace redeluge
