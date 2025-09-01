# This is a stripped down version of `Dockerfile` which is meant for local development.

FROM ubuntu:24.04@sha256:7c06e91f61fa88c08cc74f7e1b7c69ae24910d745357e0dfe1d2c0322aaf20f9

ARG LLVM_VERSION=19
ENV LLVM_VERSION=${LLVM_VERSION}

# Avoid prompts from apt
ENV DEBIAN_FRONTEND=noninteractive

# Install standard packages
RUN apt-get update && DEBIAN_FRONTEND=noninteractive \
    apt-get install -y --no-install-recommends \
    tzdata \
    && apt-get install -y \
    tar \
    git \
    curl \
    wget \
    python3-pip \
    lsb-release \
    libmicrohttpd-dev \
    libgoogle-perftools-dev \
    software-properties-common \
    && rm -rf /var/lib/apt/lists/*

# install solc for py_test if arch is not arm64 because it is not availiable

# Install solc 0.8.24 as we do not support 0.8.25 yet
RUN \
    if [ `arch` != "aarch64" ]; \
    then  \
    curl -L -o solc-0.8.25 https://github.com/ethereum/solidity/releases/download/v0.8.25/solc-static-linux \
    && chmod +x solc-0.8.25 \
    && mv solc-0.8.25 /usr/bin/solc; \
    fi

# install standard tools
RUN add-apt-repository ppa:ethereum/ethereum \
    && apt-get update \
    && apt-get install -y \
    clang-format-$LLVM_VERSION \
    clang-tidy-$LLVM_VERSION \
    llvm-$LLVM_VERSION \
    golang-go \
    ca-certificates \
    libtool \
    autoconf \
    binutils \
    cmake \
    ccache \
    ninja-build \
    # this libs are required for arm build by go part
    libzstd-dev \
    libsnappy-dev \
    # replace this with conan dependency
    rapidjson-dev \
    && rm -rf /var/lib/apt/lists/*

ENV CXX="clang++-${LLVM_VERSION}"
ENV CC="clang-${LLVM_VERSION}"

# HACK remove this when update to conan 2.0
RUN ln -s /usr/bin/clang-${LLVM_VERSION} /usr/bin/clang
RUN ln -s /usr/bin/clang++-${LLVM_VERSION} /usr/bin/clang++

# Install conan
RUN apt-get remove -y python3-distro
RUN pip3 install conan --break-system-packages

# Install conan deps
WORKDIR /workarea
COPY scripts scripts
COPY conanfile.py .

RUN mkdir -p /buildarea
RUN ./scripts/config.sh
RUN conan install . -s "build_type=RelWithDebInfo" -s "&:build_type=Debug" --profile:host=clang --profile:build=clang --build=missing --output-folder=/buildarea
