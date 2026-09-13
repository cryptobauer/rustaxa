// Pinned V1/V2 undelegation-cancellation oracle. The harness composes this
// exporter with the native simulation and V1 custody support sources, plus the
// guarded raw-storage observer installed in a disposable archive checkout.
package main

import (
	"encoding/hex"
	"encoding/json"
	"math/big"
	"os"

	contract_storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
)

type cancelScenario struct {
	Name         string               `json:"name"`
	Version      string               `json:"version"`
	Amount       string               `json:"amount"`
	Magnolia     bool                 `json:"magnolia"`
	Before       custodyState         `json:"before"`
	AfterBlock   custodyState         `json:"after_block"`
	Transactions []custodyTransaction `json:"transactions"`
}

func runV1CancelScenario(name string, amount int64, magnolia, createQueue bool) cancelScenario {
	cfg := custodyConfig(magnolia, true, false)
	database := newNativeSimulationDB()
	transition := nativeSimulationTransition(database, &cfg)
	defer transition.Close()
	advanceCustodyBlock(transition, 1)
	before := custodyCommittedState(database, cfg, 0)
	transactions := make([]custodyTransaction, 0, 2)
	nonce := uint64(0)
	if createQueue {
		transactions = append(transactions, runCustodyTransaction(
			transition, nonce, "undelegate_v1", "undelegate(address,uint256)", big.NewInt(amount),
		))
		nonce++
	}
	transactions = append(transactions, runCustodyTransaction(
		transition, nonce, "cancel_v1", "cancelUndelegate(address)", nil,
	))
	finishCustodyBlock(transition)
	return cancelScenario{
		Name: name, Version: "v1", Amount: big.NewInt(amount).String(), Magnolia: magnolia,
		Before: before, AfterBlock: custodyCommittedState(database, cfg, 1), Transactions: transactions,
	}
}

func runV2CancelScenario() cancelScenario {
	cfg := custodyConfig(true, true, false)
	database := newNativeSimulationDB()
	transition := nativeSimulationTransition(database, &cfg)
	defer transition.Close()
	advanceCustodyBlock(transition, 1)
	before := custodyCommittedState(database, cfg, 0)
	transactions := []custodyTransaction{
		runCustodyTransaction(transition, 0, "undelegate_v2_id_1", "undelegateV2(address,uint256)", big.NewInt(200)),
		runCustodyTransaction(transition, 1, "undelegate_v2_id_2", "undelegateV2(address,uint256)", big.NewInt(300)),
		runCustodyTransaction(transition, 2, "cancel_v2_id_1", "cancelUndelegateV2(address,uint64)", big.NewInt(1)),
		runCustodyTransaction(transition, 3, "cancel_v2_id_2", "cancelUndelegateV2(address,uint64)", big.NewInt(2)),
		runCustodyTransaction(transition, 4, "cancel_v2_missing", "cancelUndelegateV2(address,uint64)", big.NewInt(99)),
	}
	finishCustodyBlock(transition)
	return cancelScenario{
		Name: "v2_non_last_then_last", Version: "v2", Amount: "500", Magnolia: true,
		Before: before, AfterBlock: custodyCommittedState(database, cfg, 1), Transactions: transactions,
	}
}

func main() {
	v1ObjectKey := contract_storage.Stor_k_1([]byte{3, 0}, custodyValidator[:], custodyDelegator[:])
	document := map[string]any{
		"schema": 1,
		"selectors": map[string]string{
			"cancel_v1": hex.EncodeToString(custodyInput("cancelUndelegate(address)", nil)[:4]),
			"cancel_v2": hex.EncodeToString(custodyInput("cancelUndelegateV2(address,uint64)", big.NewInt(1))[:4]),
		},
		"action_gas":    map[string]uint64{"cancel_v1": 60_000, "cancel_v2": 60_000},
		"v1_object_key": hex.EncodeToString(v1ObjectKey[:]),
		"scenarios": []cancelScenario{
			runV1CancelScenario("v1_partial_existing_delegation", 300, true, true),
			runV1CancelScenario("v1_full_recreated_delegation", 1_000, true, true),
			runV1CancelScenario("v1_missing_queue", 300, true, false),
			runV1CancelScenario("pre_magnolia_v1_missing_validator", 1_000, false, true),
			runV2CancelScenario(),
		},
	}
	if err := json.NewEncoder(os.Stdout).Encode(document); err != nil {
		panic(err)
	}
}
