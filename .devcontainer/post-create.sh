#!/bin/sh
set -eu

git config --global include.path /root/.gitconfig-host
git config --global includeIf."gitdir:/workspaces/rustaxa/".path /root/.gitconfig-cryptobauer

mkdir -p /root/.ssh
chmod 700 /root/.ssh

if ! grep -q "github.com" /root/.ssh/known_hosts 2>/dev/null; then
  ssh-keyscan -t ed25519 github.com >> /root/.ssh/known_hosts 2>/dev/null || true
fi

cp /root/.ssh/id_ed25519_cryptobauer.pub /root/.ssh/id_ed25519_cryptobauer.select
chmod 600 /root/.ssh/id_ed25519_cryptobauer.select

git config --global core.sshCommand "ssh -F /dev/null -o IdentitiesOnly=yes -o IdentityFile=/root/.ssh/id_ed25519_cryptobauer.select"
git config --global gpg.format ssh
git config --global user.signingkey /root/.ssh/id_ed25519_cryptobauer.pub
git config --global commit.gpgsign true
git config --global tag.gpgSign true

if ! git config --global --get user.name >/dev/null 2>&1; then
  git config --global user.name "bender"
fi

if ! git config --global --get user.email >/dev/null 2>&1; then
  git config --global user.email "bender@cryptobauer.com"
fi

if [ -z "${SSH_AUTH_SOCK:-}" ]; then
  echo "Warning: SSH agent is not forwarded; pull/push/sign will fail until SSH_AUTH_SOCK is available"
fi

if [ -n "${SSH_AUTH_SOCK:-}" ] && [ -f /root/.ssh/id_ed25519_cryptobauer.pub ]; then
  BENDER_PUB="$(awk '{print $1 " " $2}' /root/.ssh/id_ed25519_cryptobauer.pub)"
  if ! ssh-add -L 2>/dev/null | awk '{print $1 " " $2}' | grep -Fx "$BENDER_PUB" >/dev/null; then
    echo "Warning: forwarded SSH agent does not have bender@cryptobauer.com private key loaded"
    echo "Host fix (macOS): ssh-add --apple-use-keychain ~/.ssh/id_ed25519_cryptobauer"
    echo "If agent has multiple keys and picks the wrong one: ssh-add -D && ssh-add --apple-use-keychain ~/.ssh/id_ed25519_cryptobauer"
  fi
fi

git config core.untrackedCache true
ln -sf /build/compile_commands.json compile_commands.json
