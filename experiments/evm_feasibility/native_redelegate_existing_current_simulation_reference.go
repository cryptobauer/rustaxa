// Actual signed-prefix H1 existing-destination BOTH-current seed and same-height DryRunner result.
// Native observation is copy-only cache plus committed genesis0 before commit;
// after commit, the complete H1 reader owns every raw fact. No pending trie read.
package main

import (
	"bytes"
	"encoding/hex"
	"encoding/json"
	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/crypto"
	"github.com/Taraxa-project/taraxa-evm/crypto/secp256k1"
	"github.com/Taraxa-project/taraxa-evm/rlp"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/chain_config"
	dpos "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/precompiled"
	storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_dry_runner"
	"math/big"
	"os"
	"reflect"
)

// No observation fills the live native cache. Present empty copies are tombstones.
type signedH1Frozen struct {
	storage.StorageReader
	cache map[common.Hash][]byte
}

func (r signedH1Frozen) GetAccountStorage(a *common.Address, k *common.Hash, cb func([]byte)) {
	if *a != *dpos.ContractAddress() {
		panic("wrong native address")
	}
	if value, found := r.cache[*k]; found {
		if len(value) > 0 {
			cb(common.CopyBytes(value))
		}
		return
	}
	r.StorageReader.GetAccountStorage(a, k, cb)
}

type signedH1Unsigned struct {
	Nonce, GasPrice       *big.Int
	Gas                   uint64
	To                    *common.Address
	Value                 *big.Int
	Input                 []byte
	ChainID, ZeroR, ZeroS *big.Int
}
type signedH1Envelope struct {
	Nonce, GasPrice *big.Int
	Gas             uint64
	To              *common.Address
	Value           *big.Int
	Input           []byte
	V, R, S         *big.Int
}

// Sign, canonically decode, independently recover, then derive the VM request.
func signedH1Prefix() (vm.Transaction, map[string]any) {
	address := *dpos.ContractAddress()
	input := redelegateSimulationABI(nativeSimulationValidator, common.BytesToAddress([]byte{0x32}), 300)
	unsigned := signedH1Unsigned{new(big.Int), new(big.Int), 200000, &address, new(big.Int), input, big.NewInt(666), new(big.Int), new(big.Int)}
	digest := crypto.Keccak256Hash(rlp.MustEncodeToBytes(&unsigned))
	signature, err := secp256k1.Sign(digest[:], bytes.Repeat([]byte{0x31}, 32))
	if err != nil {
		panic(err)
	}
	signed := signedH1Envelope{unsigned.Nonce, unsigned.GasPrice, unsigned.Gas, unsigned.To, unsigned.Value, unsigned.Input, big.NewInt(666*2 + 35 + int64(signature[64])), new(big.Int).SetBytes(signature[:32]), new(big.Int).SetBytes(signature[32:64])}
	encoded := rlp.MustEncodeToBytes(&signed)
	var decoded signedH1Envelope
	rlp.MustDecodeBytes(encoded, &decoded)
	if !bytes.Equal(encoded, rlp.MustEncodeToBytes(&decoded)) || !reflect.DeepEqual(signed, decoded) {
		panic("signed envelope differs")
	}
	recoveredSig := make([]byte, 65)
	decoded.R.FillBytes(recoveredSig[:32])
	decoded.S.FillBytes(recoveredSig[32:64])
	recoveredSig[64] = byte(new(big.Int).Sub(decoded.V, big.NewInt(666*2+35)).Uint64())
	if recoveredSig[64] > 1 {
		panic("bad recovery")
	}
	pub, err := secp256k1.RecoverPubkey(digest[:], recoveredSig)
	if err != nil {
		panic(err)
	}
	recovered := common.BytesToAddress(crypto.Keccak256(pub[1:])[12:])
	if recovered != nativeSimulationSender {
		panic("wrong recovered caller")
	}
	tx := vm.Transaction{From: recovered, To: decoded.To, Nonce: decoded.Nonce, GasPrice: decoded.GasPrice, Gas: decoded.Gas, Value: decoded.Value, Input: common.CopyBytes(decoded.Input)}
	hash := crypto.Keccak256Hash(encoded)
	return tx, map[string]any{"rlp": hex.EncodeToString(encoded), "hash": hex.EncodeToString(hash[:]), "sender": hex.EncodeToString(recovered[:]), "to": hex.EncodeToString(address[:]), "nonce": decoded.Nonce.String(), "gas_price": decoded.GasPrice.String(), "gas": decoded.Gas, "value": decoded.Value.String(), "input": hex.EncodeToString(decoded.Input), "chain_id": 666, "signature_valid": true}
}

// Export all old/current nodes, pools, metadata and both pairs plus membership
// proof rows. Missing values stay explicit; facts are decoded independently.
func signedH1Raw(reader storage.StorageReader) map[string]any {
	keys := map[common.Hash]bool{}
	add := func(parts ...[]byte) { keys[*storage.Stor_k_1(parts...)] = true }
	for _, prefix := range [][]byte{{0, 5}, append([]byte{2, 1}, nativeSimulationSender[:]...), append([]byte{2, 1}, nativeSimulationDelegator[:]...)} {
		add(prefix, []byte{1})
		for _, last := range []byte{0x31, 0x32, 0x33} {
			v := common.BytesToAddress([]byte{last})
			add(prefix, []byte{2}, v[:])
		}
		for i := byte(1); i <= 4; i++ {
			add(prefix, []byte{2}, []byte{i, 0, 0, 0})
		}
	}
	for _, last := range []byte{0x31, 0x32, 0x33} {
		v := common.BytesToAddress([]byte{last})
		add([]byte{0, 0}, v[:])
		add([]byte{0, 2}, v[:])
		add([]byte{1}, v[:], []byte{})
		add([]byte{1}, v[:], []byte{1})
		add([]byte{2, 0}, v[:], nativeSimulationSender[:])
		add([]byte{2, 0}, v[:], nativeSimulationDelegator[:])
	}
	out := map[string]any{}
	a := *dpos.ContractAddress()
	for key := range keys {
		value := []byte{}
		present := false
		reader.GetAccountStorage(&a, &key, func(v []byte) {
			if len(v) > 0 {
				present = true
				value = common.CopyBytes(v)
			}
		})
		out[hex.EncodeToString(key[:])] = map[string]any{"present": present, "value": hex.EncodeToString(value)}
	}
	return out
}
func signedH1Facts(reader storage.StorageReader) map[string]any {
	result := swapAppendSimulationFacts(reader)
	address := *dpos.ContractAddress()
	wrapper := new(storage.StorageWrapper)
	wrapper.StorageReaderWrapper.Init(&address, reader)
	validators := new(dpos.Validators)
	validators.Init(wrapper, []byte{0})
	rows := map[string]any{}
	for _, last := range []byte{0x31, 0x32, 0x33} {
		v := common.BytesToAddress([]byte{last})
		row := validators.GetValidator(&v)
		if row == nil {
			rows[hex.EncodeToString(v[:])] = nil
			continue
		}
		pool := validators.GetValidatorRewards(&v)
		rows[hex.EncodeToString(v[:])] = map[string]any{"stake": row.TotalStake.String(), "head": uint64(row.LastUpdated), "rewards": pool.RewardsPool.String(), "commission_rewards": pool.CommissionRewardsPool.String()}
	}
	result["validators"] = rows
	return result
}
func currentSignedSnapshot(seed nativeSimulationSeed, api *dpos.API) map[string]any {
	snapshot := nativeSimulationSnapshot(seed, api)
	reader := state_db.ExtendedReader{Reader: seed.database.GetBlockStateReader(1)}
	snapshot["configuration"] = seed.config
	snapshot["native_facts"] = signedH1Facts(reader)
	snapshot["native_raw"] = signedH1Raw(reader)
	return snapshot
}
func seedCurrentSignedH1() (nativeSimulationSeed, map[string]any) {
	database := newNativeSimulationDB()
	cfg := nativeSimulationConfig()
	cfg.DPOS.MinimumDeposit = big.NewInt(100)
	cfg.DPOS.DelegationLockingPeriod = 2
	cfg.Hardforks.CornusHf.DelegationLockingPeriod = 3
	cfg.Hardforks.CactiHf.DelegationLockingPeriod = 7
	cfg.GenesisBalances = core.BalanceMap{nativeSimulationSender: big.NewInt(5000), nativeSimulationDelegator: big.NewInt(3000)}
	cfg.Hardforks.AspenHf.MaxSupply = big.NewInt(8000)
	cfg.DPOS.InitialValidators = []chain_config.GenesisValidator{}
	for _, last := range []byte{0x31, 0x32} {
		vrf := byte(0x44)
		if last == 0x32 {
			vrf = 0x55
		} else if last == 0x33 {
			vrf = 0x66
		}
		delegations := core.BalanceMap{nativeSimulationDelegator: big.NewInt(1000)}
		delegations[nativeSimulationSender] = big.NewInt(1000)
		cfg.DPOS.InitialValidators = append(cfg.DPOS.InitialValidators, chain_config.GenesisValidator{Address: common.BytesToAddress([]byte{last}), Owner: nativeSimulationDelegator, VrfKey: bytes.Repeat([]byte{vrf}, 32), Commission: 100, Delegations: delegations})
	}
	transition := nativeSimulationTransition(database, &cfg)
	defer transition.Close()
	transition.BeginBlock(&vm.BlockInfo{Author: nativeSimulationValidator, GasLimit: nativeSimulationGas, Time: 1700000001, Difficulty: new(big.Int)})
	baseline := state_db.ExtendedReader{Reader: database.GetBlockStateReader(0)}
	capture := func() map[string]any {
		view := signedH1Frozen{baseline, transition.CurrentSourceCacheSnapshot()}
		return map[string]any{"raw": signedH1Raw(view), "facts": signedH1Facts(view)}
	}
	before := capture()
	tx, signed := signedH1Prefix()
	result := transition.ExecuteTransaction(&tx)
	if result.ConsensusErr != "" || result.ExecutionErr != "" {
		panic("signed prefix failed")
	}
	prefix := nativeSimulationOutput{EffectiveNonce: tx.Nonce.String(), GasUsed: result.GasUsed, ConsensusError: string(result.ConsensusErr), ExecutionError: string(result.ExecutionErr), Return: hex.EncodeToString(result.CodeRetval), Logs: []nativeSimulationLog{}}
	for _, log := range result.Logs {
		topics := []string{}
		for _, topic := range log.Topics {
			topics = append(topics, hex.EncodeToString(topic[:]))
		}
		prefix.Logs = append(prefix.Logs, nativeSimulationLog{Address: hex.EncodeToString(log.Address[:]), Topics: topics, Data: hex.EncodeToString(log.Data)})
	}
	afterPrefix := capture()
	transition.EndBlock()
	afterEnd := capture()
	transition.Commit()
	committed := state_db.ExtendedReader{Reader: database.GetBlockStateReader(1)}
	afterCommit := map[string]any{"raw": signedH1Raw(committed), "facts": signedH1Facts(committed)}
	if !reflect.DeepEqual(afterPrefix, afterEnd) || !reflect.DeepEqual(afterEnd, afterCommit) {
		panic("EndBlock/Commit changed selected native facts")
	}
	return nativeSimulationSeed{database: database, config: cfg, wrapper: nativeSimulationDelegator}, map[string]any{"signed_transaction": signed, "receipt": prefix, "before": before, "after_prefix": afterPrefix, "after_end_block": afterEnd, "after_commit": afterCommit}
}
func main() {
	x, y := secp256k1.S256().ScalarBaseMult(bytes.Repeat([]byte{0x31}, 32))
	pub := append([]byte{4}, x.FillBytes(make([]byte, 32))...)
	pub = append(pub, y.FillBytes(make([]byte, 32))...)
	nativeSimulationSender = common.BytesToAddress(crypto.Keccak256(pub[1:])[12:])
	nativeSimulationDelegator = common.BytesToAddress([]byte{0xa1})
	seed, prefix := seedCurrentSignedH1()
	api := new(dpos.API).Init(seed.config)
	factory := func(period types.BlockNum) storage.StorageReader {
		return state_db.ExtendedReader{Reader: seed.database.GetBlockStateReader(period)}
	}
	runner := new(state_dry_runner.DryRunner).Init(seed.database, func(types.BlockNum) *big.Int { return new(big.Int) }, api, factory, &seed.config)
	block := &vm.Block{Number: 1, BlockInfo: vm.BlockInfo{Author: nativeSimulationValidator, GasLimit: nativeSimulationGas, Time: 1700000001, Difficulty: new(big.Int)}}
	before := currentSignedSnapshot(seed, api)
	address := *dpos.ContractAddress()
	wide := new(big.Int).Lsh(big.NewInt(1), 512)
	tx := vm.Transaction{From: nativeSimulationSender, To: &address, Nonce: new(big.Int).Set(wide), GasPrice: new(big.Int), Gas: 200000, Value: new(big.Int), Input: redelegateSimulationABI(nativeSimulationValidator, common.BytesToAddress([]byte{0x32}), 700)}
	first := runNativeSimulationCaptured(runner, block, "existing_current_signed_h1", tx)
	tx.Nonce = new(big.Int).Set(wide)
	second := runNativeSimulationCaptured(runner, block, "existing_current_signed_h1", tx)
	if !reflect.DeepEqual(first, second) {
		panic("probe differs on repeat")
	}
	after := currentSignedSnapshot(seed, api)
	if !reflect.DeepEqual(before, after) {
		panic("simulation changed committed H1")
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"schema": 1, "state_before": before, "state_after": after, "prefix": prefix, "caller": hex.EncodeToString(nativeSimulationSender[:]), "key_provenance": "known fixture key 31 repeated32; canonical signed EIP155 chain666 prefix", "cases": []nativeSimulationCase{first}, "repeat_identical": true, "committed_state_unchanged": true, "scope": "new signed-caller existing-destination BOTH-current H1; same-height DryRunner; complete actual seed; no production or synthetic Rust root equality"}); err != nil {
		panic(err)
	}
}
