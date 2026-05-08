## -------------------------------------------------------------------
## This Makefile contains some useful commands to help you develop and
## maintain your project. You can run `make help` to get a list of all
## available commands.
##
## Note that these commands assume one is inside the devcontainer.
## -------------------------------------------------------------------

# Default LLVM version (can be overridden with make LLVM_VERSION=<version>)
LLVM_VERSION?=19
BUILD_OUTPUT_DIR?=/build
CMAKE_BUILD_TYPE?=RelWithDebInfo
CPP_INTERSECTION_PATHS?=
CPP_INTERSECTION_BASE?=upstream-main
CPP_INTERSECTION_HEAD?=main
CPP_INTERSECTION_EXCLUDES?=':(exclude)rust/**' ':(exclude).devcontainer/**' ':(exclude).github/**' ':(exclude).gitignore' ':(exclude)Makefile' ':(exclude)AGENTS.md' ':(exclude)PLAN.md'
CPP_INTERSECTION_PATCH?=$(BUILD_OUTPUT_DIR)/cpp-reference-intersection.patch
RUST_MANIFEST?=rust/Cargo.toml
REWRITE_CONSENSUS_TESTS?=rust_consensus_tests dag_test dag_block_test dag_shim_test pbft_chain_test pbft_chain_shim_test proposed_blocks_shim_test pbft_manager_test vote_test pillar_chain_test rewards_stats_test
REWRITE_FINAL_CHAIN_TESTS?=final_chain_test state_api_test rpc_test

define require_cmake_build
	@if [ ! -f "$(BUILD_OUTPUT_DIR)/CMakeCache.txt" ]; then \
		echo "Error: $(BUILD_OUTPUT_DIR) is not configured. Run 'make configure' first."; \
		exit 1; \
	fi
endef

define warn_unless_rustaxa_enabled
	@if [ -f "$(BUILD_OUTPUT_DIR)/CMakeCache.txt" ] && ! grep -q '^RUSTAXA_ENABLE:BOOL=ON' "$(BUILD_OUTPUT_DIR)/CMakeCache.txt"; then \
		echo "Warning: $(BUILD_OUTPUT_DIR) was not configured with RUSTAXA_ENABLE=ON; Rust-enabled validation may be incomplete."; \
	fi
endef

define require_cmake_target
	@if ! cmake --build "$(BUILD_OUTPUT_DIR)" --target help 2>/dev/null | grep -Eq '(^|[[:space:]])$(1)($$|[[:space:]])'; then \
		echo "Error: CMake target $(1) is not available in $(BUILD_OUTPUT_DIR). Run 'make configure' with the required RUSTAXA flags."; \
		exit 1; \
	fi
endef

.PHONY: help
help:  ## Show this help.
	@awk 'BEGIN {FS = ":.*?## "} \
		/^[a-zA-Z0-9_-]+:.*?## / { \
			printf "  \033[1;36m%-20s\033[0m %s\n", $$1, $$2; \
			next; \
		} \
		/^##/ { \
			sub(/^##[ ]?/, ""); \
			print; \
		}' $(MAKEFILE_LIST)

.PHONY: configure
configure: ## Configure the project locally.
	mkdir -p $(BUILD_OUTPUT_DIR)
	./scripts/config.sh
	conan install . -s "build_type=Release" -s "&:build_type=$(CMAKE_BUILD_TYPE)" --profile:host=clang --profile:build=clang --build=missing --output-folder=$(BUILD_OUTPUT_DIR)
	cd $(BUILD_OUTPUT_DIR) && \
	cmake $(CURDIR) \
		-DCMAKE_BUILD_TYPE=$(CMAKE_BUILD_TYPE) \
		-DCMAKE_CXX_COMPILER_LAUNCHER=ccache \
		-DTARAXA_ENABLE_LTO=OFF \
		-DTARAXA_STATIC_BUILD=ON \
		-DTARAXA_GPERF=ON \
		-DRUSTAXA_ENABLE=ON \
		-DRUSTAXA_ENABLE_VDF=ON \
		-DRUSTAXA_ENABLE_STORAGE=ON \
		-DRUSTAXA_ENABLE_FINAL_CHAIN=ON \
		-DRUSTAXA_ENABLE_SORTITION_PARAMS=ON \
		-DRUSTAXA_ENABLE_PBFT_CHAIN=ON \
		-DRUSTAXA_ENABLE_PROPOSED_BLOCKS=ON \
		-DLLVM_VERSION=$(LLVM_VERSION)

.PHONY: build
build: ## Compile the project locally.
	@if [ ! -f $(BUILD_OUTPUT_DIR)/CMakeCache.txt ]; then \
		$(MAKE) configure; \
	fi
	cmake --build $(BUILD_OUTPUT_DIR) -j6 --target=taraxad
	cp $(BUILD_OUTPUT_DIR)/tests/CTestTestfile.cmake $(BUILD_OUTPUT_DIR)/bin/

.PHONY: clean
clean: ## Clean the build directory.
	@find "$(BUILD_OUTPUT_DIR)" -mindepth 1 -delete
	@find rust/target -mindepth 1 -delete

.PHONY: cpp-intersection-patch
cpp-intersection-patch: ## Write C++ intersection patch for FROM..TO (make cpp-intersection-patch FROM=<sha> TO=<sha>)
	@if [ -z "$(FROM)" ] || [ -z "$(TO)" ]; then \
		echo "Error: FROM and TO are required"; \
		echo "Example: make cpp-intersection-patch FROM=<base_sha> TO=<tip_sha>"; \
		exit 1; \
	fi
	@paths="$(CPP_INTERSECTION_PATHS)"; \
	if [ -z "$$paths" ]; then \
		paths="$$(git diff --name-only --diff-filter=M "$(FROM)".."$(TO)" -- . $(CPP_INTERSECTION_EXCLUDES) | tr '\n' ' ')"; \
	fi; \
	if [ -z "$$paths" ]; then \
		echo "Error: no intersection paths detected (or provided via CPP_INTERSECTION_PATHS)"; \
		exit 1; \
	fi; \
	mkdir -p "$(BUILD_OUTPUT_DIR)"; \
	git diff --binary "$(FROM)".."$(TO)" -- $$paths > "$(CPP_INTERSECTION_PATCH)"; \
	echo "Wrote intersection patch to $(CPP_INTERSECTION_PATCH)"

.PHONY: cpp-reference-apply-intersection
cpp-reference-apply-intersection: ## Apply C++ intersection FROM..TO to current branch (3-way) and stage it
	@if [ -z "$(FROM)" ] || [ -z "$(TO)" ]; then \
		echo "Error: FROM and TO are required"; \
		echo "Example: make cpp-reference-apply-intersection FROM=<base_sha> TO=<tip_sha>"; \
		exit 1; \
	fi
	@if ! git diff --quiet || ! git diff --cached --quiet; then \
		echo "Error: working tree is not clean. Commit/stash changes first."; \
		exit 1; \
	fi
	@paths="$(CPP_INTERSECTION_PATHS)"; \
	if [ -z "$$paths" ]; then \
		paths="$$(git diff --name-only --diff-filter=M "$(FROM)".."$(TO)" -- . $(CPP_INTERSECTION_EXCLUDES) | tr '\n' ' ')"; \
	fi; \
	if [ -z "$$paths" ]; then \
		echo "Error: no intersection paths detected (or provided via CPP_INTERSECTION_PATHS)"; \
		exit 1; \
	fi; \
	git diff --binary "$(FROM)".."$(TO)" -- $$paths | git apply --index --3way -
	@if git diff --cached --quiet; then \
		echo "No C++ intersection changes in range $(FROM)..$(TO)"; \
	else \
		echo "Applied and staged C++ intersection changes from $(FROM)..$(TO)"; \
		echo "Next: git commit -m 'chore(cpp-reference): sync C++ intersection from $(TO)'"; \
	fi

.PHONY: rust-update
rust-update: ## Update the Rust toolchain (works around overlayfs cross-device rename by reinstalling).
	rustup toolchain remove stable 2>/dev/null || true
	rustup toolchain install stable
	rustup default stable

.PHONY: rewrite-validate-fast
rewrite-validate-fast: ## Run fast Rust rewrite checks and whitespace validation.
	cargo fmt --manifest-path $(RUST_MANIFEST) --all --check
	cargo clippy --manifest-path $(RUST_MANIFEST)
	cargo test --manifest-path $(RUST_MANIFEST)
	git diff --check

.PHONY: rewrite-validate-storage
rewrite-validate-storage: ## Run Rust storage bridge tests and C++ vs Rust storage conformance diff.
	$(call require_cmake_build)
	$(call warn_unless_rustaxa_enabled)
	$(call require_cmake_target,rust_storage_tests)
	cmake --build $(BUILD_OUTPUT_DIR) --target rust_storage_tests
	$(BUILD_OUTPUT_DIR)/bin/rust_storage_tests
	scripts/storage_conformance_diff.sh

.PHONY: rewrite-validate-consensus
rewrite-validate-consensus: rewrite-validate-fast ## Run Rust validation plus targeted consensus C++ tests.
	$(call require_cmake_build)
	$(call warn_unless_rustaxa_enabled)
	@for test in $(REWRITE_CONSENSUS_TESTS); do \
		if cmake --build "$(BUILD_OUTPUT_DIR)" --target help 2>/dev/null | grep -Eq "(^|[[:space:]])$$test($$|[[:space:]])"; then \
			cmake --build "$(BUILD_OUTPUT_DIR)" --target "$$test"; \
		else \
			echo "Skipping $$test: CMake target is not available"; \
			continue; \
		fi; \
		if [ -x "$(BUILD_OUTPUT_DIR)/bin/$$test" ]; then \
			echo "Running $$test"; \
			"$(BUILD_OUTPUT_DIR)/bin/$$test"; \
		else \
			echo "Skipping $$test: $(BUILD_OUTPUT_DIR)/bin/$$test is not built"; \
		fi; \
	done

.PHONY: rewrite-validate-final-chain
rewrite-validate-final-chain: rewrite-validate-fast ## Run Rust validation, targeted FinalChain tests, and startup smoke.
	$(call require_cmake_build)
	$(call warn_unless_rustaxa_enabled)
	@for test in $(REWRITE_FINAL_CHAIN_TESTS); do \
		if cmake --build "$(BUILD_OUTPUT_DIR)" --target help 2>/dev/null | grep -Eq "(^|[[:space:]])$$test($$|[[:space:]])"; then \
			cmake --build "$(BUILD_OUTPUT_DIR)" --target "$$test"; \
		else \
			echo "Skipping $$test: CMake target is not available"; \
			continue; \
		fi; \
		if [ -x "$(BUILD_OUTPUT_DIR)/bin/$$test" ]; then \
			echo "Running $$test"; \
			"$(BUILD_OUTPUT_DIR)/bin/$$test"; \
		else \
			echo "Skipping $$test: $(BUILD_OUTPUT_DIR)/bin/$$test is not built"; \
		fi; \
	done
	$(MAKE) rewrite-validate-smoke

.PHONY: rewrite-validate-smoke
rewrite-validate-smoke: ## Build taraxad and run a non-destructive Rust-enabled startup smoke check.
	$(call require_cmake_build)
	$(call warn_unless_rustaxa_enabled)
	$(call require_cmake_target,taraxad)
	cmake --build $(BUILD_OUTPUT_DIR) --target taraxad
	@if [ ! -x "$(BUILD_OUTPUT_DIR)/bin/taraxad" ]; then \
		echo "Error: $(BUILD_OUTPUT_DIR)/bin/taraxad is not built. Run 'cmake --build $(BUILD_OUTPUT_DIR) --target taraxad'."; \
		exit 1; \
	fi
	$(BUILD_OUTPUT_DIR)/bin/taraxad --version >/dev/null

.PHONY: cpp-intersection-list
cpp-intersection-list: ## Print detected intersection paths (override via CPP_INTERSECTION_PATHS)
	@paths="$(CPP_INTERSECTION_PATHS)"; \
	if [ -z "$$paths" ]; then \
		paths="$$(git diff --name-only --diff-filter=M "$(CPP_INTERSECTION_BASE)".."$(CPP_INTERSECTION_HEAD)" -- . $(CPP_INTERSECTION_EXCLUDES) | tr '\n' ' ')"; \
	fi; \
	if [ -z "$$paths" ]; then \
		echo "No intersection paths detected"; \
		exit 1; \
	fi; \
	for p in $$paths; do echo $$p; done
