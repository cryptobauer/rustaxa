//go:build !mixed_raw_trace

package main

func rawWriteTraceAvailable() bool { return false }

func beginRawWriteTrace() {}

func finishRawWriteTrace() []rawWrite { return nil }

func beginGasRefundTrace() {}

func finishGasRefundTrace() *gasRefund { return nil }
