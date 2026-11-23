"""Run the reviewed two-peer restart case on private copies of the fixture stores."""
import argparse
import hashlib
import os
from pathlib import Path
import shutil
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("daemon", type=Path)
parser.add_argument("evidence", type=Path, help="Retained cutover qualification directory")
parser.add_argument("output", type=Path, help="New directory for writable peer stores")
args = parser.parse_args()
daemon = args.daemon.resolve(strict=True)
source = args.evidence.resolve(strict=True)
output = args.output.resolve()
configs = {
    slot: (source / f"resume39-source111-schedule-standard-repair01-{slot}/scratch").resolve()
    for slot in ("a", "b")
}
seeds = {
    "a": configs["a"],
    "b": (source / "resume39-source116-app-offline01-evidence/scratch/native-publication-9").resolve(),
}
databases = {slot: (seed / "data/db").resolve() for slot, seed in seeds.items()}
retained = (source, *configs.values(), *seeds.values(), *databases.values())
if any(output.is_relative_to(root) for root in retained):
    parser.error("output must be outside retained evidence and stores")
driver = source / "standard-sync-resume01/entry-openethereum-parity02.py"
editor = source / "resume39-rule-editor-build02-evidence/fixture_rule_editor"
transfer = source / "resume42-transfer-input01.json"
for path, digest in [
    (driver, "5f11ddf8f2f50da8d855a7f4de0d14c9c039649bdc04fe691afaffe7e32e6d69"),
    (editor, "4b6d75b98a0a7073d5a6700c5adba2bc5d36f099ee26b16e6de365e9bc243c59"),
    (transfer, "cae676f7cf66db86018bcc1b7cd8a2bdd37a3307635e22a13c09aded6abbfe59"),
]:
    with path.open("rb") as file:
        if hashlib.file_digest(file, "sha256").hexdigest() != digest:
            raise ValueError(f"Invalid qualification input: {path}")
output.mkdir(parents=True, exist_ok=False)
mounts = []
peers = []
for slot in ("a", "b"):
    config = configs[slot]
    copy = output / slot
    shutil.copytree(databases[slot], copy / "data/db")
    (copy / "home/.taraxa/data").mkdir(parents=True)
    for name in ("config.json", "genesis.json", "wallet.json", "wallet-second.json"):
        if (config / name).is_file():
            shutil.copy2(config / name, copy / name)
    # The reviewed store reader validates these paths inside its mount namespace.
    peer = Path("/home/fry/artifacts/evm-activation-readiness-20261111-01") / f"resume39-source111-schedule-sync-resumeoe04-{slot}/scratch"
    mounts.extend(["--bind", str(copy), str(peer)])
    peers.append(str(peer))
passwd = output / "passwd"
passwd.write_text(f"root:x:0:0:Fixture:{peers[0]}/home:/bin/sh\n")
command = [
    "bwrap", "--unshare-user", "--uid", "0", "--gid", "0", "--unshare-net", "--unshare-pid",
    "--die-with-parent", "--ro-bind", "/", "/", "--dev", "/dev", *mounts,
    "--ro-bind", str(passwd), "/etc/passwd", "--", "/usr/bin/python3", str(driver),
    peers[0], str(daemon), os.readlink("/proc/self/ns/net"), str(editor), peers[1], str(transfer),
]
subprocess.run(command, check=True, timeout=270)
