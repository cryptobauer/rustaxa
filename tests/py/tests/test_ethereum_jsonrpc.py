from common.chain_tester.chain_tester import ChainTester
from common.chain_tester.misc import ContractTxExtraParams, TransactionExpectation
from common.eth.solidity import compile_single
from common.util.pytest import treat_as_warning


@treat_as_warning(TimeoutError)
def test_1(default_cluster):
    cluster = default_cluster
    chain = ChainTester(cluster, assume_no_implicit_transfers=False, auto_test_tx_and_blk_filters=True,
                        default_tx_signer=cluster.node(0).account,
                        expected_block_gas_limit=315_000_000)

    greeting = "Hello"
    greeter = chain.deploy_contract(compile_single('''
contract Greeter {

    event MyEvent(uint val);

    string public greeting;

    constructor(string memory _greeting) public {
       setGreeting(_greeting);
    }

    function setGreeting(string memory _greeting) public {
       greeting = _greeting;
    }

    function greet() view public returns (string memory) {
       return greeting;
    }

    function fireEvent(uint val) public {
        emit MyEvent(val);
    }
}
'''), greeting, tx_extra_params=ContractTxExtraParams(gas=2_000_000))
    # Finalize creation before the same signer submits the transfer burst so
    # DAG ordering cannot place higher nonces ahead of the constructor.
    chain.sync()

    def check_greeting():
        nonlocal greeting
        assert greeting == greeter.call('greet')
        assert cluster.node().eth.get_storage_at(greeter.address, 0).decode().startswith(greeting)

    def set_greeting(val):
        nonlocal greeting
        greeting = val
        return greeter.execute('setGreeting', greeting)

    def send_some_tx(use_node_signers=False):
        for i in range(7):
            node = cluster.node(i % len(cluster))
            chain.coin_transfer(node.account.address, 100, signer=node.account if use_node_signers else None)

    send_some_tx()
    chain.sync()
    check_greeting()
    set_greeting("foo")
    chain.default_tx_signer, cluster.default_node_index = ChainTester.NO_SIGNER, 0
    cluster.use_w3_provider('ws')
    # Exercise WebSocket submission with explicit node keys; production JSON-RPC
    # intentionally does not expose eth_sendTransaction/unlocked accounts.
    send_some_tx(use_node_signers=True)
    chain.sync()
    chain.default_tx_signer = cluster.node(0).account
    check_greeting()
    my_event = greeter.event('MyEvent', node_index=cluster.random_node_index)
    event_filter_all = my_event.filter_tester()
    event_filter_1 = my_event.filter_tester(from_block='earliest', val=1)
    event_trx_futures = [greeter.execute('fireEvent', i, expectation=TransactionExpectation(my_event.called(val=i)))
                         for i in range(3)]
    send_some_tx()
    chain.sync()
    event_filter_1.test_poll(event_trx_futures[1].event_logs)
    event_filter_all.test_poll([fut.event_logs[0] for fut in event_trx_futures])
    for f in [event_filter_1, event_filter_all]:
        f.test_get_filter_by_id()
        f.test_get_all_entries()
        f.test_uninstall()
