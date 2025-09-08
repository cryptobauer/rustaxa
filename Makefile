## -------------------------------------------------------------------
## This Makefile contains some useful commands to help you develop and
## maintain your project. You can run `make help` to get a list of all
## available commands.
## -------------------------------------------------------------------

# Default LLVM version (can be overridden with make LLVM_VERSION=<version>)
LLVM_VERSION ?= 19

.PHONY: help
help:  ## Show this help.
	@sed -ne '/@sed/!s/## //p' $(MAKEFILE_LIST)

.PHONY: image
image: ## Build the docker image for the project.
	docker build --build-arg LLVM_VERSION=$(LLVM_VERSION) -t rustaxa-builder -f Dockerfile.builder .

.PHONY: conan
conan: ## Run conan install to generate dependencies in build directory.
	docker run -e LLVM_VERSION=$(LLVM_VERSION) --rm -v .:/workarea rustaxa-builder bash -c "mkdir -p /workarea/build && cd /workarea && conan install . -s 'build_type=RelWithDebInfo' -s '&:build_type=Debug' --profile:host=clang --profile:build=clang --build=missing --output-folder=build"

.PHONY: conan-cached
conan-cached: ## Run conan install with persistent cache.
	docker run -e LLVM_VERSION=$(LLVM_VERSION) --rm \
		-v .:/workarea \
		-v rustaxa-cargo-cache:/root/.cargo \
		-v rustaxa-conan-cache:/root/.conan2 \
		-v rustaxa-sccache-cache:/root/.cache/sccache \
		rustaxa-builder bash -c "mkdir -p build && conan install . -s 'build_type=RelWithDebInfo' -s '&:build_type=Debug' --profile:host=clang --profile:build=clang --build=missing --output-folder=build"

.PHONY: build
build: ## Configure and build the project inside Docker container.
	docker run -e LLVM_VERSION=$(LLVM_VERSION) --rm -v .:/workarea rustaxa-builder bash -c "mkdir -p /workarea/build && cd /workarea/build && cmake .. -DCMAKE_BUILD_TYPE=Debug -DTARAXA_ENABLE_LTO=OFF -DTARAXA_STATIC_BUILD=ON -DTARAXA_GPERF=ON && make -j2 taraxad"

.PHONY: build-cached
build-cached: ## Build with persistent caches (much faster for incremental builds).
	docker run -e LLVM_VERSION=$(LLVM_VERSION) --rm \
		-v .:/workarea \
		-v rustaxa-cargo-cache:/root/.cargo \
		-v rustaxa-conan-cache:/root/.conan2 \
		-v rustaxa-sccache-cache:/root/.cache/sccache \
		rustaxa-builder bash -c "cd build && cmake .. -DCMAKE_BUILD_TYPE=Debug -DTARAXA_ENABLE_LTO=OFF -DTARAXA_STATIC_BUILD=ON -DTARAXA_GPERF=ON && make -j2 taraxad"

.PHONY: shell-cached
shell-cached: ## Enter shell with all caches mounted.
	docker run -e LLVM_VERSION=$(LLVM_VERSION) -it \
		-v .:/workarea \
		-v rustaxa-cargo-cache:/root/.cargo \
		-v rustaxa-conan-cache:/root/.conan2 \
		-v rustaxa-sccache-cache:/root/.cache/sccache \
		rustaxa-builder bash

.PHONY: clean-cache
clean-cache: ## Remove all cache volumes.
	docker volume rm rustaxa-cargo-cache rustaxa-conan-cache rustaxa-sccache-cache 2>/dev/null || true

.PHONY: test
test: ## Run the tests inside Docker container.
	docker run -e LLVM_VERSION=$(LLVM_VERSION) --rm -v .:/workarea rustaxa-builder bash -c "cd /workarea/build && ctest"

.PHONY: enter
enter: ## Enter the docker container.
	docker run -e LLVM_VERSION=$(LLVM_VERSION) -it -v .:/workarea rustaxa-builder

.PHONY: clean
clean: ## Clean the build directory.
	@rm -rf build
