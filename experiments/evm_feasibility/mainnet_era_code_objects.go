// Lossless, post-execution code-body packaging at reviewed observation paths.
package main

import (
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"math"
	"strconv"

	"github.com/Taraxa-project/taraxa-evm/crypto"
)

const eraCodeObjectsSchema = 1
const eraCodeMaxBytes = 64 * 1024 * 1024
const eraCodeMaxObjects = 4096
const eraCodeMaxAccountOccurrences = 4000000
const eraCodeMaxDonorDepth = 16

func eraCodeRequire(ok bool, path, message string) {
	if !ok { panic("code-object codec " + path + ": " + message) }
}

func eraCodeMap(parent map[string]any, key, path string) map[string]any {
	value, present := parent[key]
	if !present || value == nil { return nil }
	result, ok := value.(map[string]any)
	eraCodeRequire(ok, path+"."+key, "expected object")
	return result
}

func eraCodeRows(parent map[string]any, key, path string) []map[string]any {
	value, present := parent[key]
	if !present || value == nil { return nil }
	switch rows := value.(type) {
	case []map[string]any:
		return rows
	case []any:
		result := make([]map[string]any,len(rows))
		for i, row := range rows {
			var ok bool
			result[i], ok = row.(map[string]any)
			eraCodeRequire(ok, fmt.Sprintf("%s.%s[%d]",path,key,i), "expected account/row object")
		}
		return result
	default:
		eraCodeRequire(false,path+"."+key,"expected row array")
		return nil
	}
}

// This whitelist is intentionally not an arbitrary recursive dictionary walk.
// In particular physical history/rows/latest_rows, logs, calldata and fork
// prefix_periods are not visited. A donor document uses this same whitelist.
func eraCodeAccountOccurrences(document map[string]any) []map[string]any {
	result := []map[string]any{}
	add := func(container map[string]any, path string) {
		if container == nil { return }
		result = append(result,eraCodeRows(container,"accounts",path)...)
		eraCodeRequire(len(result) <= eraCodeMaxAccountOccurrences,path,"account occurrence bound")
	}
	transactions := func(holder map[string]any, path string) {
		for i, tx := range eraCodeRows(holder,"transactions",path) {
			txPath := fmt.Sprintf("%s.transactions[%d]",path,i)
			observer := eraCodeMap(tx,"observer",txPath)
			add(eraCodeMap(observer,"catalog",txPath+".observer"),txPath+".observer.catalog")
		}
	}
	var walk func(map[string]any,string,int)
	walk = func(doc map[string]any,path string,depth int) {
		eraCodeRequire(doc != nil && depth <= eraCodeMaxDonorDepth,path,"document/depth bound")
		_, pool := doc["code_objects"]
		_, version := doc["code_objects_schema"]
		_, count := doc["code_objects_count"]
		_, digest := doc["code_objects_sha256"]
		eraCodeRequire(!pool && !version && !count && !digest,path,"encoder requires raw, unencoded document")
		genesis := eraCodeMap(doc,"genesis",path)
		add(eraCodeMap(genesis,"native_catalog",path+".genesis"),path+".genesis.native_catalog")
		seed := eraCodeMap(doc,"seed",path)
		for _, field := range []string{"native_catalog","declared_projection_catalog","declared_donor_catalog"} {
			add(eraCodeMap(seed,field,path+".seed"),path+".seed."+field)
		}
		if donor := eraCodeMap(seed,"donor_document",path+".seed"); donor != nil { walk(donor,path+".seed.donor_document",depth+1) }
		for i, period := range eraCodeRows(doc,"periods",path) {
			periodPath := fmt.Sprintf("%s.periods[%d]",path,i)
			final := eraCodeMap(period,"final",periodPath)
			add(final,periodPath+".final")
			add(eraCodeMap(final,"native_catalog",periodPath+".final"),periodPath+".final.native_catalog")
			transactions(period,periodPath)
			transactions(eraCodeMap(period,"reward_input",periodPath),periodPath+".reward_input")
			for j, part := range eraCodeRows(period,"distributions",periodPath) {
				partPath := fmt.Sprintf("%s.distributions[%d]",periodPath,j)
				transactions(eraCodeMap(part,"input",partPath),partPath+".input")
			}
		}
		for i, pending := range eraCodeRows(doc,"pending_reward_facts",path) {
			transactions(pending,fmt.Sprintf("%s.pending_reward_facts[%d]",path,i))
		}
		pre := eraCodeMap(doc,"pre_reward",path)
		add(pre,path+".pre_reward")
		add(eraCodeMap(pre,"native_catalog",path+".pre_reward"),path+".pre_reward.native_catalog")
		for i, api := range eraCodeRows(doc,"api_cases",path) {
			for _, name := range []string{"trace","trace_sequence"} {
				tracePath := fmt.Sprintf("%s.api_cases[%d].%s",path,i,name)
				trace := eraCodeMap(api,name,fmt.Sprintf("%s.api_cases[%d]",path,i))
				for j, call := range eraCodeRows(trace,"calls",tracePath) {
					for _, phase := range []string{"before","after"} {
						add(eraCodeMap(call,phase,fmt.Sprintf("%s.calls[%d]",tracePath,j)),fmt.Sprintf("%s.calls[%d].%s",tracePath,j,phase))
					}
				}
			}
		}
	}
	walk(document,"document",0)
	return result
}

func eraCodeSize(value any) (uint64,bool) {
	switch size := value.(type) {
	case uint64: return size,size <= eraCodeMaxBytes
	case uint: return uint64(size),uint64(size) <= eraCodeMaxBytes
	case int: return uint64(size),size >= 0 && size <= eraCodeMaxBytes
	case float64: return uint64(size),!math.IsNaN(size) && !math.IsInf(size,0) && size >= 0 && size <= eraCodeMaxBytes && size == math.Trunc(size)
	case json.Number:
		n,err := strconv.ParseUint(string(size),10,64)
		return n,err == nil && strconv.FormatUint(n,10) == string(size) && n <= eraCodeMaxBytes
	default: return 0,false
	}
}

// eraEncodeCodeObjects must run AFTER all execution, observation and callback
// sequences and BEFORE output selection. Two passes are mandatory: account maps
// alias across actual transaction, reward-input and pending/distribution views.
// The first pass sees only raw maps; the second mutates each occurrence using
// already validated replacement facts, including repeated aliased maps.
// Adds document.code_objects/schema and returns that same invocation-local pool.
func eraEncodeCodeObjects(document map[string]any) map[string]string {
	accounts := eraCodeAccountOccurrences(document)
	pool := map[string]string{}
	type replacement struct { account map[string]any; hash string }
	replacements := []replacement{}
	total := uint64(0)
	for i, account := range accounts {
		path := fmt.Sprintf("account occurrence%d",i)
		eraCodeRequire(account != nil,path,"nil account object")
		_, reference := account["code_object"]
		eraCodeRequire(!reference,path,"preencoded/conflicting account reference")
		value,present := account["code"]
		if !present { continue }
		text,ok := value.(string)
		eraCodeRequire(ok,path,"code must be hex string")
		if text == "" { continue }
		eraCodeRequire(len(text) <= eraCodeMaxBytes*2,path,"object byte bound")
		body,err := hex.DecodeString(text)
		eraCodeRequire(err == nil && hex.EncodeToString(body) == text,path,"canonical nonempty code hex")
		hash,ok := account["code_hash"].(string)
		eraCodeRequire(ok && len(hash) == 64,path,"code hash type/width")
		hashBytes,err := hex.DecodeString(hash)
		eraCodeRequire(err == nil && hex.EncodeToString(hashBytes) == hash && crypto.Keccak256Hash(body).Hex()[2:] == hash,path,"code hash mismatch")
		size,ok := eraCodeSize(account["code_size"])
		eraCodeRequire(ok && size == uint64(len(body)),path,"code size mismatch")
		if prior,exists := pool[hash]; exists { eraCodeRequire(prior == text,path,"conflicting object definition") } else {
			pool[hash] = text
			total += size
			eraCodeRequire(total <= eraCodeMaxBytes && len(pool) <= eraCodeMaxObjects,path,"pool bound")
		}
		replacements = append(replacements,replacement{account:account,hash:hash})
	}
	for _, value := range replacements {
		delete(value.account,"code")
		value.account["code_object"] = value.hash
	}
	document["code_objects"] = pool
	document["code_objects_schema"] = eraCodeObjectsSchema
	return pool
}

// Digest wire format shared with Python/Rust: compact JSON string map, sorted
// hash keys and exact canonical hex bodies. The envelope independently binds
// schema1, count and SHA256; each object key authenticates its Keccak bytes.
// Root-owned shard envelopes may use this binding without sharing namespaces.
func eraCodeObjectsBinding(pool map[string]string) map[string]any {
	encoded,err := json.Marshal(pool)
	if err != nil { panic(err) }
	return map[string]any{"code_objects_schema":eraCodeObjectsSchema,"code_objects_sha256":fmt.Sprintf("%x",sha256.Sum256(encoded)),
		"code_objects_count":len(pool)}
}
