#!/usr/bin/env python3
"""Run a cold bounded Go BeginBlock/EndBlock fixture in a pinned disposable archive.

The original checkout and all databases remain untouched. A hash-guarded observer
patch rejects buffered TrieSink mutation attempts; separate process controls prove
that rejection. Authenticated Rust observations are reused as fixture bytes, while
this Go port supplies no authentication, complete snapshot or root derivation.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile

HERE = Path(__file__).resolve().parent
REPO = HERE.parent.parent
SOURCE = Path(os.environ.get("TARAXA_EVM_SOURCE", REPO / "submodules/taraxa-evm"))
REVISION = "6c7e5338b22d5e596cc2365a88d1f94840e1ee1b"
PINS = {
    "doc/evm_research/n4_empty_native_effects.json": "c5cc64181cd6611648c975220d29403eafd3e9d4995b34fd90bf54c03cb870ba",
    "doc/evm_research/n4_retained_dpos_config.json": "3c817ff7974da68e0d5865b34e92765c7e50c71a6892c81d299f28c579e38634",
    "doc/evm_research/n4_independent_reward_plan.json": "f88d67ecbd06d6c483466290262fabe8fb03e4532164cf0311fdd63232bcb0e9",
    "libraries/cli/include/cli/config_jsons/mainnet/mainnet_genesis.json": "7931f2b9b92e018dad154becbf20058d2f97f8ad5d9cc0d4b443da7c72ae3ce7",
    "doc/evm_research/n4_replay_preflight.json": "d68ab554634e7907f2b43ab43f753d6b9351d1b760afc8b91eb3f77c523ae437",
}
TRIE_TARGET = "taraxa/state/state_transition/trie_sink.go"
TRIE_SHA = "90d3ac9dd49e3e78cdbe5c35ac34d1d8d508324c282ee7af22b23c74909f8aa4"
SOURCE_PINS = {
    "taraxa/C/state.go": "b6effa29e1d465a0bcc1badcf398a57e982d16b0d6f4f9b14844632306f7e3ec",
    "taraxa/state/state_transition/state_transition.go": "d447ac4599f0107ccad4379f95a7269a6d5729283d8dcf00db850ea8fe119e90",
    "taraxa/state/state_transition/state_hardforks.go": "1432c0e39e7ecaee8683c3e9fd28571e97f816e906c3938b0f38fa886c246aae",
    "taraxa/state/contracts/dpos/precompiled/dpos_contract.go": "28bb58adb9d99d604fdacf0c284deea49c6887eee789381a5c5a231b5c89f565",
    "taraxa/state/contracts/slashing/precompiled/slashing_contract.go": "2ac21dbe9ece16e11f1ab85d2670488d24a2a0e8940d2c181ced142511c6edfb",
    TRIE_TARGET: TRIE_SHA,
}
OBSERVER = b'''package state_transition

// Disposable-archive validation instrumentation; no original checkout edits.
var emptyEffectsMutationAttempts uint64
func rejectEmptyEffectsMutation() {
    emptyEffectsMutationAttempts++
    panic("empty-effects observer rejects TrieSink mutation attempt")
}
func EmptyEffectsMutationAttempts() uint64 { return emptyEffectsMutationAttempts }
'''


def sha(data):
    return hashlib.sha256(data).hexdigest()


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def pinned_inputs():
    reports = {}
    for path, expected in PINS.items():
        raw = (REPO / path).read_bytes()
        require(sha(raw) == expected, f"input fingerprint drift: {path}")
        reports[path] = json.loads(raw)
    return list(reports.values())


def camel(name):
    return "".join(part.capitalize() for part in name.split("_"))


def integer(value):
    return int(value, 0) if isinstance(value, str) else value


def fixture_input(native, retained, rewards, candidate, preflight):
    require(native["period"] == 25706949 and native["prior_identity"]["period"] == 25706948, "native period drift")
    require(native["tool_source_sha256"] == "43e852fa5162d72e69caa0ed45a3a866aece2951a18958094ea45be967fea0a2", "native source fingerprint drift")
    require(native["qualification"]["conditional_legacy_no_additional_durable_effects_witness"], "conditional native prerequisites failed")
    require(native["read_bound"]["owner_top_level_calls"] == 3, "native read-bound drift")
    account, jail = native["parent_dpos_account"], native["parent_jailed_list"]
    require(account["qualified"] and account["code_size"] == 3000 and account["physical_rlp_hex"], "missing parent DPoS account")
    require(jail["qualified"] and jail["required_value_hex"] == "c0" and jail["authenticated_outcome"] == "Ok(Member([192]))" and jail["physical_outcome"] == "Ok(Present([192]))", "jailed-list membership/value drift")
    for report in (retained, rewards):
        require(report["input_copy"] == native["input_copy"], "mixed working-copy artifacts")
    scalar = retained["retained_dpos_configuration"]["selected"]
    require(scalar["update_period"] == 0 and retained["retained_dpos_configuration"]["record_count"] == 1, "sole retained baseline required")
    require(scalar["scalar_fields"]["delegation_delay"]["unsigned_decimal"] == "5", "retained delay drift")
    planner = rewards["planner_candidate"]
    require(rewards["period"] == 25706949 and planner["cache_current_period"] and not planner["clear_cached_stats"] and planner["distribution_stats_count"] == 0 and planner["concrete_rewards_input_hex"] == "c0", "nonempty/unavailable reward planning")
    require(rewards["independent_artifacts"]["expected_stats_used_for_weight_rate_total_or_author"] is False, "circular planner inputs")
    dpos = {camel(key): int(value["unsigned_decimal"]) for key, value in scalar["scalar_fields"].items()}
    # Candidate genesis inventory is carried verbatim, never synthetic. It is
    # constructor policy input and is not a retained full native-state snapshot.
    dpos["InitialValidators"] = [
        {"Address": v["address"], "Owner": v["owner"], "VrfKey": base64.b64encode(bytes.fromhex(v["vrf_key"])).decode(), "Commission": v["commission"], "Endpoint": v["endpoint"], "Description": v["description"], "Delegations": {"0x" + key.removeprefix("0x"): integer(value) for key, value in v["delegations"].items()}}
        for v in candidate["dpos"]["initial_validators"]
    ]
    forks = {}
    for key, value in candidate["hardforks"].items():
        if key == "redelegations":
            forks[camel(key)] = [{camel(name): field if name in ("validator", "delegator") else integer(field) for name, field in entry.items()} for entry in value]
        elif key == "rewards_distribution_frequency":
            forks[camel(key)] = {str(integer(period)): integer(frequency) for period, frequency in value.items()}
        elif isinstance(value, dict):
            forks[camel(key)] = {camel(name): field if name == "bridge_contract_address" else integer(field) for name, field in value.items()}
        else:
            forks[camel(key)] = integer(value)
    context = preflight["execution_context"]
    return {"parent_period": 25706948, "parent_root": native["prior_identity"]["state_root_hex"], "dpos_physical_rlp": account["physical_rlp_hex"], "jail_physical_rlp": "c0", "author": "0x" + context["pbft_author_hex"], "timestamp": context["pbft_timestamp"], "gas_limit": context["block_gas_limit"], "config": {"EVMChainConfig": {"chainId": candidate["chain_id"]}, "GenesisBalances": {"0x" + key.removeprefix("0x"): integer(value) for key, value in candidate["initial_balances"].items()}, "DPOS": dpos, "Hardforks": forks}}


def patch_archive(tree):
    for target, expected in SOURCE_PINS.items():
        require(sha((tree / target).read_bytes()) == expected, f"immutable source fingerprint drift: {target}")
    target = tree / TRIE_TARGET
    raw = target.read_bytes()
    for needle in (b"func (self *TrieSink) StartMutation(addr *common.Address) state_evm.AccountMutation {\n", b"func (self *TrieSink) Delete(addr *common.Address) {\n"):
        require(raw.count(needle) == 1, "observer insertion point is not unique")
        raw = raw.replace(needle, needle + b"\trejectEmptyEffectsMutation()\n", 1)
    target.write_bytes(raw)
    (target.parent / "empty_effects_observer.go").write_bytes(OBSERVER)
    return {"immutable_target_sha256": TRIE_SHA, "patched_target_sha256": sha(raw), "observer_sha256": sha(OBSERVER), "observation_points": ["TrieSink.StartMutation", "TrieSink.Delete"], "original_checkout_modified": False}


def run_reference():
    inputs = pinned_inputs()
    fixture = fixture_input(*inputs)
    with tempfile.TemporaryDirectory(prefix="rustaxa-empty-native-") as directory:
        tree = Path(directory)
        archive = subprocess.check_output(["git", "-C", str(SOURCE), "archive", REVISION])
        subprocess.run(["tar", "-x", "-C", directory], input=archive, check=True)
        patch = patch_archive(tree)
        command = tree / "cmd/empty_native_effects_reference"
        command.mkdir(parents=True)
        (command / "main.go").write_bytes((HERE / "empty_native_effects_reference.go").read_bytes())
        fixture_file = tree / "empty-native-fixture.json"
        fixture_bytes = (json.dumps(fixture, sort_keys=True, separators=(",", ":")) + "\n").encode()
        fixture_file.write_bytes(fixture_bytes)
        executable = tree / "empty-native-reference"
        subprocess.run(["go", "build", "-mod=readonly", "-o", str(executable), "./cmd/empty_native_effects_reference"], cwd=tree, check=True)
        witness = json.loads(subprocess.check_output([str(executable), "--input", str(fixture_file)], cwd=tree))
        controls = [json.loads(subprocess.check_output([str(executable), "--control", mode], cwd=tree)) for mode in ("start", "delete")]
        require(witness["read_count"] == 2 and witness["backend_put_attempts"] == 0 and witness["commit_attempts"] == 0 and witness["trie_mutation_attempts"] == 0, "cold operation contract failed")
        require(all(control["rejected"] and control["mutation_attempts"] == 1 and control["reason"] == "empty-effects observer rejects TrieSink mutation attempt" for control in controls), "observer negative control failed")
    return {"schema": 1, "immutable_go_revision": REVISION, "go_version": subprocess.check_output(["go", "version"]).decode().strip(), "input_sha256": PINS, "immutable_source_sha256": SOURCE_PINS, "harness_sha256": {name: sha((HERE / name).read_bytes()) for name in ("empty_native_effects_reference.go", "empty_native_effects_reference.py")}, "observer_archive_patch": patch, "fixture_json_sha256": sha(fixture_bytes), "constructor_configuration": {"dpos_scalar_source": "all twelve retained baseline scalar fields", "inventory_and_hardfork_source": "pinned checked-in candidate genesis validators, balances and complete Taraxa hardfork configuration", "ethereum_chain_config_scope": "chainId only; no ordinary transaction or bytecode execution", "initial_validator_count": len(fixture["config"]["DPOS"]["InitialValidators"]), "constructor_config_sha256": sha(json.dumps(fixture["config"], sort_keys=True, separators=(",", ":")).encode()), "producer_configuration_qualified": False, "complete_native_snapshot_reconstructed": False}, "witness": witness, "negative_controls": controls, "scope": "actual cold immutable-Go Init/BeginBlock/EndBlock/Close with observer-only archive instrumentation; fixture bytes reuse earlier Rust authentication; no warm state, transactions, rewards execution, complete snapshot, PrepareCommit, root derivation, production or adoption claim"}


def validated_output(supplied, forbidden=None):
    """Resolve existing parents and reject DB/source aliases or existing files.

    Validation uses filesystem metadata only and writes nothing. Exclusive file
    creation still handles a competing output created after validation.
    """
    parent = supplied.parent.resolve(strict=True)
    output = parent / supplied.name
    roots = forbidden if forbidden is not None else (REPO / "data", REPO / "local/evm-state-db/snapshot-litenode-copy", SOURCE)
    for root in roots:
        require(not output.is_relative_to(root.resolve(strict=True)), "report overlaps protected database/source tree")
    require(not output.exists() and not output.is_symlink(), "report already exists")
    return output


def test_output_guards():
    """Exercise rejection boundaries without building Go or opening databases."""
    with tempfile.TemporaryDirectory(prefix="rustaxa-empty-output-policy-") as directory:
        base = Path(directory)
        roots = tuple(base / name for name in ("data", "copy", "go-source"))
        for root in roots:
            root.mkdir()
        require(validated_output(base / "new.json", roots) == base / "new.json", "new safe output rejected")
        for root in roots:
            alias = base / (root.name + "-alias")
            alias.symlink_to(root, target_is_directory=True)
            for parent in (root, alias):
                try:
                    validated_output(parent / "new.json", roots)
                except RuntimeError:
                    pass
                else:
                    raise RuntimeError("protected tree/source alias was accepted")
        existing = base / "existing.json"
        existing.write_text("preserved")
        dangling = base / "dangling.json"
        dangling.symlink_to(base / "missing")
        for path in (existing, dangling):
            try:
                validated_output(path, roots)
            except RuntimeError:
                pass
            else:
                raise RuntimeError("existing/symlink output was accepted")
        require(existing.read_text() == "preserved", "existing output changed")
    print("Pure output guard checks passed; no Go execution or database opens")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("output", type=Path, nargs="?", help="new report outside database and original Go source trees")
    parser.add_argument("--self-test", action="store_true", help="run pure output policy checks only")
    args = parser.parse_args()
    if args.self_test:
        require(args.output is None, "self-test accepts no output path")
        test_output_guards()
        return
    if args.output is None:
        parser.error("output path is required")
    output = validated_output(args.output)
    report = run_reference()
    with output.open("x") as file:
        json.dump(report, file, indent=2)
        file.write("\n")
    print(f"Cold Go witness and isolated mutation controls passed: {output}")


if __name__ == "__main__":
    main()
