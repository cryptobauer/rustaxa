#!/bin/sh
set -eu

git config --global include.path /root/.gitconfig-host
git config --global includeIf."gitdir:/workspaces/rustaxa/".path /root/.gitconfig-cryptobauer
git config --global core.sshCommand "ssh -i /root/.ssh/id_ed25519_cryptobauer"
git config core.untrackedCache true
ln -sf /build/compile_commands.json compile_commands.json

if ! timeout 8s gpg --homedir /root/.gnupg-host --lock-never --export 2>/dev/null | gpg --batch --import 2>/dev/null; then
  echo "Skipping GPG key import (host keyring locked or unavailable)"
fi
