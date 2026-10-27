// Declared synthetic pending-statistics inputs. Never used by VM/history APIs.
package main

import (
 "encoding/hex"
 "encoding/json"
 "math/big"
 "github.com/Taraxa-project/taraxa-evm/common"
 "github.com/Taraxa-project/taraxa-evm/core/types"
 "github.com/Taraxa-project/taraxa-evm/rlp"
 "github.com/Taraxa-project/taraxa-evm/taraxa/state/chain_config"
 dpos "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/precompiled"
 slashing "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/slashing/precompiled"
 contract_storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
 "github.com/Taraxa-project/taraxa-evm/taraxa/state/rewards_stats"
 "github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
)

func eraPendingStats(c chain_config.ChainConfig,input eraInput,latest *memoryLatest) (map[uint64]rewards_stats.RewardsStats,map[uint64]map[string]any) {
 cache:=map[uint64]rewards_stats.RewardsStats{};facts:=map[uint64]map[string]any{}
 periods:=[]uint64{};for _,p:=range input.Pending {if p<=input.FirstRetained {periods=append(periods,p)}}
 if len(periods)==0 {return cache,facts}
 low:=uint64(0);if periods[0]>1+uint64(c.DPOS.DelegationDelay) {low=periods[0]-1-uint64(c.DPOS.DelegationDelay)}
 high:=periods[len(periods)-1]-1
 boundaries:=[]uint64{c.Hardforks.FixRedelegateBlockNum,c.Hardforks.MagnoliaHf.BlockNum,c.Hardforks.PhalaenopsisHfBlockNum,c.Hardforks.FixClaimAllBlockNum,
  c.Hardforks.AspenHf.BlockNumPartOne,c.Hardforks.AspenHf.BlockNumPartTwo,c.Hardforks.FicusHf.BlockNum,c.Hardforks.CornusHf.BlockNum,c.Hardforks.CactiHf.BlockNum}
 for from:=range c.Hardforks.RewardsDistributionFrequency {boundaries=append(boundaries,from)}
 for _,boundary:=range boundaries {if low<boundary && boundary<=high {panic("constant synthetic pending domain crosses a rule activation")}}
 if input.Donor!=nil && input.Donor.Periods[len(input.Donor.Periods)-1]>=low {panic("donor does not predate earliest pending reader coordinate")}
 physical:=state_db.ExtendedReader{Reader:latest.readerAt(types.BlockNum(input.FirstRetained))}
 listKey:=common.BytesToHash([]byte{2});jailed:=[]common.Address{};ee:=*slashing.ContractAddress()
 physical.GetAccountStorage(&ee,&listKey,func(raw []byte){must(rlp.DecodeBytes(raw,&jailed))})
 for _,validator:=range jailed {
  key:=contract_storage.Stor_k_1([]byte{0},validator[:])
  physical.GetAccountStorage(&ee,key,func(raw []byte){var expiry uint64;must(rlp.DecodeBytes(raw,&expiry));if low<=expiry && expiry<=high {panic("constant synthetic pending domain crosses jail expiry")}})
 }
 var signature string
 for _,p:=range periods {
  requested:=[]uint64{}
  factory:=func(at types.BlockNum) contract_storage.StorageReader {
   if uint64(at)<low||uint64(at)>high {panic("pending logical evaluator outside declared domain")}
   requested=append(requested,uint64(at));return physical
  }
  prior:=new(dpos.API).Init(c).NewDelayedReader(types.BlockNum(p-1),factory)
  stats,fact:=eraStatsFromReader(c,p,new(big.Int),nil,prior)
  fact["transactions"]=[]map[string]any{}
  planner:=fact["planner_facts"].(map[string]any)
  current,_:=json.Marshal([]any{planner["total_eligible_vote_count"],planner["eligible_votes_by_validator"]})
  if signature=="" {signature=string(current)} else if signature!=string(current) {panic("synthetic pending eligibility is not constant")}
  planner["eligibility_reader"]=map[string]any{"kind":"declared_constant_logical_seed_evaluator","request_period":p-1,"factory_periods":requested,
   "logical_domain_first":low,"logical_domain_last":high,"physical_seed_period":input.FirstRetained,"logical_root":hex.EncodeToString(latest.descriptor.StateRoot[:]),
   "provenance":"declared synthetic statistics inputs; not physical historical-reader evidence"}
  cache[p]=stats;facts[p]=fact
 }
 return cache,facts
}
