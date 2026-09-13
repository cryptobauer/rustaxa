//go:build mixed_raw_trace

package main

import (
	"encoding/hex"
	"sync"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/vm"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
)

var mixedRawWriteCollector struct {
	sync.Mutex
	writes []rawWrite
}

var mixedGasRefundCollector struct {
	sync.Mutex
	refund *gasRefund
	active bool
}

func rawWriteTraceAvailable() bool { return true }

func beginRawWriteTrace() {
	mixedRawWriteCollector.Lock()
	if mixedRawWriteCollector.writes != nil {
		mixedRawWriteCollector.Unlock()
		panic("raw-write trace already active")
	}
	mixedRawWriteCollector.writes = make([]rawWrite, 0)
	mixedRawWriteCollector.Unlock()
	state_evm.SetMixedPeriodRawWriteObserver(func(address common.Address, key common.Hash, value []byte) {
		mixedRawWriteCollector.Lock()
		mixedRawWriteCollector.writes = append(mixedRawWriteCollector.writes, rawWrite{
			Address: hex.EncodeToString(address[:]),
			Key:     hex.EncodeToString(key[:]),
			Value:   hex.EncodeToString(value),
		})
		mixedRawWriteCollector.Unlock()
	})
}

func finishRawWriteTrace() []rawWrite {
	state_evm.SetMixedPeriodRawWriteObserver(nil)
	mixedRawWriteCollector.Lock()
	defer mixedRawWriteCollector.Unlock()
	if mixedRawWriteCollector.writes == nil {
		panic("raw-write trace is not active")
	}
	ret := make([]rawWrite, len(mixedRawWriteCollector.writes))
	copy(ret, mixedRawWriteCollector.writes)
	mixedRawWriteCollector.writes = nil
	return ret
}

func beginGasRefundTrace() {
	mixedGasRefundCollector.Lock()
	if mixedGasRefundCollector.active {
		mixedGasRefundCollector.Unlock()
		panic("gas-refund trace already active")
	}
	mixedGasRefundCollector.active = true
	mixedGasRefundCollector.Unlock()
	vm.SetMixedPeriodGasRefundObserver(func(counter uint64, applied uint64) {
		mixedGasRefundCollector.Lock()
		defer mixedGasRefundCollector.Unlock()
		if mixedGasRefundCollector.refund != nil {
			panic("transaction produced multiple gas-refund observations")
		}
		mixedGasRefundCollector.refund = &gasRefund{Counter: counter, Applied: applied}
	})
}

func finishGasRefundTrace() *gasRefund {
	vm.SetMixedPeriodGasRefundObserver(nil)
	mixedGasRefundCollector.Lock()
	defer mixedGasRefundCollector.Unlock()
	if !mixedGasRefundCollector.active || mixedGasRefundCollector.refund == nil {
		panic("transaction produced no gas-refund observation")
	}
	ret := *mixedGasRefundCollector.refund
	mixedGasRefundCollector.refund = nil
	mixedGasRefundCollector.active = false
	return &ret
}
