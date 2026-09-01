import atexit
import json
import os
import shutil
import subprocess
from dataclasses import dataclass
from pathlib import Path
from subprocess import Popen
from typing import Optional, Sequence

import web3
from eth_account.signers.local import LocalAccount
from web3 import Web3
from web3.eth import Eth
from web3.net import Net

from common.util.wait import wait

_localhost = '127.0.0.1'

_validator_identities = (
    ("de2b1203d72d3549ee2f733b00b2789414c7cea5", "d05dc12c1df1edc9f3367fba550b7971fc2de6c5998d8784051c5be69abc9644"),
    ("973ecb1c08c8eb5a7eaa0d3fd3aab7924f2838b0", "931f5e7db07c9969e438db7e287eabbaaca49ca414f5f3a402ea6997ade40081"),
    ("4fae949ac2b72960fbe857b56532e2d3c8418d5e", "97485c51e033260894132aa326bb1c984a70ac7f4202315b90e669fb701a8f64"),
    ("415cf514eb6a5a8bd4d325d4874eae8cf26bcfe0", "783a02cd21f22a9f305ccedaef677f16b0d6c48dd9844cb676d5178ed628eb92"),
    ("b770f7a99d0b7ad9adf6520be77ca20ee99b0858", "46b69174b3ec82cc136509367c93f1bb616ed16cbc189dc0afa628c919469c59"),
)


class Node:
    class InitMode:
        pass

    @dataclass
    class ManagedProcessInitMode(InitMode):
        executable_path: str
        clean_data = True

    @dataclass
    class RemoteInitMode(InitMode):
        host: str = _localhost

    def __init__(self, cfg_file_path, wallet_file_path, genesis_file_path, mode: InitMode, default_w3_provider_type='http'):
        wallet_file_path, cfg_file_path, genesis_file_path = Path(wallet_file_path), Path(cfg_file_path), Path(genesis_file_path)
        with open(cfg_file_path, mode="r") as f:
            cfg = json.load(f)
        self._w3_by_type = {}
        self._proc = None
        self._launch_command = None
        self._data_path = Path(cfg["data_path"])
        with open(wallet_file_path, mode="r") as f:
            self.account: LocalAccount = web3.Account.from_key(json.load(f)["node_secret"])
        self.w3: Optional[Web3] = None
        self.eth: Optional[Eth] = None
        self.net: Optional[Net] = None

        atexit.register(self.destructor)

        net_host = _localhost
        if isinstance(mode, Node.ManagedProcessInitMode):
            if mode.clean_data:
                shutil.rmtree(self._data_path, ignore_errors=True)
            os.makedirs(self._data_path, exist_ok=True)

            datadir_cfg_file_path = Path(cfg["data_path"] + "/" + cfg_file_path.name)

            shutil.copyfile(cfg_file_path, datadir_cfg_file_path)

            datadir_wallet_file_path = Path(cfg["data_path"] + "/" + wallet_file_path.name)
            shutil.copyfile(wallet_file_path, datadir_wallet_file_path)

            datadir_genesis_file_path = Path(cfg["data_path"] + "/" + genesis_file_path.name)
            # shutil.copyfile(genesis_file_path, datadir_genesis_file_path)

            # add default chain config section in config file and add default account to it
            config_result = subprocess.run(
                [mode.executable_path, "--command", "config", "--config", datadir_cfg_file_path,
                 "--wallet", datadir_wallet_file_path, "--genesis", datadir_genesis_file_path, "--chain-id", "0"],
                check=False,
            )
            if config_result.returncode != 0:
                raise RuntimeError(f"node config generation failed with exit code {config_result.returncode}")

            with open(datadir_genesis_file_path, mode="r") as f:
                genesis = json.load(f)
                stake = int(genesis["dpos"]["validator_maximum_stake"], 16)
                spendable_balance = int("0x1ffffffffffffff", 16)
                genesis["initial_balances"] = {
                    address: hex(stake + spendable_balance) for address, _ in _validator_identities
                }
                genesis["dpos"]["initial_validators"] = [
                    {
                        "address": address,
                        "commission": "0x0",
                        "delegations": {address: hex(stake)},
                        "description": f"Taraxa integration validator {index}",
                        "endpoint": "",
                        "owner": address,
                        "vrf_key": f"0x{vrf_key}",
                    }
                    for index, (address, vrf_key) in enumerate(_validator_identities, start=1)
                ]
            with open(datadir_genesis_file_path, mode="w") as f:
                json.dump(genesis, f)

            self._launch_command = [mode.executable_path, "--config", str(datadir_cfg_file_path),
                                    "--wallet", str(datadir_wallet_file_path), "--genesis", str(datadir_genesis_file_path),
                                    "--data-dir", str(self._data_path)]
            self.start()
        elif isinstance(mode, Node.RemoteInitMode):
            net_host = mode.host
        else:
            raise AssertionError("unknown init mode")

        cfg_rpc = cfg["network"].get("rpc", {})
        rpc_http_port = cfg_rpc.get("http_port", None)
        if rpc_http_port is not None:
            self._w3_by_type['http'] = Web3(Web3.HTTPProvider(
                endpoint_uri=f"http://{net_host}:{rpc_http_port}",
                request_kwargs=dict(timeout=45),
            ))
        rpc_ws_port = cfg_rpc.get("ws_port", None)
        if rpc_ws_port is not None:
            self._w3_by_type['ws'] = Web3(Web3.WebsocketProvider(
                endpoint_uri=f"ws://{net_host}:{rpc_ws_port}",
                websocket_timeout=45,
            ))
        assert self._w3_by_type, "No API clients were created from the config - the node is barely testable"
        self.use_w3_provider(default_w3_provider_type)
        self._wait_until_listening()

    @property
    def crashed(self):
        return self._proc is not None and self._proc.poll() is not None

    @property
    def running(self):
        return self._proc is not None and self._proc.poll() is None

    @property
    def data_path(self):
        return self._data_path

    @property
    def rebuild_backups(self):
        return sorted(self._data_path.glob("db.concrete-root-rebuild-backup-*"))

    def _wait_until_listening(self):
        wait(lambda: self.net.listening, fail_immediately=lambda _: self.crashed)

    def start(self, extra_args: Sequence[str] = ()):
        """Start a managed node with its prepared config and optional one-shot CLI arguments."""
        if self._launch_command is None:
            raise RuntimeError("remote nodes cannot be started by the test harness")
        if self.running:
            raise RuntimeError("node is already running")
        self._proc = Popen([*self._launch_command, *extra_args])
        if self.w3 is not None:
            self._wait_until_listening()

    def stop(self, timeout_seconds=20):
        """Gracefully terminate a managed node, escalating to SIGKILL after the timeout."""
        if not self.running:
            return
        self._proc.terminate()
        try:
            self._proc.wait(timeout=timeout_seconds)
        except subprocess.TimeoutExpired:
            self._proc.kill()
            self._proc.wait(timeout=timeout_seconds)

    def kill(self, timeout_seconds=20):
        """Immediately kill a managed node and reap its process."""
        if not self.running:
            return
        self._proc.kill()
        self._proc.wait(timeout=timeout_seconds)

    def restart(self, extra_args: Sequence[str] = (), graceful=True):
        """Restart a managed node over its existing data, optionally adding one-shot CLI arguments."""
        if graceful:
            self.stop()
        else:
            self.kill()
        self.start(extra_args)

    def destructor(self):
        self.stop()

    def use_w3_provider(self, provider_type: str):
        w3 = self._w3_by_type.get(provider_type, None)
        assert w3 is not None, f"{provider_type} w3 provider is not available"
        self.w3, self.eth, self.net = w3, w3.eth, w3.net
        return self
