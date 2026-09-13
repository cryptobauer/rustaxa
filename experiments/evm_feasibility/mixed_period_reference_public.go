//go:build !concrete_observer

package main

import (
	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_transition"
)

func observerAvailable() bool { return false }

func observerGenesisCatalog(*state_transition.StateTransition) (catalogIdentities, state_db.ExtendedReader) {
	panic("concrete observer API is unavailable in the public pin")
}

func observerFinalizeTransaction(*state_transition.StateTransition, uint64) (common.Hash, catalogIdentities, state_db.ExtendedReader) {
	panic("concrete observer API is unavailable in the public pin")
}

func observerRecordRewards(*state_transition.StateTransition) {
	panic("concrete observer API is unavailable in the public pin")
}

func observerPeriodCatalog(*state_transition.StateTransition) (catalogIdentities, state_db.ExtendedReader) {
	panic("concrete observer API is unavailable in the public pin")
}
