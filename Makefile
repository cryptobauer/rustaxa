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
CMAKE_BUILD_TYPE?=Debug

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
		-DLLVM_VERSION=$(LLVM_VERSION)

.PHONY: build
build: ## Compile the project locally.
	@if [ ! -f $(BUILD_OUTPUT_DIR)/CMakeCache.txt ]; then \
		$(MAKE) configure; \
	fi
	cmake --build $(BUILD_OUTPUT_DIR) -j6
	cp $(BUILD_OUTPUT_DIR)/tests/CTestTestfile.cmake $(BUILD_OUTPUT_DIR)/bin/

.PHONY: clean
clean: ## Clean the build directory.
	@find "$(BUILD_OUTPUT_DIR)" -mindepth 1 -delete
