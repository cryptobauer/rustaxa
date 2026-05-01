#!/bin/sh
set -eu

git config --global include.path /root/.gitconfig-host
git config --global includeIf."gitdir:/workspaces/rustaxa/".path /root/.gitconfig-cryptobauer
git config --global core.sshCommand "ssh -i /root/.ssh/id_ed25519_cryptobauer"
git config core.untrackedCache true
ln -sf /build/compile_commands.json compile_commands.json

if [ -d /root/.gnupg-host ]; then
  TMP_GNUPG_HOST="$(mktemp -d)"
  cleanup_gpg_copy() {
    rm -rf "$TMP_GNUPG_HOST"
  }
  trap cleanup_gpg_copy EXIT INT TERM

  if (cd /root/.gnupg-host && tar --exclude='S.*' --exclude='*.lock' -cf - .) | (cd "$TMP_GNUPG_HOST" && tar -xf -); then
    chmod 700 "$TMP_GNUPG_HOST"
    if ! timeout 20s gpg --homedir "$TMP_GNUPG_HOST" --batch --export 2>/dev/null | gpg --batch --import 2>/dev/null; then
      echo "Skipping GPG key import (failed to export from host keyring copy)"
    fi
  else
    echo "Skipping GPG key import (unable to copy host keyring)"
  fi
fi
