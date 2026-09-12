// SPDX-License-Identifier: GPL-3.0-or-later
// The session: settings, adding torrents, draining alerts.

#include "shim.h"

#include <stdexcept>
#include <tuple>
#include <algorithm>
#include <vector>

#include <libtorrent/alert_types.hpp>
#include <libtorrent/ip_filter.hpp>
#include <libtorrent/magnet_uri.hpp>
#include <libtorrent/read_resume_data.hpp>
#include <libtorrent/session_params.hpp>
#include <libtorrent/session_stats.hpp>
#include <libtorrent/settings_pack.hpp>
#include <libtorrent/torrent_flags.hpp>
#include <libtorrent/torrent_info.hpp>
#include <libtorrent/create_torrent.hpp>
#include <libtorrent/version.hpp>

namespace redeluge {

std::string to_string(rust::Str value) {
  return std::string(value.data(), value.size());
}

std::string hex_of(lt::sha1_hash const& hash) {
  static char const* digits = "0123456789abcdef";
  auto const* bytes = reinterpret_cast<unsigned char const*>(hash.data());
  std::string out;
  out.reserve(hash.size() * 2);
  for (int i = 0; i < hash.size(); ++i) {
    out.push_back(digits[bytes[i] >> 4]);
    out.push_back(digits[bytes[i] & 0x0f]);
  }
  return out;
}

// A v2-only torrent has no v1 hash. Deluge keys everything on the 40-character
// v1 hex string, so the truncated v2 hash stands in, which is what libtorrent
// itself does for backwards compatibility.
std::string hex_of(lt::info_hash_t const& hashes) {
  if (hashes.has_v1()) return hex_of(hashes.v1);
  if (hashes.has_v2()) return hex_of(lt::sha1_hash(hashes.v2.data()));
  return std::string();
}

namespace {

/// Setting kinds as the bridge tags them.
constexpr uint8_t kInt = 0;
constexpr uint8_t kBool = 1;
constexpr uint8_t kStr = 2;

/// libtorrent encodes a setting's type in its index. This recovers it.
uint8_t kind_of(int index) {
  switch (index & lt::settings_pack::type_mask) {
    case lt::settings_pack::string_type_base:
      return kStr;
    case lt::settings_pack::bool_type_base:
      return kBool;
    default:
      return kInt;
  }
}

int require_setting(std::string const& name, uint8_t expected) {
  int const index = lt::setting_by_name(name);
  if (index < 0) throw std::runtime_error("unknown session setting: " + name);

  uint8_t const actual = kind_of(index);
  if (actual != expected) {
    static char const* names[] = {"int", "bool", "string"};
    throw std::runtime_error("session setting " + name + " is a " +
                             names[actual] + ", not a " + names[expected]);
  }
  return index;
}

std::shared_ptr<lt::torrent_info> parse_torrent_bytes(uint8_t const* data,
                                                      std::size_t size) {
  if (size == 0) throw std::runtime_error("torrent file is empty");

  lt::error_code ec;
  auto info = std::make_shared<lt::torrent_info>(
      lt::span<char const>(reinterpret_cast<char const*>(data),
                           static_cast<std::ptrdiff_t>(size)),
      ec, lt::from_span);
  if (ec) throw std::runtime_error("unreadable torrent file: " + ec.message());
  return info;
}

}  // namespace

Session::Session(SessionConfig const& config) {
  lt::settings_pack pack;
  pack.set_str(lt::settings_pack::listen_interfaces,
               std::string(config.listen_interfaces));
  pack.set_str(lt::settings_pack::user_agent, std::string(config.user_agent));
  pack.set_bool(lt::settings_pack::enable_dht, config.enable_dht);
  pack.set_bool(lt::settings_pack::enable_lsd, config.enable_lsd);
  pack.set_bool(lt::settings_pack::enable_upnp, config.enable_upnp);
  pack.set_bool(lt::settings_pack::enable_natpmp, config.enable_natpmp);
  pack.set_int(lt::settings_pack::alert_queue_size, config.alert_queue_size);

  // The same categories the Python AlertManager subscribes to. Subscribing to
  // more would work but floods the queue; subscribing to fewer silently drops
  // alerts the daemon depends on.
  pack.set_int(lt::settings_pack::alert_mask,
               lt::alert_category::error | lt::alert_category::port_mapping |
                   lt::alert_category::storage | lt::alert_category::tracker |
                   lt::alert_category::status | lt::alert_category::ip_block |
                   lt::alert_category::performance_warning |
                   lt::alert_category::file_progress);

  session_ = std::make_unique<lt::session>(lt::session_params(pack));
}

Session::~Session() = default;

void Session::apply_settings(rust::Slice<SettingValue const> settings) {
  lt::settings_pack pack;
  for (SettingValue const& setting : settings) {
    std::string const name(setting.name);
    int const index = require_setting(name, setting.kind);

    switch (setting.kind) {
      case kInt:
        // libtorrent settings are 32-bit; a value that does not fit would wrap
        // silently into something the operator did not ask for.
        if (setting.int_value < std::numeric_limits<int>::min() ||
            setting.int_value > std::numeric_limits<int>::max()) {
          throw std::runtime_error("value out of range for setting " + name);
        }
        pack.set_int(index, static_cast<int>(setting.int_value));
        break;
      case kBool:
        pack.set_bool(index, setting.bool_value);
        break;
      default:
        pack.set_str(index, std::string(setting.str_value));
        break;
    }
  }
  session_->apply_settings(std::move(pack));
}

int64_t Session::setting_int(rust::Str name) const {
  std::string const key = to_string(name);
  return session_->get_settings().get_int(require_setting(key, kInt));
}

bool Session::setting_bool(rust::Str name) const {
  std::string const key = to_string(name);
  return session_->get_settings().get_bool(require_setting(key, kBool));
}

rust::String Session::setting_str(rust::Str name) const {
  std::string const key = to_string(name);
  return rust::String(session_->get_settings().get_str(require_setting(key, kStr)));
}

void Session::post_session_stats() { session_->post_session_stats(); }

bool Session::is_listening() const { return session_->is_listening(); }

uint16_t Session::listen_port() const {
  int const port = session_->listen_port();
  return port > 0 ? static_cast<uint16_t>(port) : 0;
}

namespace {

lt::address parse_address(rust::Str text) {
  lt::error_code ec;
  lt::address const address = lt::make_address(to_string(text), ec);
  if (ec) {
    throw std::runtime_error("not an IP address: " + to_string(text));
  }
  return address;
}

}  // namespace

void Session::set_ip_filter(rust::Slice<IpRange const> rules) {
  lt::ip_filter filter;
  for (IpRange const& rule : rules) {
    lt::address const first = parse_address(rule.first);
    lt::address const last = parse_address(rule.last);
    // add_rule asserts on a mixed pair rather than reporting it, and an assert
    // in a release build is undefined behaviour, so it is checked here.
    if (first.is_v4() != last.is_v4()) {
      throw std::runtime_error("range mixes IPv4 and IPv6: " + to_string(rule.first) +
                               " - " + to_string(rule.last));
    }
    if (last < first) {
      throw std::runtime_error("range ends before it starts: " + to_string(rule.first) +
                               " - " + to_string(rule.last));
    }
    filter.add_rule(first, last,
                    rule.blocked ? lt::ip_filter::blocked : 0);
  }
  session_->set_ip_filter(filter);
}

int64_t Session::ip_filter_ranges() const {
  auto const exported = session_->get_ip_filter().export_filter();
  return static_cast<int64_t>(std::get<0>(exported).size() + std::get<1>(exported).size());
}

rust::String Session::add_torrent(AddTorrentRequest const& request) {
  lt::error_code ec;
  lt::add_torrent_params params;

  bool const has_file = !request.torrent_file.empty();
  bool const has_magnet = !request.magnet_uri.empty();
  bool const has_resume = !request.resume_data.empty();

  if (has_resume) {
    params = lt::read_resume_data(
        lt::span<char const>(
            reinterpret_cast<char const*>(request.resume_data.data()),
            static_cast<std::ptrdiff_t>(request.resume_data.size())),
        ec);
    if (ec) throw std::runtime_error("unreadable resume data: " + ec.message());
  } else if (has_magnet) {
    params = lt::parse_magnet_uri(std::string(request.magnet_uri), ec);
    if (ec) throw std::runtime_error("invalid magnet uri: " + ec.message());
  } else if (!has_file) {
    throw std::runtime_error(
        "add_torrent needs a torrent file, a magnet uri or resume data");
  }

  // Metadata wins over resume data: resume data for a magnet carries no files,
  // and attaching the torrent file is what turns it into a real torrent.
  if (has_file) {
    params.ti = parse_torrent_bytes(request.torrent_file.data(),
                                   request.torrent_file.size());
  }

  if (!request.save_path.empty()) params.save_path = std::string(request.save_path);
  if (params.save_path.empty()) {
    throw std::runtime_error("a save path is required");
  }
  if (!request.name.empty()) params.name = std::string(request.name);

  if (!request.file_priorities.empty()) {
    params.file_priorities.clear();
    params.file_priorities.reserve(request.file_priorities.size());
    for (uint8_t priority : request.file_priorities) {
      params.file_priorities.push_back(lt::download_priority_t{priority});
    }
  }

  if (!request.trackers.empty()) {
    params.trackers.clear();
    params.tracker_tiers.clear();
    for (auto const& url : request.trackers) {
      params.trackers.push_back(std::string(url));
      params.tracker_tiers.push_back(0);
    }
  }

  params.storage_mode = request.pre_allocate ? lt::storage_mode_allocate
                                             : lt::storage_mode_sparse;

  params.flags |= lt::torrent_flags_t(request.flags_set);
  params.flags &= ~lt::torrent_flags_t(request.flags_unset);
  // Adding a torrent that is already there must fail rather than quietly
  // return the existing handle, which would lose the caller's options.
  params.flags |= lt::torrent_flags::duplicate_is_error;

  lt::torrent_handle handle = session_->add_torrent(std::move(params), ec);
  if (ec) throw std::runtime_error("add_torrent failed: " + ec.message());
  if (!handle.is_valid()) {
    throw std::runtime_error("add_torrent returned an invalid handle");
  }

  std::string const info_hash = hex_of(handle.info_hashes());
  handles_[info_hash] = handle;
  return rust::String(info_hash);
}

rust::String Session::add_magnet(rust::Str uri, rust::Str save_path) {
  AddTorrentRequest request;
  request.magnet_uri = rust::String(to_string(uri));
  request.save_path = rust::String(to_string(save_path));
  return add_torrent(request);
}

rust::String Session::add_torrent_from_resume(rust::Slice<uint8_t const> resume_data,
                                              rust::Str save_path) {
  if (resume_data.empty()) throw std::runtime_error("resume data is empty");

  AddTorrentRequest request;
  request.resume_data.reserve(resume_data.size());
  for (uint8_t byte : resume_data) request.resume_data.push_back(byte);
  request.save_path = rust::String(to_string(save_path));
  return add_torrent(request);
}

bool Session::wait_for_alert(int32_t millis) {
  if (millis < 0) millis = 0;
  return session_->wait_for_alert(lt::milliseconds(millis)) != nullptr;
}

rust::Vec<FlatAlert> Session::pop_alerts() {
  std::vector<lt::alert*> alerts;
  session_->pop_alerts(&alerts);

  rust::Vec<FlatAlert> out;
  out.reserve(alerts.size());
  for (lt::alert* alert : alerts) out.push_back(flatten_alert(alert));
  return out;
}

std::unique_ptr<Session> new_session(SessionConfig const& config) {
  return std::make_unique<Session>(config);
}

rust::String libtorrent_version() { return rust::String(LIBTORRENT_VERSION); }

int32_t session_stat_index(rust::Str name) {
  return lt::find_metric_idx(to_string(name));
}

rust::Vec<rust::String> session_stat_names() {
  // Placed at their own index, not in the order libtorrent lists them.
  //
  // `session_stats_alert` carries one flat array of values, and each metric
  // says where in it to look through `value_index`. Reading the list in order
  // and counting along it assumes those two agree, and they do not: the
  // counters and the gauges are numbered in separate ranges. Every name from
  // the point where they diverge then read somebody else's value, which is why
  // the number of connected peers came out in the hundreds of thousands.
  auto const metrics = lt::session_stats_metrics();
  int highest = -1;
  for (auto const& metric : metrics) {
    highest = std::max(highest, metric.value_index);
  }

  std::vector<std::string> placed(static_cast<std::size_t>(highest + 1));
  for (auto const& metric : metrics) {
    placed[static_cast<std::size_t>(metric.value_index)] = metric.name;
  }

  rust::Vec<rust::String> out;
  out.reserve(placed.size());
  for (auto const& name : placed) {
    // A gap is possible in principle; an empty name is skipped by the caller.
    out.push_back(rust::String(name));
  }
  return out;
}

rust::Vec<uint8_t> create_torrent(rust::Str path, int32_t piece_length, rust::Str comment,
                                  rust::Str creator, bool private_torrent,
                                  rust::Slice<rust::String const> trackers,
                                  rust::Slice<rust::String const> web_seeds,
                                  HashProgress& progress) {
  std::string const root = to_string(path);
  if (root.empty()) throw std::runtime_error("a path is required");

  lt::file_storage storage;
  lt::error_code ec;
  // Hidden files are skipped, which is what every torrent creator does: a
  // .DS_Store in a torrent is noise nobody wants to seed.
  lt::add_files(storage, root, [](std::string const& name) {
    auto const slash = name.find_last_of("/\\");
    std::string const base = slash == std::string::npos ? name : name.substr(slash + 1);
    return base.empty() || base[0] != '.';
  });
  if (storage.num_files() == 0) {
    throw std::runtime_error("no files found at " + root);
  }

  lt::create_torrent builder(storage, piece_length);
  if (!comment.empty()) builder.set_comment(to_string(comment).c_str());
  if (!creator.empty()) builder.set_creator(to_string(creator).c_str());
  builder.set_priv(private_torrent);

  for (std::size_t index = 0; index < trackers.size(); ++index) {
    builder.add_tracker(std::string(trackers[index]), static_cast<int>(index));
  }
  for (auto const& seed : web_seeds) builder.add_url_seed(std::string(seed));

  // Hashing reads every byte, which is why the Rust side runs this where
  // blocking is allowed.
  std::string const parent =
      root.substr(0, root.find_last_of("/\\") == std::string::npos
                         ? 0
                         : root.find_last_of("/\\"));
  int const total_pieces = builder.num_pieces();
  lt::set_piece_hashes(
      builder, parent.empty() ? "." : parent,
      [&progress, total_pieces](lt::piece_index_t piece) {
        progress.note_piece(static_cast<int32_t>(piece), total_pieces);
      },
      ec);
  if (ec) throw std::runtime_error("could not hash the files: " + ec.message());

  std::vector<char> encoded;
  lt::bencode(std::back_inserter(encoded), builder.generate());

  rust::Vec<uint8_t> out;
  out.reserve(encoded.size());
  for (char byte : encoded) out.push_back(static_cast<uint8_t>(byte));
  return out;
}

rust::String torrent_file_info_hash(rust::Slice<uint8_t const> torrent_file) {
  auto const info = parse_torrent_bytes(torrent_file.data(), torrent_file.size());
  return rust::String(hex_of(info->info_hashes()));
}

}  // namespace redeluge
