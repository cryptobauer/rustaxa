package main

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"math/big"
	"sort"
	"strings"

	"github.com/Taraxa-project/taraxa-evm/accounts/abi"
	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/crypto"
	"github.com/Taraxa-project/taraxa-evm/crypto/secp256k1"
	"github.com/Taraxa-project/taraxa-evm/params"
	"github.com/Taraxa-project/taraxa-evm/rlp"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/chain_config"
	dpos "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/precompiled"
	dpos_sol "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/solidity"
	contract_storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/rewards_stats"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_transition"
)

const (
	fullTransactionGas = uint64(500_000)
	fullBlockGas       = uint64(10_000_000)
)

var (
	fullDelegatorKey   = bytes.Repeat([]byte{0x02}, 32)
	fullValidatorKey   = bytes.Repeat([]byte{0x09}, 32)
	initialDelegator   = common.HexToAddress("0x0000000000000000000000000000000000000032")
	missingBeneficiary = common.HexToAddress("0x00000000000000000000000000000000000000dd")
)

type bytecodeAssembly struct {
	code    []byte
	labels  map[string]int
	patches map[int]string
}

func newAssembly() *bytecodeAssembly {
	return &bytecodeAssembly{labels: make(map[string]int), patches: make(map[int]string)}
}

func (a *bytecodeAssembly) emit(code ...byte) { a.code = append(a.code, code...) }

func (a *bytecodeAssembly) label(name string) {
	if _, present := a.labels[name]; present {
		panic("duplicate bytecode label " + name)
	}
	a.labels[name] = len(a.code)
}

func (a *bytecodeAssembly) pushLabel(name string) {
	a.emit(0x61, 0, 0)
	a.patches[len(a.code)-2] = name
}

func (a *bytecodeAssembly) bytes() []byte {
	ret := common.CopyBytes(a.code)
	for offset, name := range a.patches {
		value, present := a.labels[name]
		if !present || value > 0xffff {
			panic("missing or oversized bytecode label " + name)
		}
		ret[offset] = byte(value >> 8)
		ret[offset+1] = byte(value)
	}
	return ret
}

func pushUint(value uint64) []byte {
	width := 1
	for shifted := value >> 8; shifted != 0; shifted >>= 8 {
		width++
	}
	ret := []byte{byte(0x5f + width)}
	for index := width - 1; index >= 0; index-- {
		ret = append(ret, byte(value>>uint(index*8)))
	}
	return ret
}

func initRuntime(runtime []byte, stores ...[2]byte) []byte {
	prefix := make([]byte, 0, len(stores)*5)
	for _, store := range stores {
		prefix = append(prefix, 0x60, store[1], 0x60, store[0], 0x55)
	}
	// PUSH2 length, PUSH2 offset, PUSH1 0, CODECOPY, PUSH2 length,
	// PUSH1 0, RETURN. PUSH2 keeps later workload runtimes representable.
	offset := len(prefix) + 15
	if len(runtime) > 0xffff || offset > 0xffff {
		panic("workload runtime exceeds PUSH2 initcode bounds")
	}
	header := []byte{
		0x61, byte(len(runtime) >> 8), byte(len(runtime)), 0x61, byte(offset >> 8), byte(offset),
		0x60, 0, 0x39, 0x61, byte(len(runtime) >> 8), byte(len(runtime)), 0x60, 0, 0xf3,
	}
	return append(append(prefix, header...), runtime...)
}

func lifecycleRuntime(beneficiary common.Address) []byte {
	// Empty calldata stores 7 in slot 0; any nonempty calldata deletes the
	// contract through the selected existing beneficiary.
	ret := []byte{0x36, 0x15, 0x60, 0x1b, 0x57, 0x73}
	ret = append(ret, beneficiary[:]...)
	return append(ret, 0xff, 0x5b, 0x60, 7, 0x60, 0, 0x55, 0)
}

func restoredSlotRuntime() []byte {
	// Empty calldata writes slot 1 and calls itself with one byte. The nested
	// path writes slot 0 then reverts, restoring the original value 7.
	return mustDecodeHex("3615600f57600860005560006000fd5b60096001556001600053600060006001600060003061fffff15000")
}

func selfDestructRuntime(beneficiary common.Address) []byte {
	ret := append([]byte{0x73}, beneficiary[:]...)
	return append(ret, 0xff)
}

func revertingSelfDestructCaller(child common.Address) []byte {
	ret := mustDecodeHex("60006000600060006000")
	ret = append(ret, 0x73)
	ret = append(ret, child[:]...)
	return append(ret, mustDecodeHex("61fffff15060006000fd")...)
}

func dposCalldata(method string, args ...any) []byte {
	parsed, err := abi.JSON(strings.NewReader(dpos_sol.TaraxaDposClientMetaData))
	must(err)
	ret, err := parsed.Pack(method, args...)
	must(err)
	return ret
}

func emitCodeCopy(a *bytecodeAssembly, label string, size, memoryOffset uint64) {
	a.emit(pushUint(size)...)
	a.pushLabel(label)
	a.emit(pushUint(memoryOffset)...)
	a.emit(0x39)
}

func emitCall(a *bytecodeAssembly, target common.Address, value, inputOffset, inputSize, outputOffset, outputSize, gas uint64) {
	a.emit(pushUint(outputSize)...)
	a.emit(pushUint(outputOffset)...)
	a.emit(pushUint(inputSize)...)
	a.emit(pushUint(inputOffset)...)
	a.emit(pushUint(value)...)
	a.emit(0x73)
	a.emit(target[:]...)
	a.emit(pushUint(gas)...)
	a.emit(0xf1)
}

type dispatcherMode struct {
	mode       byte
	method     string
	args       []any
	value      uint64
	parentFail bool
	second     *dispatcherMode
}

func nativeOwnerDispatcher(validator common.Address) ([]byte, map[string][]byte) {
	modes := []dispatcherMode{
		{mode: 1, method: "setCommission", args: []any{validator, uint16(100)}},
		{mode: 2, method: "setCommission", args: []any{validator, uint16(200)}},
		{mode: 3, method: "setCommission", args: []any{validator, uint16(300)}, second: &dispatcherMode{method: "setCommission", args: []any{validator, uint16(400)}}},
		{mode: 5, method: "setCommission", args: []any{validator, uint16(500)}},
		{mode: 6, method: "setCommission", args: []any{validator, uint16(600)}},
		{mode: 7, method: "setCommission", args: []any{validator, uint16(700)}},
		{mode: 8, method: "delegate", args: []any{validator}, value: 500, parentFail: true},
		{mode: 9, method: "undelegateV2", args: []any{validator, big.NewInt(500)}},
		{mode: 10, method: "confirmUndelegateV2", args: []any{validator, uint64(1)}, parentFail: true},
		{mode: 11, method: "confirmUndelegateV2", args: []any{validator, uint64(1)}},
	}
	a := newAssembly()
	// Extract calldata byte zero with BYTE and retain it through the jump table.
	a.emit(0x60, 0, 0x35, 0x60, 0, 0x1a)
	for _, mode := range modes {
		a.emit(0x80, 0x60, mode.mode, 0x14)
		a.pushLabel(fmt.Sprintf("mode_%02x", mode.mode))
		a.emit(0x57)
	}
	a.emit(0x50, 0x60, 0, 0x60, 0, 0xfd)
	payloads := make(map[string][]byte)
	for _, mode := range modes {
		name := fmt.Sprintf("mode_%02x", mode.mode)
		firstData := name + "_first"
		payloads[firstData] = dposCalldata(mode.method, mode.args...)
		a.label(name)
		a.emit(0x5b, 0x50)
		emitCodeCopy(a, firstData, uint64(len(payloads[firstData])), 256)
		emitCall(a, *dpos.ContractAddress(), mode.value, 256, uint64(len(payloads[firstData])), 0, 64, 200_000)
		a.emit(pushUint(64)...)
		a.emit(0x52)
		if mode.second != nil {
			secondData := name + "_second"
			payloads[secondData] = dposCalldata(mode.second.method, mode.second.args...)
			emitCodeCopy(a, secondData, uint64(len(payloads[secondData])), 256)
			emitCall(a, *dpos.ContractAddress(), mode.second.value, 256, uint64(len(payloads[secondData])), 96, 64, 200_000)
			a.emit(pushUint(160)...)
			a.emit(0x52)
		}
		if mode.parentFail {
			a.emit(0x60, 0, 0x60, 0, 0xfd)
		} else if mode.second != nil {
			a.emit(pushUint(192)...)
			a.emit(0x60, 0, 0xf3)
		} else {
			a.emit(pushUint(96)...)
			a.emit(0x60, 0, 0xf3)
		}
	}
	keys := make([]string, 0, len(payloads))
	for key := range payloads {
		keys = append(keys, key)
	}
	sort.Strings(keys)
	for _, key := range keys {
		a.label(key)
		a.emit(payloads[key]...)
	}
	return a.bytes(), payloads
}

func callDispatcherAndRevert(dispatcher common.Address, mode byte) []byte {
	ret := []byte{0x60, mode, 0x60, 0, 0x53}
	emitter := newAssembly()
	emitter.emit(ret...)
	emitCall(emitter, dispatcher, 0, 0, 1, 0, 192, 200_000)
	emitter.emit(0x50, 0x60, 0, 0x60, 0, 0xfd)
	return emitter.bytes()
}

func mixedStatelessRuntime(dispatcher common.Address) ([]byte, []byte) {
	modexp := append(make([]byte, 31), 1)
	modexp = append(modexp, append(make([]byte, 31), 1)...)
	modexp = append(modexp, append(make([]byte, 31), 1)...)
	modexp = append(modexp, 2, 5, 13)
	a := newAssembly()
	// SHA-256("abc") -> memory[64:96], flag -> memory[448:480].
	a.emit(0x62, 'a', 'b', 'c', 0x60, 0, 0x52)
	emitCall(a, common.BytesToAddress([]byte{2}), 0, 29, 3, 64, 32, 50_000)
	a.emit(pushUint(448)...)
	a.emit(0x52)
	// Owner-dispatch mode 6 -> memory[96:192], flag -> memory[480:512].
	a.emit(0x60, 6, 0x60, 0, 0x53)
	emitCall(a, dispatcher, 0, 0, 1, 96, 96, 200_000)
	a.emit(pushUint(480)...)
	a.emit(0x52)
	// MODEXP 2^5 mod 13 -> memory[400], flag -> memory[512:544].
	emitCodeCopy(a, "modexp", uint64(len(modexp)), 256)
	emitCall(a, common.BytesToAddress([]byte{5}), 0, 256, uint64(len(modexp)), 400, 1, 50_000)
	a.emit(pushUint(512)...)
	a.emit(0x52)
	// Owner-dispatch mode 7 -> memory[192:288], flag -> memory[544:576].
	a.emit(0x60, 7, 0x60, 0, 0x53)
	emitCall(a, dispatcher, 0, 0, 1, 192, 96, 200_000)
	a.emit(pushUint(544)...)
	a.emit(0x52)
	// Return SHA, padded MODEXP output, and the four CALL flags.
	for _, copy := range [][2]uint64{{64, 0}, {400, 32}, {448, 64}, {480, 96}, {512, 128}, {544, 160}} {
		a.emit(pushUint(copy[0])...)
		a.emit(0x51)
		a.emit(pushUint(copy[1])...)
		a.emit(0x52)
	}
	a.emit(pushUint(192)...)
	a.emit(0x60, 0, 0xf3)
	a.label("modexp")
	a.emit(modexp...)
	return a.bytes(), modexp
}

func addressForPrivateKey(key []byte) common.Address {
	x, y := secp256k1.S256().ScalarBaseMult(key)
	publicKey := secp256k1.S256().Marshal(x, y)
	return common.BytesToAddress(crypto.Keccak256(publicKey[1:])[12:])
}

func fullWitnessConfig(sender, secondSigner, validator, owner common.Address) chain_config.ChainConfig {
	return chain_config.ChainConfig{
		EVMChainConfig: paramsChainConfig(),
		GenesisBalances: core.BalanceMap{
			sender:           big.NewInt(100_000_000),
			secondSigner:     big.NewInt(2_000_000),
			initialDelegator: big.NewInt(2_000),
		},
		DPOS: chain_config.DPOSConfig{
			EligibilityBalanceThreshold: big.NewInt(100), VoteEligibilityBalanceStep: big.NewInt(10),
			ValidatorMaximumStake: big.NewInt(1_000_000), MinimumDeposit: big.NewInt(1),
			MaxBlockAuthorReward: 10, DagProposersReward: 50,
			CommissionChangeDelta: 1_000, CommissionChangeFrequency: 0,
			DelegationDelay: 1, DelegationLockingPeriod: 1, BlocksPerYear: 1, YieldPercentage: 20,
			InitialValidators: []chain_config.GenesisValidator{{
				Address: validator, Owner: owner, VrfKey: bytes.Repeat([]byte{0x44}, 32),
				Delegations: core.BalanceMap{initialDelegator: big.NewInt(1_000)},
			}},
		},
		Hardforks: fullHardforks(),
	}
}

func paramsChainConfig() params.ChainConfig { return params.ChainConfig{ChainId: chainID} }

func fullHardforks() chain_config.HardforksConfig {
	return chain_config.HardforksConfig{
		FixRedelegateBlockNum: 0, RewardsDistributionFrequency: map[uint64]uint32{0: 1},
		MagnoliaHf:             chain_config.MagnoliaHfConfig{BlockNum: 0, JailTime: 1},
		PhalaenopsisHfBlockNum: 0, FixClaimAllBlockNum: 0,
		AspenHf:      chain_config.AspenHfConfig{BlockNumPartOne: 0, BlockNumPartTwo: maxPeriod, MaxSupply: big.NewInt(122_000_000), GeneratedRewards: new(big.Int)},
		FicusHf:      chain_config.FicusHfConfig{BlockNum: 0, PillarBlocksInterval: 1_000},
		CornusHf:     chain_config.CornusHfConfig{BlockNum: 0, DelegationLockingPeriod: 1, DagGasLimit: fullBlockGas, PbftGasLimit: fullBlockGas},
		SoleiroliaHf: chain_config.SoleiroliaHfConfig{BlockNum: maxPeriod},
		CactiHf:      chain_config.CactiHfConfig{BlockNum: maxPeriod, DelegationLockingPeriod: 1, JailTime: 1},
	}
}

type fullTransactionSpec struct {
	transactionSpec
	PrivateKey       []byte
	ExpectedError    string
	ExpectedContract *common.Address
	ExpectedOutput   []byte
}

func fullSpec(name string, nonce uint64, signer common.Address, key []byte, to *common.Address, value uint64, input []byte) fullTransactionSpec {
	if addressForPrivateKey(key) != signer {
		panic("full transaction signer does not match its private key")
	}
	return fullTransactionSpec{transactionSpec: transactionSpec{
		Name: name, Nonce: nonce, GasPrice: 1, Gas: fullTransactionGas, To: to, Value: value, Input: input,
	}, PrivateKey: key}
}

func mustDecodeHex(value string) []byte {
	ret, err := hex.DecodeString(value)
	must(err)
	return ret
}

func fullConfigurationJSON(cfg chain_config.ChainConfig) map[string]any {
	ret := configurationJSON(cfg, 1)
	ret["transaction_gas_limit"] = fullTransactionGas
	ret["block_gas_limit"] = fullBlockGas
	ret["declared_genesis_balance_total"] = "102002000"
	ret["selected_periods_are_pillar_boundaries"] = false
	return ret
}

func observeAddressSet(reader state_db.ExtendedReader, addresses []common.Address) []map[string]any {
	unique := make(map[common.Address]struct{})
	for _, address := range addresses {
		unique[address] = struct{}{}
	}
	ordered := make([]common.Address, 0, len(unique))
	for address := range unique {
		ordered = append(ordered, address)
	}
	sort.Slice(ordered, func(i, j int) bool { return string(ordered[i][:]) < string(ordered[j][:]) })
	ret := make([]map[string]any, len(ordered))
	for index, address := range ordered {
		ret[index] = observeAccountReader(reader, address)
	}
	return ret
}

func observeStorageSlot(reader state_db.ExtendedReader, address common.Address, key common.Hash) map[string]any {
	ret := map[string]any{
		"address": hex.EncodeToString(address[:]), "key": hex.EncodeToString(key[:]),
		"present": false, "raw_value": "",
	}
	reader.GetAccountStorage(&address, &key, func(value []byte) {
		if len(value) == 0 {
			return
		}
		ret["present"] = true
		ret["raw_value"] = hex.EncodeToString(value)
	})
	return ret
}

func requireObserved(observation map[string]any, present bool, rawValue string, label string) {
	if observation["present"] != present || observation["raw_value"] != rawValue {
		panic(fmt.Sprintf("%s observation differs: %v", label, observation))
	}
}

func requireObservedAccount(observation map[string]any, present bool, balance string, label string) {
	if observation["present"] != present {
		panic(fmt.Sprintf("%s presence differs: %v", label, observation))
	}
	if present && observation["balance"] != balance {
		panic(fmt.Sprintf("%s balance differs: %v", label, observation))
	}
}

func fullTransactionRow(spec fullTransactionSpec, signed signedTransaction, tx vm.Transaction, result vm.ExecutionResult, cumulativeGas uint64) map[string]any {
	if result.ConsensusErr != "" {
		panic(fmt.Sprintf("%s consensus failure: %q", spec.Name, result.ConsensusErr))
	}
	if spec.ExpectedError == "" {
		if result.ExecutionErr != "" {
			panic(fmt.Sprintf("%s unexpected execution failure: %q", spec.Name, result.ExecutionErr))
		}
	} else if !strings.Contains(string(result.ExecutionErr), spec.ExpectedError) {
		panic(fmt.Sprintf("%s execution failure %q does not contain %q", spec.Name, result.ExecutionErr, spec.ExpectedError))
	}
	if spec.ExpectedOutput != nil && !bytes.Equal(result.CodeRetval, spec.ExpectedOutput) {
		panic(fmt.Sprintf("%s output %x, want %x", spec.Name, result.CodeRetval, spec.ExpectedOutput))
	}
	status := uint8(1)
	if result.ExecutionErr != "" {
		status = 0
	}
	if tx.From != signed.Sender || !sameAddress(tx.To, signed.Decoded.To) || tx.Nonce.Cmp(signed.Decoded.Nonce) != 0 ||
		tx.GasPrice.Cmp(signed.Decoded.GasPrice) != 0 || tx.Gas != signed.Decoded.Gas ||
		tx.Value.Cmp(signed.Decoded.Value) != 0 || !bytes.Equal(tx.Input, signed.Decoded.Input) {
		panic("full StateAPI transaction does not match its decoded signed transaction")
	}
	logs := make([]map[string]any, 0, len(result.Logs))
	for _, log := range result.Logs {
		topics := make([]string, len(log.Topics))
		for index, topic := range log.Topics {
			topics[index] = hex.EncodeToString(topic[:])
		}
		logs = append(logs, map[string]any{
			"address": hex.EncodeToString(log.Address[:]), "topics": topics, "data": hex.EncodeToString(log.Data),
		})
	}
	var created *common.Address
	if spec.To == nil && status == 1 {
		created = &result.NewContractAddr
		if spec.ExpectedContract != nil && result.NewContractAddr != *spec.ExpectedContract {
			panic(fmt.Sprintf("%s created %x, want %x", spec.Name, result.NewContractAddr, *spec.ExpectedContract))
		}
	}
	receipt := externalReceipt{
		Status: status, GasUsed: result.GasUsed, CumulativeGasUsed: cumulativeGas,
		Logs: result.Logs, NewContract: created,
	}
	return map[string]any{
		"name": spec.Name, "nonce": spec.Nonce, "to": addressHex(spec.To),
		"sender": hex.EncodeToString(signed.Sender[:]), "chain_id": signed.ChainID,
		"value": signed.Decoded.Value.String(), "gas_price": signed.Decoded.GasPrice.String(),
		"gas_limit": signed.Decoded.Gas, "input": hex.EncodeToString(signed.Decoded.Input),
		"signed_rlp": hex.EncodeToString(signed.RLP), "hash": hex.EncodeToString(signed.Hash[:]),
		"v": signed.Decoded.V.String(), "r": signed.Decoded.R.String(), "s": signed.Decoded.S.String(),
		"state_api_transaction_rlp":      hex.EncodeToString(rlp.MustEncodeToBytes(&tx)),
		"state_api_execution_result_rlp": hex.EncodeToString(rlp.MustEncodeToBytes(&result)),
		"receipt_rlp":                    hex.EncodeToString(rlp.MustEncodeToBytes(&receipt)),
		"status":                         status, "gas_used": result.GasUsed, "cumulative_gas_used": cumulativeGas,
		"output": hex.EncodeToString(result.CodeRetval), "created": hex.EncodeToString(result.NewContractAddr[:]),
		"logs": logs, "execution_error": string(result.ExecutionErr), "consensus_error": string(result.ConsensusErr),
	}
}

type fullPeriodInput struct {
	number       uint64
	transactions []fullTransactionSpec
}

type rawWrite struct {
	Address string `json:"address"`
	Key     string `json:"key"`
	Value   string `json:"value"`
}

type gasRefund struct {
	Counter uint64 `json:"counter"`
	Applied uint64 `json:"applied"`
}

func attachRawWrites(target map[string]any, writes []rawWrite) {
	if rawWriteTraceAvailable() {
		target["ordered_raw_writes"] = writes
	}
}

func catalogIdentityClosure(identities catalogIdentities) catalogIdentities {
	identities.Invocations = nil
	return identities
}

func transactionModeInput(mode byte) []byte { return []byte{mode} }

func abiWord(value byte) []byte {
	ret := make([]byte, 32)
	ret[31] = value
	return ret
}

func dispatcherOutput(first []byte, firstSuccess byte, second []byte, secondSuccess *byte) []byte {
	ret := make([]byte, 96)
	copy(ret, first)
	copy(ret[64:], abiWord(firstSuccess))
	if secondSuccess != nil {
		ret = append(ret, make([]byte, 96)...)
		copy(ret[96:], second)
		copy(ret[160:], abiWord(*secondSuccess))
	}
	return ret
}

func statelessInterleaveOutput() []byte {
	ret := make([]byte, 192)
	digest := sha256.Sum256([]byte("abc"))
	copy(ret, digest[:])
	ret[32] = 6
	for offset := 64; offset < len(ret); offset += 32 {
		copy(ret[offset:], abiWord(1))
	}
	return ret
}

func fullPeriodInputs(sender, secondSigner, validator common.Address) ([]fullPeriodInput, map[string]any, []common.Address) {
	contractA := crypto.CreateAddress(&sender, big.NewInt(1))
	contractB := crypto.CreateAddress(&sender, big.NewInt(3))
	childC := crypto.CreateAddress(&sender, big.NewInt(5))
	parentD := crypto.CreateAddress(&sender, big.NewInt(6))
	dispatcherE := crypto.CreateAddress(&sender, big.NewInt(8))
	parentF := crypto.CreateAddress(&sender, big.NewInt(9))
	statelessG := crypto.CreateAddress(&sender, big.NewInt(10))
	lifecycle := lifecycleRuntime(recipient)
	restored := restoredSlotRuntime()
	child := selfDestructRuntime(missingBeneficiary)
	parent := revertingSelfDestructCaller(childC)
	dispatcher, dispatcherPayloads := nativeOwnerDispatcher(validator)
	revertingNative := callDispatcherAndRevert(dispatcherE, 5)
	stateless, modexp := mixedStatelessRuntime(dispatcherE)
	setCommission := func(value uint16) []byte { return dposCalldata("setCommission", validator, value) }
	directOOGInput := setCommission(300)
	intrinsicGas, err := vm.IntrinsicGas(directOOGInput, false)
	must(err)
	dposAddress := *dpos.ContractAddress()
	periods := []fullPeriodInput{
		{number: 1, transactions: []fullTransactionSpec{
			fullSpec("transfer", 0, sender, testPrivateKey, &recipient, 7, nil),
			{transactionSpec: transactionSpec{Name: "create_lifecycle", Nonce: 1, GasPrice: 1, Gas: fullTransactionGas, Input: initRuntime(lifecycle, [2]byte{0, 1})}, PrivateKey: testPrivateKey, ExpectedContract: &contractA},
			fullSpec("write_lifecycle_slot", 2, sender, testPrivateKey, &contractA, 0, nil),
			{transactionSpec: transactionSpec{Name: "create_restored_slot", Nonce: 3, GasPrice: 1, Gas: fullTransactionGas, Input: initRuntime(restored, [2]byte{0, 7})}, PrivateKey: testPrivateKey, ExpectedContract: &contractB},
			fullSpec("restored_slot_selfcall", 4, sender, testPrivateKey, &contractB, 0, nil),
			{transactionSpec: transactionSpec{Name: "create_suicide_child", Nonce: 5, GasPrice: 1, Gas: fullTransactionGas, Value: 7, Input: initRuntime(child)}, PrivateKey: testPrivateKey, ExpectedContract: &childC},
			{transactionSpec: transactionSpec{Name: "create_reverting_parent", Nonce: 6, GasPrice: 1, Gas: fullTransactionGas, Input: initRuntime(parent)}, PrivateKey: testPrivateKey, ExpectedContract: &parentD},
			func() fullTransactionSpec {
				s := fullSpec("parent_revert_child_selfdestruct", 7, sender, testPrivateKey, &parentD, 0, nil)
				s.ExpectedError = "execution reverted"
				return s
			}(),
			{transactionSpec: transactionSpec{Name: "create_native_dispatcher", Nonce: 8, GasPrice: 1, Gas: fullTransactionGas, Input: initRuntime(dispatcher)}, PrivateKey: testPrivateKey, ExpectedContract: &dispatcherE},
			{transactionSpec: transactionSpec{Name: "create_native_reverter", Nonce: 9, GasPrice: 1, Gas: fullTransactionGas, Input: initRuntime(revertingNative)}, PrivateKey: testPrivateKey, ExpectedContract: &parentF},
			{transactionSpec: transactionSpec{Name: "create_stateless_interleave", Nonce: 10, GasPrice: 1, Gas: fullTransactionGas, Input: initRuntime(stateless)}, PrivateKey: testPrivateKey, ExpectedContract: &statelessG},
		}},
		{number: 2, transactions: []fullTransactionSpec{
			func() fullTransactionSpec {
				s := fullSpec("set_commission_100", 11, sender, testPrivateKey, &dispatcherE, 0, transactionModeInput(1))
				s.ExpectedOutput = dispatcherOutput(nil, 1, nil, nil)
				return s
			}(),
			func() fullTransactionSpec {
				s := fullSpec("set_commission_wrong_owner", 0, secondSigner, fullDelegatorKey, &dposAddress, 0, setCommission(800))
				s.ExpectedError = "not owner"
				return s
			}(),
			func() fullTransactionSpec {
				s := fullSpec("set_commission_native_oog", 12, sender, testPrivateKey, &dposAddress, 0, directOOGInput)
				s.Gas = intrinsicGas + 19_999
				s.ExpectedError = "out of gas"
				return s
			}(),
			func() fullTransactionSpec {
				s := fullSpec("set_commission_200", 13, sender, testPrivateKey, &dispatcherE, 0, transactionModeInput(2))
				s.ExpectedOutput = dispatcherOutput(nil, 1, nil, nil)
				return s
			}(),
			func() fullTransactionSpec {
				s := fullSpec("set_commission_multi_300_400", 14, sender, testPrivateKey, &dispatcherE, 0, transactionModeInput(3))
				one := byte(1)
				s.ExpectedOutput = dispatcherOutput(nil, 1, nil, &one)
				return s
			}(),
			func() fullTransactionSpec {
				s := fullSpec("set_commission_500_parent_revert", 15, sender, testPrivateKey, &parentF, 0, nil)
				s.ExpectedError = "execution reverted"
				return s
			}(),
			func() fullTransactionSpec {
				s := fullSpec("stateless_native_interleave_600_700", 16, sender, testPrivateKey, &statelessG, 0, nil)
				s.ExpectedOutput = statelessInterleaveOutput()
				return s
			}(),
			fullSpec("new_delegator_delegate_500", 1, secondSigner, fullDelegatorKey, &dposAddress, 500, dposCalldata("delegate", validator)),
			func() fullTransactionSpec {
				s := fullSpec("native_delegate_500_parent_revert", 17, sender, testPrivateKey, &dispatcherE, 500, transactionModeInput(8))
				s.ExpectedError = "execution reverted"
				return s
			}(),
			fullSpec("delete_lifecycle", 18, sender, testPrivateKey, &contractA, 0, []byte{1}),
		}},
		{number: 3, transactions: []fullTransactionSpec{
			fullSpec("observe_deleted_lifecycle", 19, sender, testPrivateKey, &contractA, 0, nil),
			func() fullTransactionSpec {
				s := fullSpec("new_delegator_undelegate_v2_500", 2, secondSigner, fullDelegatorKey, &dposAddress, 0, dposCalldata("undelegateV2", validator, big.NewInt(500)))
				s.ExpectedOutput = abiWord(1)
				return s
			}(),
			func() fullTransactionSpec {
				s := fullSpec("new_delegator_early_confirm", 3, secondSigner, fullDelegatorKey, &dposAddress, 0, dposCalldata("confirmUndelegateV2", validator, uint64(1)))
				s.ExpectedError = "not yet ready"
				return s
			}(),
			func() fullTransactionSpec {
				s := fullSpec("native_undelegate_v2_500", 20, sender, testPrivateKey, &dispatcherE, 0, transactionModeInput(9))
				s.ExpectedOutput = dispatcherOutput(abiWord(1), 1, nil, nil)
				return s
			}(),
			func() fullTransactionSpec {
				s := fullSpec("native_early_confirm_parent_revert", 21, sender, testPrivateKey, &dispatcherE, 0, transactionModeInput(10))
				s.ExpectedError = "execution reverted"
				return s
			}(),
		}},
		{number: 4, transactions: []fullTransactionSpec{
			fullSpec("new_delegator_confirm", 4, secondSigner, fullDelegatorKey, &dposAddress, 0, dposCalldata("confirmUndelegateV2", validator, uint64(1))),
			func() fullTransactionSpec {
				s := fullSpec("native_confirm_parent_revert", 22, sender, testPrivateKey, &dispatcherE, 0, transactionModeInput(10))
				s.ExpectedError = "execution reverted"
				return s
			}(),
			func() fullTransactionSpec {
				s := fullSpec("native_repeat_confirm_missing", 23, sender, testPrivateKey, &dispatcherE, 0, transactionModeInput(11))
				s.ExpectedOutput = dispatcherOutput(nil, 0, nil, nil)
				return s
			}(),
			fullSpec("restored_slot_after_reopen", 24, sender, testPrivateKey, &contractB, 0, nil),
			fullSpec("continuation_transfer", 25, sender, testPrivateKey, &recipient, 1, nil),
		}},
	}
	bytecode := map[string]any{
		"contract_addresses": map[string]string{
			"lifecycle_a": hex.EncodeToString(contractA[:]), "restored_slot_b": hex.EncodeToString(contractB[:]),
			"suicide_child_c": hex.EncodeToString(childC[:]), "reverting_parent_d": hex.EncodeToString(parentD[:]),
			"native_dispatcher_e": hex.EncodeToString(dispatcherE[:]), "native_reverter_f": hex.EncodeToString(parentF[:]),
			"stateless_interleave_g": hex.EncodeToString(statelessG[:]),
		},
		"lifecycle_runtime": hex.EncodeToString(lifecycle), "restored_slot_runtime": hex.EncodeToString(restored),
		"suicide_child_runtime": hex.EncodeToString(child), "reverting_parent_runtime": hex.EncodeToString(parent),
		"native_dispatcher_runtime": hex.EncodeToString(dispatcher), "native_reverter_runtime": hex.EncodeToString(revertingNative),
		"stateless_interleave_runtime": hex.EncodeToString(stateless), "modexp_input": hex.EncodeToString(modexp),
		"native_dispatcher_payloads": func() map[string]string {
			ret := make(map[string]string)
			for key, value := range dispatcherPayloads {
				ret[key] = hex.EncodeToString(value)
			}
			return ret
		}(),
	}
	addresses := []common.Address{sender, secondSigner, initialDelegator, validator, recipient, missingBeneficiary, contractA, contractB, childC, parentD, dispatcherE, parentF, statelessG, dposAddress}
	return periods, bytecode, addresses
}

func executeFullPeriod(st *state_transition.StateTransition, latest *memoryLatest, cfg chain_config.ChainConfig, observer bool, input fullPeriodInput, validator common.Address, allIdentities *catalogIdentities, observedAddresses []common.Address) map[string]any {
	beginRawWriteTrace()
	st.BeginBlock(&vm.BlockInfo{Author: validator, GasLimit: fullBlockGas, Difficulty: new(big.Int)})
	beginBlockWrites := finishRawWriteTrace()
	rows := make([]map[string]any, 0, len(input.transactions))
	hashes := make([]string, 0, len(input.transactions))
	fees := new(big.Int)
	var cumulativeGas uint64
	for index, spec := range input.transactions {
		sender := addressForPrivateKey(spec.PrivateKey)
		signed := signTransaction(spec.transactionSpec, sender, spec.PrivateKey)
		tx := transactionFromSigned(spec.transactionSpec, signed, sender)
		beginRawWriteTrace()
		beginGasRefundTrace()
		result := st.ExecuteTransaction(&tx)
		refund := finishGasRefundTrace()
		transactionWrites := finishRawWriteTrace()
		cumulativeGas += result.GasUsed
		row := fullTransactionRow(spec, signed, tx, result, cumulativeGas)
		row["index"] = index
		row["fee"] = new(big.Int).Mul(new(big.Int).SetUint64(result.GasUsed), tx.GasPrice).String()
		attachRawWrites(row, transactionWrites)
		if rawWriteTraceAvailable() {
			row["gas_refund"] = refund
		}
		fees.Add(fees, new(big.Int).Mul(new(big.Int).SetUint64(result.GasUsed), tx.GasPrice))
		hashes = append(hashes, hex.EncodeToString(signed.Hash[:]))
		if observer {
			root, identities, reader := observerFinalizeTransaction(st, uint64(index))
			*allIdentities = mergeCatalogs(*allIdentities, catalogIdentityClosure(identities))
			row["observer"] = map[string]any{
				"root": hex.EncodeToString(root[:]), "catalog": captureCatalog(reader, identities),
				"catalog_identities": jsonValue(identities),
				"accounts":           observeAddressSet(reader, observedAddresses),
			}
		}
		rows = append(rows, row)
	}
	storageFactory := func(period types.BlockNum) contract_storage.StorageReader {
		return state_db.ExtendedReader{Reader: latest.readerAt(period)}
	}
	priorDpos := new(dpos.API).Init(cfg).NewReader(types.BlockNum(input.number-1), storageFactory)
	eligibleVotes := priorDpos.TotalEligibleVoteCount()
	validatorVotes := priorDpos.GetEligibleVoteCount(&validator)
	if eligibleVotes == 0 || validatorVotes == 0 {
		panic(fmt.Sprintf("period %d has no eligible Go DPoS votes", input.number))
	}
	stats := rewards_stats.RewardsStats{
		BlockAuthor: validator, BlocksPerYear: cfg.DPOS.BlocksPerYear,
		ValidatorsStats: map[common.Address]rewards_stats.ValidatorStats{
			validator: {DagBlocksCount: 1, VoteWeight: 1, FeesRewards: new(big.Int).Set(fees)},
		},
		TotalDagBlocksCount: 1, TotalVotesWeight: 1, MaxVotesWeight: 1,
	}
	beginRawWriteTrace()
	minted := st.DistributeRewards(&stats)
	rewardWrites := finishRawWriteTrace()
	if minted == nil || minted.IsZero() {
		panic(fmt.Sprintf("period %d produced no minted reward", input.number))
	}
	beginRawWriteTrace()
	st.EndBlock()
	endBlockWrites := finishRawWriteTrace()
	if observer {
		observerRecordRewards(st)
	}
	preparedRoot := st.PrepareCommit()
	var periodIdentityJSON any
	var finalCatalog any
	if observer {
		identities, reader := observerPeriodCatalog(st)
		*allIdentities = mergeCatalogs(*allIdentities, catalogIdentityClosure(identities))
		periodIdentityJSON = jsonValue(identities)
		periodClosure := *allIdentities
		periodClosure.Invocations = identities.Invocations
		finalCatalog = captureNativeCatalog(reader, periodClosure)
	}
	committedRoot := st.Commit()
	if preparedRoot != committedRoot {
		panic(fmt.Sprintf("period %d prepared/committed roots differ", input.number))
	}
	finalReader := state_db.ExtendedReader{Reader: latest.readerAt(types.BlockNum(input.number))}
	period := map[string]any{
		"number": input.number, "transactions": rows,
		"planner_facts": map[string]any{
			"dag_blocks":                []map[string]any{{"author": hex.EncodeToString(validator[:]), "difficulty": 1, "transaction_hashes": hashes}},
			"certificate_votes":         []map[string]any{{"validator": hex.EncodeToString(validator[:]), "period": input.number, "weight": 1}},
			"total_eligible_vote_count": eligibleVotes, "validator_eligible_vote_count": validatorVotes,
			"committee_size": 1, "pillar_interval": 1_000, "is_pillar_boundary": input.number%1_000 == 0,
		},
		"reward_input": map[string]any{
			"block_author": hex.EncodeToString(validator[:]), "blocks_per_year": cfg.DPOS.BlocksPerYear,
			"total_dag_blocks_count": 1, "total_votes_weight": 1, "max_votes_weight": 1,
			"validators": []map[string]any{{"validator": hex.EncodeToString(validator[:]), "dag_blocks_count": 1, "vote_weight": 1, "fees_reward": fees.String()}},
		},
		"reward_output":             map[string]string{"actual_transaction_fees": fees.String(), "minted_reward": minted.ToBig().String()},
		"period_catalog_identities": periodIdentityJSON,
		"final": map[string]any{
			"root": hex.EncodeToString(preparedRoot[:]), "committed_root": hex.EncodeToString(committedRoot[:]),
			"rows": latest.rows.exportPhysical(), "latest_rows": latest.rows.exportLatest(),
			"accounts":                      observeAddressSet(finalReader, observedAddresses),
			"native_storage_by_hashed_path": nativeStorageByPath(finalReader), "native_catalog": finalCatalog,
		},
	}
	if rawWriteTraceAvailable() {
		period["ordered_raw_writes"] = map[string]any{
			"begin_block": beginBlockWrites,
			"rewards":     rewardWrites,
			"end_block":   endBlockWrites,
		}
	}
	return period
}

func runFullWitness(mode string) map[string]any {
	observer := mode == "observer"
	if mode != "batched" && !observer {
		panic("mode must be batched or observer")
	}
	if observer && !observerAvailable() {
		panic("observer mode requested from a pin without the concrete observer API")
	}
	sender := addressForPrivateKey(testPrivateKey)
	secondSigner := addressForPrivateKey(fullDelegatorKey)
	validator := addressForPrivateKey(fullValidatorKey)
	owner := crypto.CreateAddress(&sender, big.NewInt(8))
	if sender != common.HexToAddress("0x1a642f0e3c3af545e7acbd38b07251b3990914f1") ||
		secondSigner != common.HexToAddress("0x5050a4f4b3f9338c3472dcc01a87c76a144b3c9c") ||
		validator != common.HexToAddress("0x58da990a8f4a3a6ca7cb6315d68a140105917352") ||
		owner != common.HexToAddress("0xd0e2105ab025bd19229668044fef3f80d9660675") {
		panic("full workload fixed key/address derivation changed")
	}
	cfg := fullWitnessConfig(sender, secondSigner, validator, owner)
	periodInputs, bytecode, addresses := fullPeriodInputs(sender, secondSigner, validator)
	latest := newMemoryLatest()
	beginRawWriteTrace()
	st := newStateTransition(latest, &cfg)
	genesisWrites := finishRawWriteTrace()
	genesisDescriptor := latest.GetCommittedDescriptor()
	genesisReader := state_db.ExtendedReader{Reader: latest.readerAt(0)}
	allIdentities := catalogIdentities{}
	genesis := map[string]any{
		"period": 0, "root": hex.EncodeToString(genesisDescriptor.StateRoot[:]),
		"rows": latest.rows.exportPhysical(), "latest_rows": latest.rows.exportLatest(),
		"accounts":                      observeAddressSet(genesisReader, addresses),
		"native_storage_by_hashed_path": nativeStorageByPath(genesisReader),
	}
	attachRawWrites(genesis, genesisWrites)
	if observer {
		identities, reader := observerGenesisCatalog(st)
		allIdentities = mergeCatalogs(allIdentities, catalogIdentityClosure(identities))
		genesis["native_catalog"] = captureNativeCatalog(reader, allIdentities)
	}
	periods := make([]map[string]any, 0, len(periodInputs))
	for index, periodInput := range periodInputs {
		periods = append(periods, executeFullPeriod(st, latest, cfg, observer, periodInput, validator, &allIdentities, addresses))
		st.Close()
		if index+1 < len(periodInputs) {
			st = newStateTransition(latest, &cfg)
		}
	}
	historicalPeriodOne := state_db.ExtendedReader{Reader: latest.readerAt(1)}
	historicalPeriodTwo := state_db.ExtendedReader{Reader: latest.readerAt(2)}
	historicalPeriodFour := state_db.ExtendedReader{Reader: latest.readerAt(4)}
	contractA := crypto.CreateAddress(&sender, big.NewInt(1))
	contractB := crypto.CreateAddress(&sender, big.NewInt(3))
	childC := crypto.CreateAddress(&sender, big.NewInt(5))
	dispatcherE := crypto.CreateAddress(&sender, big.NewInt(8))
	zeroSlot := common.Hash{}
	oneSlot := common.Hash{31: 1}
	lifecycleAtOne := observeAccountReader(historicalPeriodOne, contractA)
	lifecycleAtTwo := observeAccountReader(historicalPeriodTwo, contractA)
	lifecycleSlotAtOne := observeStorageSlot(historicalPeriodOne, contractA, zeroSlot)
	lifecycleSlotAtTwo := observeStorageSlot(historicalPeriodTwo, contractA, zeroSlot)
	restoredSlotZeroAtOne := observeStorageSlot(historicalPeriodOne, contractB, zeroSlot)
	restoredSlotOneAtOne := observeStorageSlot(historicalPeriodOne, contractB, oneSlot)
	restoredSlotZeroAtFour := observeStorageSlot(historicalPeriodFour, contractB, zeroSlot)
	restoredSlotOneAtFour := observeStorageSlot(historicalPeriodFour, contractB, oneSlot)
	childAtOne := observeAccountReader(historicalPeriodOne, childC)
	beneficiaryAtOne := observeAccountReader(historicalPeriodOne, missingBeneficiary)
	dispatcherAtTwo := observeAccountReader(historicalPeriodTwo, dispatcherE)
	dispatcherAtFour := observeAccountReader(historicalPeriodFour, dispatcherE)
	requireObservedAccount(lifecycleAtOne, true, "0", "period-one lifecycle account")
	requireObservedAccount(lifecycleAtTwo, false, "", "period-two lifecycle account")
	requireObserved(lifecycleSlotAtOne, true, "07", "period-one lifecycle slot zero")
	requireObserved(lifecycleSlotAtTwo, true, "07", "period-two orphaned lifecycle storage row")
	requireObserved(restoredSlotZeroAtOne, true, "07", "period-one restored slot zero")
	requireObserved(restoredSlotOneAtOne, true, "09", "period-one restored slot one")
	requireObserved(restoredSlotZeroAtFour, true, "07", "period-four restored slot zero")
	requireObserved(restoredSlotOneAtFour, true, "09", "period-four restored slot one")
	requireObservedAccount(childAtOne, true, "7", "reverted selfdestruct child")
	requireObservedAccount(beneficiaryAtOne, false, "", "reverted selfdestruct beneficiary")
	requireObservedAccount(dispatcherAtTwo, true, "0", "reverted delegate custody")
	if dispatcherAtFour["present"] != true {
		panic("period-four native dispatcher is absent")
	}
	storagePath := crypto.Keccak256Hash(zeroSlot[:])
	physicalStorageKey := crypto.Keccak256Hash(contractA[:], storagePath[:])
	retainedSlot, retained := latest.rows.physicalValue(state_db.COL_acc_trie_value, physicalStorageKey, 1)
	if !retained || !bytes.Equal(retainedSlot, []byte{7}) {
		panic(fmt.Sprintf("deleted lifecycle period-one CF5 row differs: present=%v value=%x", retained, retainedSlot))
	}
	return map[string]any{
		"schema":         2,
		"scope":          "finite signed four-period StateTransition mixed workload with incremental TrieSink roots and memory close/reopen; no application header, RocksDB, production route, or reconstructed-root claim",
		"execution_mode": mode, "configuration": fullConfigurationJSON(cfg),
		"inputs": map[string]any{
			"sender_private_key": hex.EncodeToString(testPrivateKey), "sender": hex.EncodeToString(sender[:]),
			"new_delegator_private_key": hex.EncodeToString(fullDelegatorKey), "new_delegator": hex.EncodeToString(secondSigner[:]),
			"pbft_private_key": hex.EncodeToString(fullValidatorKey), "validator": hex.EncodeToString(validator[:]),
			"validator_owner": hex.EncodeToString(owner[:]), "initial_delegator": hex.EncodeToString(initialDelegator[:]),
			"genesis_allocations": []map[string]string{
				{"address": hex.EncodeToString(sender[:]), "balance": "100000000"},
				{"address": hex.EncodeToString(secondSigner[:]), "balance": "2000000"},
				{"address": hex.EncodeToString(initialDelegator[:]), "balance": "2000"},
			},
			"initial_validator": map[string]any{
				"address": hex.EncodeToString(validator[:]), "owner": hex.EncodeToString(owner[:]),
				"vrf_key": hex.EncodeToString(bytes.Repeat([]byte{0x44}, 32)), "commission": 0,
				"delegations": []map[string]string{{"delegator": hex.EncodeToString(initialDelegator[:]), "amount": "1000"}},
			},
			"bytecode": bytecode,
		},
		"genesis": genesis, "periods": periods,
		"historical_reads": map[string]any{
			"period_1_lifecycle_account":              lifecycleAtOne,
			"period_1_lifecycle_slot_0":               lifecycleSlotAtOne,
			"period_2_lifecycle_account":              lifecycleAtTwo,
			"period_2_orphaned_lifecycle_storage_row": lifecycleSlotAtTwo,
			"retained_period_1_lifecycle_cf5": map[string]any{
				"logical_key": hex.EncodeToString(physicalStorageKey[:]), "period": 1,
				"physical_key": hex.EncodeToString(append(append([]byte(nil), physicalStorageKey[:]...), 0, 0, 0, 0, 0, 0, 0, 1)),
				"value":        hex.EncodeToString(retainedSlot),
			},
			"restored_slot_contract": map[string]any{
				"period_1_slot_0": restoredSlotZeroAtOne, "period_1_slot_1": restoredSlotOneAtOne,
				"period_4_slot_0": restoredSlotZeroAtFour, "period_4_slot_1": restoredSlotOneAtFour,
			},
			"reverted_selfdestruct": map[string]any{"child": childAtOne, "beneficiary": beneficiaryAtOne},
			"native_custody":        map[string]any{"dispatcher_period_2": dispatcherAtTwo, "dispatcher_period_4": dispatcherAtFour},
		},
		"reopen":           map[string]any{"close_reopen_after_periods": []uint64{1, 2, 3}, "continued_after_each_reopen": true},
		"concrete_columns": []string{"CF1/code", "CF2/main_trie_node", "CF3/main_trie_value(versioned)", "CF4/account_trie_node", "CF5/account_trie_value(versioned)"},
		"row_model":        "memory LatestState and PendingBlockState preserve exact incremental TrieSink writes and version history; close/reopen reconstructs StateTransition over the same committed rows",
		"observer_api":     map[string]any{"available": observerAvailable(), "scope": "local observer methods only; public pin exports the existing period-batched lifetime"},
		"raw_write_trace": map[string]any{
			"available": rawWriteTraceAvailable(),
			"scope":     "archive-only synchronous observer at SetStateRawIrreversibly entry; preserves setter operation order, repeats, and tombstones; does not claim physical persistence order or universal raw-state irreversibility",
		},
		"gas_refund_trace": map[string]any{
			"available": rawWriteTraceAvailable(),
			"scope":     "archive-only synchronous observation of the single refund counter and capped refund applied by EVM.Main; the patched expression preserves one GetRefund and one MinU64 evaluation",
		},
	}
}
