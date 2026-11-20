#pragma once
#ifdef RUSTAXA_ENABLE
#include "rust/cxx.h"
#include <memory>
namespace rustaxa { struct BridgeConsensusApplication; class NativeSelectedProductionConfig; }
namespace taraxa {
namespace cli { class Config; }
std::unique_ptr<rustaxa::NativeSelectedProductionConfig> loadSelectedProductionConfig(const cli::Config& config);
rust::Box<rustaxa::BridgeConsensusApplication> createQualifiedProductionApplication(const cli::Config& config);
}  // namespace taraxa
#endif
