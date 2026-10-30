// Added to a verified temporary copy of native_integration_reference.go.
func integrationBoundaryInputs() ([]fullPeriodInput, map[string]any, []common.Address) {
	ps := make([]fullPeriodInput, 12)
	for i := range ps { ps[i].number = uint64(i + 1) }
	add := func(period, who int, name string, value uint64, input []byte, expected string) {
		spec := fullSpec(name, 0, addressForPrivateKey(integrationKeys[who]), integrationKeys[who], dpos.ContractAddress(), value, input)
		spec.ExpectedError = expected
		ps[period-1].transactions = append(ps[period-1].transactions, spec)
	}
	for i, v := range integrationValidators {
		add(1, i, fmt.Sprintf("register_%d", i), 1000, integrationPack(false, "registerValidator", v, integrationProof(i), bytes.Repeat([]byte{byte(0x44 + i)}, 32), uint16(100), "validator", "endpoint"), "")
	}
	b := integrationValidators[1]
	for _, who := range []int{0, 3, 4} {
		add(2, who, fmt.Sprintf("delegate_%d", who), 200, integrationPack(false, "delegate", b), "")
		amount := int64(10)
		if who == 4 { amount = 200 }
		add(3, who, fmt.Sprintf("withdraw_%d", who), 0, integrationPack(false, "undelegate", b, big.NewInt(amount)), "")
	}
	add(2, 2, "delegate_confirm_first", 200, integrationPack(false, "delegate", b), "")
	add(3, 2, "withdraw_confirm_first", 0, integrationPack(false, "undelegate", b, big.NewInt(10)), "")
	missing := "Undelegation does not exist"
	add(4, 1, "wrong_caller", 0, integrationPack(false, "cancelUndelegate", b), missing)
	add(6, 2, "confirm_before_cancel", 0, integrationPack(false, "confirmUndelegate", b), "")
	add(6, 2, "cancel_after_confirm", 0, integrationPack(false, "cancelUndelegate", b), missing)
	for i, who := range []int{0, 3, 4} {
		p := 5 + i
		add(p, who, fmt.Sprintf("cancel_%d", who), 0, integrationPack(false, "cancelUndelegate", b), "")
		add(p, who, fmt.Sprintf("repeat_%d", who), 0, integrationPack(false, "cancelUndelegate", b), missing)
		add(p, who, fmt.Sprintf("confirm_after_cancel_%d", who), 0, integrationPack(false, "confirmUndelegate", b), missing)
	}
	for p := 8; p <= 12; p++ {
		add(p, 2, fmt.Sprintf("query_%d", p), 0, integrationPack(false, "getValidator", b), "")
	}
	addresses := []common.Address{integrationValidators[0], b, integrationValidators[2], *dpos.ContractAddress(), *slashing.ContractAddress()}
	allocations := []map[string]string{}
	for _, key := range integrationKeys {
		v := addressForPrivateKey(key)
		addresses = append(addresses, v)
		allocations = append(allocations, map[string]string{"address": hex.EncodeToString(v[:]), "balance": "1000000000"})
	}
	owner := addressForPrivateKey(integrationKeys[0])
	inputs := map[string]any{"genesis_allocations": allocations, "initial_validators": []any{}, "initial_validator": map[string]any{"address": hex.EncodeToString(integrationValidators[0][:]), "owner": hex.EncodeToString(owner[:]), "vrf_key": strings.Repeat("44", 32), "commission": 100, "delegations": []any{}}}
	nonces := map[common.Address]uint64{}
	for i := range ps { for j := range ps[i].transactions {
		spec := &ps[i].transactions[j]
		sender := addressForPrivateKey(spec.PrivateKey)
		spec.Nonce = nonces[sender]
		nonces[sender]++
	} }
	return ps, inputs, addresses
}

func integrationBoundaryPending(latest *memoryLatest) []map[string]any {
	reader := state_db.ExtendedReader{Reader: latest.readerAt(types.BlockNum(3))}
	rows := []map[string]any{}
	validator := integrationValidators[1]
	for _, who := range []int{0, 3, 4} {
		caller := addressForPrivateKey(integrationKeys[who])
		key := contract_storage.Stor_k_1([]byte{3, 0}, validator[:], caller[:])
		found := false
		reader.GetAccountStorage(dpos.ContractAddress(), key, func(raw []byte) {
			var entry dpos.UndelegationV1
			rlp.MustDecodeBytes(raw, &entry)
			rows = append(rows, map[string]any{"caller": hex.EncodeToString(caller[:]), "validator": hex.EncodeToString(validator[:]), "key": hex.EncodeToString(key[:]), "raw": hex.EncodeToString(raw), "amount": entry.Amount.String(), "block": uint64(entry.Block)})
			found = true
		})
		if !found { panic("missing committed boundary queue") }
	}
	return rows
}
