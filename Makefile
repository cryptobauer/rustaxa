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
CPP_INTERSECTION_BASE?=main
CPP_INTERSECTION_REF?=cpp-reference
CPP_INTERSECTION_EXCLUDES?=':(exclude)rust/**' ':(exclude).devcontainer/**' ':(exclude).github/**' ':(exclude)Makefile' ':(exclude)RUST_REWRITE.md'
CPP_INTERSECTION_PATCH?=$(BUILD_OUTPUT_DIR)/cpp-reference-intersection.patch

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
		paths="$$(git diff --name-only "$(CPP_INTERSECTION_BASE)".."$(CPP_INTERSECTION_REF)" -- . $(CPP_INTERSECTION_EXCLUDES) | tr '\n' ' ')"; \
	fi; \
	if [ -z "$$paths" ]; then \
		echo "Error: no intersection paths detected (or provided via CPP_INTERSECTION_PATHS)"; \
		exit 1; \
	fi; \
	@mkdir -p "$(BUILD_OUTPUT_DIR)"
	@git diff --binary "$(FROM)".."$(TO)" -- $$paths > "$(CPP_INTERSECTION_PATCH)"
	@echo "Wrote intersection patch to $(CPP_INTERSECTION_PATCH)"

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
		paths="$$(git diff --name-only "$(CPP_INTERSECTION_BASE)".."$(CPP_INTERSECTION_REF)" -- . $(CPP_INTERSECTION_EXCLUDES) | tr '\n' ' ')"; \
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

.PHONY: cpp-intersection-list
cpp-intersection-list: ## Print detected intersection paths (override via CPP_INTERSECTION_PATHS)
	@paths="$(CPP_INTERSECTION_PATHS)"; \
	if [ -z "$$paths" ]; then \
		paths="$$(git diff --name-only "$(CPP_INTERSECTION_BASE)".."$(CPP_INTERSECTION_REF)" -- . $(CPP_INTERSECTION_EXCLUDES) | tr '\n' ' ')"; \
	fi; \
	if [ -z "$$paths" ]; then \
		echo "No intersection paths detected"; \
		exit 1; \
	fi; \
	for p in $$paths; do echo $$p; done
