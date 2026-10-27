// Bounded donor-to-synthetic initialization. Execute only through the pinned harness.
package main

import (
	"bytes"
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"sort"

	"github.com/Taraxa-project/taraxa-evm/common"
	"github.com/Taraxa-project/taraxa-evm/core/types"
	"github.com/Taraxa-project/taraxa-evm/crypto"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/chain_config"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
	"github.com/Taraxa-project/taraxa-evm/taraxa/state/state_transition"
)

// eraSeedSource is an actual finalized donor result owned by the serial harness.
// Document retains its execution/config/source evidence; Latest must be that
// same finalized owner, never a state reconstructed from expected target data.
// Rust obtains its native donor snapshot from its own matching finalized owner.
type eraSeedSource struct {
	Latest *memoryLatest
	IDs catalogIdentities
	Addresses []common.Address
	Writes []rawWrite
	Document map[string]any
}

func eraSeedRequire(ok bool, message string) {
	if !ok { panic("donor seed: " + message) }
}

func eraSeedHex(text string, size int) []byte {
	value, err := hex.DecodeString(text)
	eraSeedRequire(err == nil && hex.EncodeToString(value) == text, "noncanonical hex")
	eraSeedRequire(size < 0 || len(value) == size, "hex identity width")
	return value
}

func eraSeedDigest(value any) string {
	encoded, err := json.Marshal(value)
	must(err)
	return fmt.Sprintf("%x", sha256.Sum256(encoded))
}

// eraSourceWrites preserves every source occurrence and phase boundary order.
// It accepts the actual in-process Go exporter types, not lossy JSON-number
// conversions. Distribution writes must equal the redundant flattened phase.
func eraSourceWrites(document map[string]any) []rawWrite {
	seed, ok := document["seed"].(map[string]any)
	eraSeedRequire(ok, "source seed document type")
	initial, ok := seed["ordered_raw_writes"].([]rawWrite)
	eraSeedRequire(ok, "source seed transcript type")
	previous, ok := seed["period"].(uint64)
	eraSeedRequire(ok, "source seed coordinate type")
	periods, ok := document["periods"].([]map[string]any)
	eraSeedRequire(ok && len(periods) > 0, "source execution periods required")
	result := append([]rawWrite{}, initial...)
	for _, period := range periods {
		p, valid := period["number"].(uint64)
		eraSeedRequire(valid && previous != ^uint64(0) && p == previous+1, "source period chronology")
		previous = p
		phases, valid := period["ordered_raw_writes"].(map[string]any)
		eraSeedRequire(valid && len(phases) == 3, "source phase shape")
		begin, valid := phases["begin_block"].([]rawWrite)
		eraSeedRequire(valid, "source BeginBlock transcript type")
		result = append(result, begin...)
		txs, valid := period["transactions"].([]map[string]any)
		eraSeedRequire(valid, "source transaction vector type")
		for _, tx := range txs {
			writes, valid := tx["ordered_raw_writes"].([]rawWrite)
			eraSeedRequire(valid, "source transaction transcript type")
			result = append(result, writes...)
		}
		parts, valid := period["distributions"].([]map[string]any)
		eraSeedRequire(valid, "source distribution vector type")
		order, valid := period["distribution_order"].([]uint64)
		eraSeedRequire(valid && len(order) == len(parts), "source distribution source vector")
		flattened := []rawWrite{}
		for i, part := range parts {
			sourcePeriod, valid := part["period"].(uint64)
			eraSeedRequire(valid && sourcePeriod == order[i], "source distribution occurrence identity")
			writes, valid := part["ordered_raw_writes"].([]rawWrite)
			eraSeedRequire(valid, "source distribution transcript type")
			flattened = append(flattened, writes...)
		}
		rewards, valid := phases["rewards"].([]rawWrite)
		eraSeedRequire(valid && len(rewards) == len(flattened), "source flattened reward count")
		for i := range rewards { eraSeedRequire(rewards[i] == flattened[i], "source flattened reward occurrence") }
		result = append(result, flattened...)
		end, valid := phases["end_block"].([]rawWrite)
		eraSeedRequire(valid, "source EndBlock transcript type")
		result = append(result, end...)
	}
	return result
}

// eraRebasedSeed reconstructs a NEW synthetic retained history from complete
// actual donor logical facts. It does not execute the donor-target time gap,
// migrate hardfork formats, rewrite absolute dates, or clone runtime timers.
// Every root is computed by TrieSink and only then compared with the donor.
func eraRebasedSeed(c chain_config.ChainConfig, at uint64, source *memoryLatest,
	sourceIDs catalogIdentities, sourceAddresses []common.Address, sourceWrites []rawWrite,
) (*memoryLatest, catalogIdentities, map[string]any, []common.Address) {
	eraSeedRequire(source != nil && source.rows != nil && source.pending == nil, "actual finalized donor required")
	eraSeedRequire(at >= 6, "target retained interval underflow")
	first := at-5
	donor := source.descriptor
	eraSeedRequire(donor.BlockNum != types.BlockNumberNIL && uint64(donor.BlockNum) < first, "donor must precede target first retained coordinate")
	physical := source.rows.exportPhysical()
	latestRows := source.rows.exportLatest()
	physicalDigest, latestDigest := eraSeedDigest(physical), eraSeedDigest(latestRows)
	reader := state_db.ExtendedReader{Reader: source.readerAt(donor.BlockNum)}
	// Copy containers; preserve callbacks, duplicates and original observer/CREATE
	// order. Initial observer sorting belongs before donor execution, not here.
	ids := catalogIdentities{Accounts: append([]common.Address{}, sourceIDs.Accounts...),
		Slots: append([]catalogSlot{}, sourceIDs.Slots...), Invocations: sourceIDs.Invocations}
	addresses := append([]common.Address{}, sourceAddresses...)
	writes := append([]rawWrite{}, sourceWrites...)
	accountIDs := map[common.Address]bool{}
	accountPaths := map[string]common.Address{}
	for _, address := range ids.Accounts {
		eraSeedRequire(!accountIDs[address], "duplicate source account catalog identity")
		accountIDs[address] = true
		path := crypto.Keccak256Hash(address[:])
		accountPaths[hex.EncodeToString(path[:])] = address
	}
	for _, address := range addresses { eraSeedRequire(accountIDs[address], "observer address outside complete source catalog") }
	slotIDs := map[catalogSlot]bool{}
	physicalSlots := map[string]catalogSlot{}
	for _, slot := range ids.Slots {
		eraSeedRequire(accountIDs[slot.Address] && !slotIDs[slot], "duplicate slot or missing slot account identity")
		slotIDs[slot] = true
		path := crypto.Keccak256Hash(slot.Key[:])
		dbKey := crypto.Keccak256Hash(slot.Address[:], path[:])
		physicalSlots[hex.EncodeToString(dbKey[:])] = slot
	}
	// All versioned physical identities, including zero/tombstone rows, must
	// have declared logical preimages. No unknown extra can disappear behind an
	// otherwise matching reconstructed root. Physical histories are retained
	// by the donor document; the target gets its own fresh physical history.
	tombstones := []map[string]any{}
	for _, column := range []state_db.Column{state_db.COL_main_trie_value, state_db.COL_acc_trie_value} {
		selected := map[string]string{}
		versions := map[string]uint64{}
		for encoded, value := range physical[column] {
			raw := eraSeedHex(encoded, 40)
			period := binary.BigEndian.Uint64(raw[32:])
			eraSeedRequire(period <= uint64(donor.BlockNum), "donor contains uncommitted/future physical value")
			identity := hex.EncodeToString(raw[:32])
			if column == state_db.COL_main_trie_value {
				_, present := accountPaths[identity]
				eraSeedRequire(present, "physical account missing from donor catalog")
			} else {
				_, present := physicalSlots[identity]
				eraSeedRequire(present, "physical raw slot missing from donor catalog")
			}
			if prior, present := versions[identity]; !present || period > prior {
				versions[identity], selected[identity] = period, value
			}
		}
		eraSeedRequire(len(selected) == len(latestRows[column]), "donor physical/latest identity coverage")
		for identity, value := range selected {
			current, present := latestRows[column][identity]
			eraSeedRequire(present && current == value, "donor physical/latest value mismatch")
			if value == "" {
				tombstones = append(tombstones, map[string]any{"column":column,"key":identity,"source_period":versions[identity]})
			}
		}
	}
	// This is derived provenance metadata, not a mutation/callback transcript.
	// Sort its keys so repeated captures do not depend on Go map iteration.
	sort.Slice(tombstones, func(i, j int) bool {
		a, b := tombstones[i]["column"].(state_db.Column), tombstones[j]["column"].(state_db.Column)
		if a != b { return a < b }
		return tombstones[i]["key"].(string) < tombstones[j]["key"].(string)
	})
	// Replay native Puts exactly. A reward-key/statistics map cannot supply
	// missing membership or a missing raw callback; the source catalog and the
	// actual committed reader must agree with this complete transcript.
	ledger := map[catalogSlot][]byte{}
	for _, row := range writes {
		address := common.BytesToAddress(eraSeedHex(row.Address, 20))
		k := common.BytesToHash(eraSeedHex(row.Key, 32))
		slot := catalogSlot{Address:address, Key:k}
		eraSeedRequire(slotIDs[slot], "source raw write missing from declared slot catalog")
		ledger[slot] = eraSeedHex(row.Value, -1)
	}
	for slot, value := range ledger {
		var actual []byte
		reader.GetAccountStorage(&slot.Address, &slot.Key, func(v []byte) { actual = common.CopyBytes(v) })
		eraSeedRequire(bytes.Equal(actual, value), "source transcript does not reproduce committed raw value")
	}
	presentAccounts := map[common.Address]state_db.Account{}
	changes := map[common.Address]state_evm.AccountChange{}
	for _, address := range ids.Accounts {
		address := address
		reader.GetAccount(&address, func(account state_db.Account) {
			presentAccounts[address] = account
			change := state_evm.AccountChange{Account:account, RawStorageDirty:state_evm.RawStorage{}}
			change.StorageRootHash = nil // reconstruct independently from declared raw slots
			if account.CodeHash != nil {
				code := reader.GetCode(account.CodeHash)
				stored, exists := physical[state_db.COL_code][hex.EncodeToString(account.CodeHash[:])]
				eraSeedRequire(exists && stored == hex.EncodeToString(code), "donor code bytes/history missing")
				eraSeedRequire(uint64(len(code)) == account.CodeSize && crypto.Keccak256Hash(code) == *account.CodeHash, "donor code identity")
				change.Code, change.CodeDirty = common.CopyBytes(code), true
			} else { eraSeedRequire(account.CodeSize == 0, "donor code size without hash") }
			changes[address] = change
		})
	}
	for _, slot := range ids.Slots {
		slot := slot
		var value []byte
		reader.GetAccountStorage(&slot.Address, &slot.Key, func(v []byte) { value = common.CopyBytes(v) })
		change, exists := changes[slot.Address]
		eraSeedRequire(exists || len(value) == 0, "unsupported nonempty orphan storage under a deleted account")
		if exists {
			change.RawStorageDirty[slot.Key] = common.CopyBytes(value)
			changes[slot.Address] = change
		}
		// All native raw storage is emitted by the source wrapper transcript.
		// Ordinary EVM storage is bound separately by complete physical/trie paths.
		if slot.Address == common.HexToAddress("fe") || slot.Address == common.HexToAddress("ee") {
			eraSeedRequire(bytes.Equal(ledger[slot],value), "native catalog row lacks complete source transcript")
		}
	}
	for address := range presentAccounts {
		address := address
		covered := map[common.Hash][]byte{}
		for _, slot := range ids.Slots {
			if slot.Address != address { continue }
			var value []byte
			reader.GetAccountStorage(&slot.Address, &slot.Key, func(v []byte) { value = common.CopyBytes(v) })
			if len(value) != 0 { path := crypto.Keccak256Hash(slot.Key[:]); covered[path] = value }
		}
		live := map[common.Hash][]byte{}
		reader.ForEachStorage(&address, func(path *common.Hash, value []byte) { live[*path] = common.CopyBytes(value) })
		eraSeedRequire(len(live) == len(covered), "donor account trie/catalog storage coverage")
		for path, value := range live { eraSeedRequire(bytes.Equal(covered[path],value), "donor account trie/catalog storage mismatch") }
	}
	donorCatalog := captureNativeCatalog(reader, ids)
	latest := newMemoryLatest()
	latest.descriptor.BlockNum = types.BlockNum(first-1)
	pending := latest.BeginPendingBlock()
	sink := new(state_transition.TrieSink).Init(nil, state_transition.TrieSinkOpts{})
	sink.SetIO(pending)
	for _, a := range ids.Accounts {
		address := a
		if change, exists := changes[address]; exists {
			mutation := sink.StartMutation(&address)
			mutation.Update(change)
			mutation.Commit()
		}
	}
	root := sink.Commit()
	sink.Close()
	must(latest.Commit(root))
	eraSeedRequire(root == donor.StateRoot, "complete donor inventory does not reconstruct actual donor root")
	history := []map[string]any{{"period":first,"root":hex.EncodeToString(root[:])}}
	for p := first+1; p <= at; p++ {
		pending := latest.BeginPendingBlock()
		writer := new(state_transition.TrieSink).Init(&root, state_transition.TrieSinkOpts{})
		writer.SetIO(pending)
		root = writer.Commit()
		writer.Close()
		must(latest.Commit(root))
		eraSeedRequire(root == donor.StateRoot, "empty synthetic initialization changed donor logical state")
		history = append(history, map[string]any{"period":p,"root":hex.EncodeToString(root[:])})
	}
	check := state_db.ExtendedReader{Reader:latest.readerAt(types.BlockNum(at))}
	targetCatalog := captureNativeCatalog(check, ids)
	eraSeedRequire(eraSeedDigest(targetCatalog) == eraSeedDigest(donorCatalog), "donor/target account, code, raw-slot or presence mismatch")
	eraSeedRequire(source.pending == nil && source.descriptor == donor && eraSeedDigest(source.rows.exportPhysical()) == physicalDigest && eraSeedDigest(source.rows.exportLatest()) == latestDigest,
		"donor changed during private target reconstruction")
	result := map[string]any{
		"period":at,"root":hex.EncodeToString(root[:]),"history":history,
		"native_catalog":targetCatalog,"declared_donor_catalog":donorCatalog,
		"donor_identity":map[string]any{"period":uint64(donor.BlockNum),"root":hex.EncodeToString(donor.StateRoot[:]),
			"physical_sha256":physicalDigest,"latest_sha256":latestDigest,"catalog_sha256":eraSeedDigest(donorCatalog),"ordered_raw_writes_sha256":eraSeedDigest(writes)},
		"donor_tombstone_provenance":tombstones,
		"donor_inventory_coverage":map[string]any{"all_physical_account_preimages_covered":true,
			"all_physical_storage_preimages_covered":true,"all_live_account_storage_trie_paths_covered":true,
			"code_source":"actual donor code bytes with matching hash, size and retained physical row",
			"physical_difference":"source tombstones and unreachable physical extras remain authenticated donor provenance; target contains only newly reconstructed logical state and its own physical history"},
		"configuration":configurationJSON(c,1000),
		"rows":latest.rows.exportPhysical(),"latest_rows":latest.rows.exportLatest(),"ordered_raw_writes":writes,
		"provenance":"new synthetic retained state reconstructed from actual bounded donor logical facts; target roots independently computed; no elapsed rewards, hardfork migrations, cleanup or system actions replayed during the donor-target gap; absolute source dates and raw transcript preserved; fresh target physical history is not donor physical compatibility",
	}
	return latest, ids, result, addresses
}
