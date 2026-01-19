FROM ghcr.io/cryptobauer/taraxa-build/taraxa-builder:debug

ENV LANG=C.UTF-8
ENV LC_ALL=C.UTF-8

RUN apt update \
    && apt install -y \
        clangd \
        curl \
        fd-find \
        fzf \
        git \
        htop \
        less \
        ninja-build \
        openssh-client \
        tree \
        vim \
        zsh \
        llvm-19 \
        libclang-19-dev \
    && rm -rf /var/lib/apt/lists/*

# Install Oh My Zsh
RUN sh -c "$(curl -fsSL https://raw.githubusercontent.com/ohmyzsh/ohmyzsh/master/tools/install.sh)" "" --unattended

# Set zsh as default shell for root
RUN chsh -s $(which zsh) root

# Configure fzf to use fd
RUN echo 'export FZF_DEFAULT_COMMAND="fdfind --type f"' >> /root/.zshrc && \
    echo 'export FZF_CTRL_T_COMMAND="$FZF_DEFAULT_COMMAND"' >> /root/.zshrc

# Needed for Rust (bindgen) (remove when in base image together with llm-19 install)
ENV LLVM_CONFIG_PATH=/usr/lib/llvm-19/bin/llvm-config
ENV LIBCLANG_PATH=/usr/lib/llvm-19/lib
