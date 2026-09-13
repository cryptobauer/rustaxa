//go:build concrete_observer

package main

import (
	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_transition"
)

func observerAvailable() bool { return true }

func observerGenesisCatalog(st *state_transition.StateTransition) (catalogIdentities, state_db.ExtendedReader) {
	accounts, slots, invocations := st.ConcreteProjectionCatalog()
	return catalogIdentities{Accounts: accounts, Slots: catalogSlots(slots), Invocations: jsonValue(invocations)}, st.ConcreteProjectionReader()
}

func observerFinalizeTransaction(st *state_transition.StateTransition, index uint64) (common.Hash, catalogIdentities, state_db.ExtendedReader) {
	st.FinalizeConcreteTransaction()
	root := st.PrepareIntermediateRoot()
	accounts, slots, invocations := st.ConcreteTransactionCatalog(index)
	return root, catalogIdentities{Accounts: accounts, Slots: catalogSlots(slots), Invocations: jsonValue(invocations)}, st.ConcreteProjectionReader()
}

func observerRecordRewards(st *state_transition.StateTransition) {
	st.RecordConcreteRewardsProjection()
}

func observerPeriodCatalog(st *state_transition.StateTransition) (catalogIdentities, state_db.ExtendedReader) {
	accounts, slots, invocations := st.ConcreteProjectionCatalog()
	return catalogIdentities{Accounts: accounts, Slots: catalogSlots(slots), Invocations: jsonValue(invocations)}, st.ConcreteProjectionReader()
}

func catalogSlots(slots []state_evm.ConcreteStorageSlot) []catalogSlot {
	ret := make([]catalogSlot, len(slots))
	for index, slot := range slots {
		ret[index] = catalogSlot{Address: slot.Address, Key: slot.Key}
	}
	return ret
}
