// Disposable historical consumers over the same complete era histories.
package main

import (
 "bytes"
 "encoding/hex"
 "encoding/json"
 "fmt"
 "math/big"

 "github.com/Taraxa-project/taraxa-evm/common"
 "github.com/Taraxa-project/taraxa-evm/core/types"
 "github.com/Taraxa-project/taraxa-evm/core/vm"
 "github.com/Taraxa-project/taraxa-evm/crypto"
 "github.com/Taraxa-project/taraxa-evm/taraxa/state/chain_config"
 dpos "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/precompiled"
 slashing "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/slashing/precompiled"
 contract_storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
 "github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
 "github.com/Taraxa-project/taraxa-evm/taraxa/state/state_dry_runner"
)

var eraAPIConfiguration chain_config.ChainConfig

func eraBlock(period uint64) *vm.Block {
 gas:=uint64(315000000)
 if period>=eraAPIConfiguration.Hardforks.CornusHf.BlockNum {gas=eraAPIConfiguration.Hardforks.CornusHf.PbftGasLimit}
 return &vm.Block{Number:types.BlockNum(period),BlockInfo:vm.BlockInfo{Author:integrationValidators[1],GasLimit:gas,Difficulty:new(big.Int)}}
}

func eraCopiedView(source *memoryLatest,period uint64,root common.Hash) *memoryLatest {
 out:=newMemoryLatest()
 source.rows.mu.Lock()
 for column,rows:=range source.rows.physical {for key,value:=range rows {out.rows.physical[column][key]=common.CopyBytes(value)}}
 for column,rows:=range source.rows.latest {for key,value:=range rows {out.rows.latest[column][key]=common.CopyBytes(value)}}
 source.rows.mu.Unlock()
 out.rows.period=period
 out.descriptor=state_db.StateDescriptor{BlockNum:types.BlockNum(period),StateRoot:root}
 return out
}

func eraAPIs(latest *memoryLatest,cfg chain_config.ChainConfig,heads []uint64,ids catalogIdentities,addresses []common.Address,last uint64,ordinaryNonce uint64,ordinaryName string,validatorIndex int) []map[string]any {
 eraAPIConfiguration=cfg
 from:=addressForPrivateKey(integrationKeys[4])
 if validatorIndex<0 || validatorIndex>=len(integrationValidators) {panic("API validator input outside declared set")}
 a:=integrationValidators[validatorIndex]
 requests:=[]integrationAPIRequest{
  {name:"immediate_delegation",from:from,to:*dpos.ContractAddress(),input:integrationPack(false,"getTotalDelegation",from)},
  {name:"delayed_votes",from:from,to:*dpos.ContractAddress(),input:integrationPack(false,"getValidatorEligibleVotesCount",a)},
  {name:"delayed_jail",from:from,to:*slashing.ContractAddress(),input:integrationPack(true,"getJailBlock",a)},
  {name:"delayed_jail_list",from:from,to:*slashing.ContractAddress(),input:integrationPack(true,"getJailedValidators")},
  {name:"private_claim",from:from,to:*dpos.ContractAddress(),input:integrationPack(false,"claimRewards",a)},
  {name:"negative_short_selector",from:from,to:*dpos.ContractAddress(),input:[]byte{0x01}},
 }
 owner:=addressForPrivateKey(integrationKeys[0]);ordinary:=crypto.CreateAddress(&owner,new(big.Int).SetUint64(ordinaryNonce))
 requests=append(requests,integrationAPIRequest{name:ordinaryName,from:from,to:ordinary,value:1})
 factory:=func(p types.BlockNum) contract_storage.StorageReader {return state_db.ExtendedReader{Reader:latest.readerAt(p)}}
 before,_:=json.Marshal(latest.rows.exportPhysical())
 cases:=[]map[string]any{}
 for _,head:=range heads {
  if head>uint64(latest.descriptor.BlockNum) {panic("future historical API head")}
  for _,request:=range requests {
   probe:=func(gas uint64) map[string]any {
    runner:=new(state_dry_runner.DryRunner).Init(latest,func(types.BlockNum)*big.Int{return new(big.Int)},new(dpos.API).Init(cfg),factory,&cfg)
    tx:=request.transaction(gas,0)
    beginRawWriteTrace();result:=runner.Apply(eraBlock(head),&tx);writes:=finishRawWriteTrace()
    return map[string]any{"gas":gas,"effective_nonce":tx.Nonce.String(),"output":integrationAPIResult(result),"ordered_raw_writes":writes,"executed_input_after":hex.EncodeToString(tx.Input)}
   }
   call:=probe(fullTransactionGas);again:=probe(fullTransactionGas)
   x,_:=json.Marshal(call);y,_:=json.Marshal(again);if !bytes.Equal(x,y){panic("fresh historical probe changed result")}
   used:=call["output"].(map[string]any)["gas_used"].(uint64);lower:=uint64(0);if used>0 {lower=used-1}
   estimates:=[]map[string]any{}
   for _,cap:=range []uint64{fullTransactionGas,used,lower} {
    first:=probe(cap);probes:=[]map[string]any{first};out:=first["output"].(map[string]any)
    if out["consensus_error"]=="" && out["execution_error"]=="" {
     lo,hi:=out["gas_used"].(uint64),cap
     for hi-lo>hi/20 {mid:=lo+(hi-lo)/2;r:=probe(mid);probes=append(probes,r);out=r["output"].(map[string]any)
      if out["consensus_error"]!="" {break};if out["execution_error"]=="" {hi=mid}else{lo=mid}
     }
    }
    estimates=append(estimates,map[string]any{"name":fmt.Sprintf("%d/%s/%d",head,request.name,cap),"cap":cap,"probes":probes})
   }
   tracePeriod:=head+1;if tracePeriod>last {tracePeriod=head}
   trace:=integrationTrace(latest,cfg,tracePeriod,nil,request,ids,addresses)
   sequence:=integrationTrace(latest,cfg,tracePeriod,[]integrationAPIRequest{request},request,ids,addresses)
   cases=append(cases,map[string]any{"name":fmt.Sprintf("%d/%s",head,request.name),"head":head,"trace_period":tracePeriod,
    "request":integrationAPIInput(request),"call":call,"repeat_call":again,"estimates":estimates,"trace":trace,"trace_sequence":sequence})
  }
 }
 after,_:=json.Marshal(latest.rows.exportPhysical());if !bytes.Equal(before,after){panic("historical API mutated committed rows")}
 return cases
}
