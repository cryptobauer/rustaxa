from common.chain_tester.chain_tester import ChainTester
from common.eth.w3 import ContractFactory
from common.util.asserts import assert_equal
from common.util.wait import Timeout, wait


def _agreed_block(cluster, block_number):
    blocks = [node.eth.get_block(block_number) for node in cluster]
    for field in ("hash", "parentHash", "stateRoot", "transactionsRoot", "receiptsRoot"):
        assert all(block[field] == blocks[0][field] for block in blocks)
    return blocks[0]


def _assert_same_roots(actual, expected):
    for field in ("hash", "parentHash", "stateRoot", "transactionsRoot", "receiptsRoot"):
        assert actual[field] == expected[field]


def _wait_for_agreement(cluster, block_number):
    def matching_block():
        blocks = [node.eth.get_block(block_number) for node in cluster]
        fields = ("hash", "parentHash", "stateRoot", "transactionsRoot", "receiptsRoot")
        if not all(all(block[field] == blocks[0][field] for field in fields) for block in blocks):
            return None
        return blocks[0]

    return wait(matching_block, timeout=Timeout(num_attempts=300, backoff_seconds=1))


def _probe_contract():
    """Return a pinned, compiler-independent single-slot EVM probe."""
    # A 36-byte ABI call stores argument zero; a four-byte getter call returns
    # it. This avoids compiler and post-Constantinople opcode dependencies.
    runtime = bytearray((
        0x36, 0x60, 0x24, 0x14, 0x60, 0x12, 0x57,
        0x60, 0x00, 0x54, 0x60, 0x00, 0x52, 0x60, 0x20, 0x60, 0x00, 0xF3,
        0x5B, 0x60, 0x04, 0x35, 0x60, 0x00, 0x55, 0x00,
    ))
    assert len(runtime) < 256
    init = bytes((0x60, len(runtime), 0x60, 0x0C, 0x60, 0x00, 0x39,
                  0x60, len(runtime), 0x60, 0x00, 0xF3))
    abi = [
        {"inputs": [{"name": "nextValue", "type": "uint256"}], "name": "set", "outputs": [],
         "stateMutability": "nonpayable", "type": "function"},
        {"inputs": [], "name": "value", "outputs": [{"name": "", "type": "uint256"}],
         "stateMutability": "view", "type": "function"},
    ]
    return ContractFactory(bytecode=(init + runtime).hex(), abi=abi)


def test_multi_node_concrete_root_restart_rebuild_and_full_resync(default_cluster):
    """Prove restart durability and clean concrete-root rebuild/resync agreement across a validator quorum."""
    cluster = default_cluster
    chain = ChainTester(cluster, assume_no_implicit_transfers=False,
                        default_tx_signer=cluster.node(0).account,
                        expected_block_gas_limit=315_000_000,
                        require_quiescent_tip=False)

    probe = chain.deploy_contract(_probe_contract())
    chain.sync()
    probe.execute("set", 7)
    chain.sync()

    contract_call = probe.execute("set", 11)
    native_transfer = chain.coin_transfer(cluster.node(1).account.address, 101,
                                          signer=cluster.node(2).account)
    chain.sync()
    durable_block = _agreed_block(cluster, chain.last_blk_num)
    assert probe.call("value") == 11
    for transaction in (contract_call, native_transfer):
        receipts = [cluster_node.eth.get_transaction_receipt(transaction.hash) for cluster_node in cluster]
        assert_equal(receipts)

    cluster.restart_node(0)
    restarted_block = _wait_for_agreement(cluster, durable_block.number)
    _assert_same_roots(restarted_block, durable_block)

    # Prove ordinary crash recovery over the intact database independently of
    # the destructive rebuild path below.
    cluster.kill_node(1)
    cluster.restart_node(1, graceful=False)
    crash_restarted_block = _wait_for_agreement(cluster, durable_block.number)
    _assert_same_roots(crash_restarted_block, durable_block)
    assert probe.call("value", node_index=1) == 11
    for transaction in (contract_call, native_transfer):
        receipts = [cluster_node.eth.get_transaction_receipt(transaction.hash) for cluster_node in cluster]
        assert_equal(receipts)

    post_crash_transfer = chain.coin_transfer(cluster.node(1).account.address, 151,
                                              signer=cluster.node(4).account)
    chain.sync()
    durable_block = _agreed_block(cluster, chain.last_blk_num)
    assert_equal(cluster_node.eth.get_transaction_receipt(post_crash_transfer.hash) for cluster_node in cluster)

    rebuilt_node = cluster.node(2)
    old_backups = set(rebuilt_node.rebuild_backups)
    cluster.kill_node(2)
    cluster.restart_node(2, extra_args=("--rebuild-db",), graceful=False)
    assert set(rebuilt_node.rebuild_backups) - old_backups

    resynced_block = _wait_for_agreement(cluster, durable_block.number)
    _assert_same_roots(resynced_block, durable_block)
    assert probe.call("value", node_index=2) == 11
    for transaction in (contract_call, native_transfer):
        receipts = [cluster_node.eth.get_transaction_receipt(transaction.hash) for cluster_node in cluster]
        assert_equal(receipts)

    chain.coin_transfer(cluster.node(2).account.address, 202,
                        signer=cluster.node(3).account)
    chain.sync()
    _wait_for_agreement(cluster, chain.last_blk_num)
