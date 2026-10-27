#pragma once

// Finite closeout safeguard for repo-owned bridge tests. The parent retains
// every root until the child exits and its actual gtest results are audited.
#include <gtest/gtest.h>
#include <json/json.h>
#include <fcntl.h>
#include <sys/stat.h>
#include <unistd.h>

#include <atomic>
#include <cstdlib>
#include <filesystem>
#include <fstream>
#include <stdexcept>
#include <string>

namespace rustaxa_closeout {
inline bool active() { return std::getenv("RUSTAXA_ERA_LAUNCH_MANIFEST") != nullptr; }

inline uint64_t tree_bytes(const std::filesystem::path& root) {
  uint64_t total = 0;
  for (const auto& entry : std::filesystem::recursive_directory_iterator(root)) {
    if (!entry.is_symlink() && entry.is_regular_file()) total += entry.file_size();
  }
  return total;
}

inline void append(const std::filesystem::path& file, const Json::Value& value) {
  Json::StreamWriterBuilder builder;
  builder["indentation"] = "";
  const auto text = Json::writeString(builder, value) + "\n";
  const int fd = ::open(file.c_str(), O_WRONLY | O_APPEND | O_CREAT, 0600);
  if (fd < 0) throw std::runtime_error("closeout ledger open");
  const auto count = ::write(fd, text.data(), text.size());
  const bool synced = ::fsync(fd) == 0;
  ::close(fd);
  if (count != static_cast<ssize_t>(text.size()) || !synced) throw std::runtime_error("closeout ledger sync");
  const int parent = ::open(file.parent_path().c_str(), O_RDONLY | O_DIRECTORY);
  if (parent < 0) throw std::runtime_error("closeout ledger parent");
  const bool parent_synced = ::fsync(parent) == 0;
  ::close(parent);
  if (!parent_synced) throw std::runtime_error("closeout parent sync");
}

inline std::filesystem::path path(const std::string& label) {
  const auto temporary = std::filesystem::temp_directory_path();
  if (!active()) return temporary / label;
  const std::filesystem::path launch_path(std::getenv("RUSTAXA_ERA_LAUNCH_MANIFEST"));
  Json::Value launch;
  std::ifstream input(launch_path);
  input >> launch;
  const auto parent = std::filesystem::path(launch["scratch_parent"].asString());
  struct stat parent_stat {};
  if (::stat(parent.c_str(), &parent_stat) != 0 ||
      Json::UInt64(parent_stat.st_dev) != launch["scratch_parent_identity"][0].asUInt64() ||
      Json::UInt64(parent_stat.st_ino) != launch["scratch_parent_identity"][1].asUInt64() ||
      temporary != parent / "legacy-temps" || std::filesystem::is_symlink(temporary)) {
    throw std::runtime_error("closeout parent identity or containment");
  }
  static std::atomic<unsigned long> sequence{0};
  const auto root = temporary / (label + "-" + std::to_string(::getpid()) + "-" + std::to_string(sequence++));
  if (std::filesystem::exists(root) || std::filesystem::is_symlink(root)) {
    throw std::runtime_error("closeout root must be NEW");
  }
  const auto* info = ::testing::UnitTest::GetInstance()->current_test_info();
  if (!info) throw std::runtime_error("closeout test identity");
  Json::Value event;
  event["state"] = "registered_before_creation";
  event["root"] = root.string();
  event["test"] = std::string(info->test_suite_name()) + "." + info->name();
  event["launch"] = launch["identity"];
  event["source_manifest"] = launch["source_manifest"];
  event["disposition"] = "retain until child termination and source-bound gtest audit";
  const auto& resources = launch["resources"];
  const auto actual = tree_bytes(resources["run"].asString()) + tree_bytes(resources["corpus"].asString());
  const auto baseline = resources["baseline_actual_bytes"].asUInt64();
  const uint64_t charged = resources["prior_charged_bytes"].asUInt64() +
                           (actual > baseline ? actual - baseline : 0) + 2 * 1024 * 1024;
  event["live_charged_bytes"] = Json::UInt64(charged);
  // Admission reserves one finite bridge case; all completed bridge roots
  // remain charged until the parent audits the actual command and test XML.
  if (charged + 32 * 1024 * 1024 > (resources.isMember("retained_live_cap_bytes") ? resources["retained_live_cap_bytes"].asUInt64() : 1342177280ULL)) {
    throw std::runtime_error("closeout C++ scratch peak admission");
  }
  const auto ledger = launch_path.parent_path() / "cpp-scratch.jsonl";
  append(ledger, event);
  if (!std::filesystem::create_directory(root)) throw std::runtime_error("closeout exclusive root creation");
  struct stat root_stat {};
  if (::lstat(root.c_str(), &root_stat) != 0 || !S_ISDIR(root_stat.st_mode)) {
    throw std::runtime_error("closeout new root identity");
  }
  event["state"] = "created";
  event["identity"].append(Json::UInt64(root_stat.st_dev));
  event["identity"].append(Json::UInt64(root_stat.st_ino));
  append(ledger, event);
  return root;
}
}  // namespace rustaxa_closeout
