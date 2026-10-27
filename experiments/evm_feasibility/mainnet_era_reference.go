// Finite mainnet-policy synthetic corpus. Run only through its pinned harness.
package main

import (
 "bytes"
 "crypto/sha256"
 "encoding/hex"
 "encoding/json"
 "flag"
 "fmt"
 "math/big"
 "os"
 "sort"
 "strings"

 "github.com/Taraxa-project/taraxa-evm/common"
 "github.com/Taraxa-project/taraxa-evm/core"
 "github.com/Taraxa-project/taraxa-evm/core/types"
 "github.com/Taraxa-project/taraxa-evm/core/vm"
 "github.com/Taraxa-project/taraxa-evm/crypto"
 "github.com/Taraxa-project/taraxa-evm/crypto/secp256k1"
 "github.com/Taraxa-project/taraxa-evm/rlp"
 "github.com/Taraxa-project/taraxa-evm/taraxa/state/chain_config"
 dpos "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/precompiled"
 slashing "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/slashing/precompiled"
 "github.com/Taraxa-project/taraxa-evm/taraxa/state/rewards_stats"
 "github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
 "github.com/Taraxa-project/taraxa-evm/taraxa/state/state_evm"
 "github.com/Taraxa-project/taraxa-evm/taraxa/state/state_transition"
 "github.com/Taraxa-project/taraxa-evm/taraxa/state/state_transition/op_stack"
 contract_storage "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/storage"
)

type eraInput struct {
 Configuration chain_config.ChainConfig `json:"configuration"`
 Name string `json:"name"`
 Head uint64 `json:"head"`
 FirstRetained uint64 `json:"first_retained"`
 Periods []uint64 `json:"periods"`
 Pending []uint64 `json:"pending_periods"`
 StopBeforeRewards uint64 `json:"stop_before_rewards"`
 Donor *eraInput `json:"donor,omitempty"`
 Actions []eraAction `json:"actions,omitempty"`
 NoWarmupDelegation bool `json:"no_warmup_delegation,omitempty"`
 SeedActionsOnly bool `json:"seed_actions_only,omitempty"`
 APIHeads []uint64 `json:"api_heads,omitempty"`
 APIOrdinaryNonce *uint64 `json:"api_ordinary_nonce,omitempty"`
 OrdinaryAPIName string `json:"ordinary_api_name,omitempty"`
 APIValidator int `json:"api_validator,omitempty"`
 Schedule []struct { Period uint64 `json:"period"`; Order []uint64 `json:"order"` } `json:"schedule"`
}

func eraAmount(s string) *big.Int { n,ok:=new(big.Int).SetString(s,10); if !ok {panic(s)}; return n }

func eraConfig(c chain_config.ChainConfig) chain_config.ChainConfig {
 c.GenesisBalances=core.BalanceMap{}
 for _,k:=range integrationKeys { c.GenesisBalances[addressForPrivateKey(k)]=eraAmount("100000000000000000000000000") }
 c.DPOS.InitialValidators=nil
 for i,v:=range integrationValidators {
  owner:=addressForPrivateKey(integrationKeys[i])
  c.DPOS.InitialValidators=append(c.DPOS.InitialValidators,chain_config.GenesisValidator{
   Address:v,Owner:owner,VrfKey:bytes.Repeat([]byte{byte(0x44+i)},32),Commission:uint16(100+i*50),
   Description:"synthetic mainnet-policy validator",Delegations:core.BalanceMap{owner:eraAmount("1000000000000000000000000")},
  })
 }
 return c
}

// Projection genesis only supplies explicitly declared synthetic logical inputs.
// Its activation coordinates are NEVER used by subsequent period execution.
func eraProjection(c chain_config.ChainConfig, at uint64) chain_config.ChainConfig {
 active:=func(b uint64) uint64 {if b<=at{return 0};return maxPeriod}
 c.Hardforks.FixRedelegateBlockNum=active(c.Hardforks.FixRedelegateBlockNum)
 c.Hardforks.Redelegations=nil // the seed is not the correction transition
 c.Hardforks.MagnoliaHf.BlockNum=active(c.Hardforks.MagnoliaHf.BlockNum)
 c.Hardforks.PhalaenopsisHfBlockNum=active(c.Hardforks.PhalaenopsisHfBlockNum)
 c.Hardforks.FixClaimAllBlockNum=active(c.Hardforks.FixClaimAllBlockNum)
 c.Hardforks.AspenHf.BlockNumPartOne=active(c.Hardforks.AspenHf.BlockNumPartOne)
 c.Hardforks.AspenHf.BlockNumPartTwo=active(c.Hardforks.AspenHf.BlockNumPartTwo)
 c.Hardforks.FicusHf.BlockNum=active(c.Hardforks.FicusHf.BlockNum)
 c.Hardforks.CornusHf.BlockNum=active(c.Hardforks.CornusHf.BlockNum)
 c.Hardforks.SoleiroliaHf.BlockNum=active(c.Hardforks.SoleiroliaHf.BlockNum)
 c.Hardforks.CactiHf.BlockNum=active(c.Hardforks.CactiHf.BlockNum)
 return c
}

func eraSeed(c chain_config.ChainConfig, at uint64) (*memoryLatest,catalogIdentities,map[string]any,[]common.Address) {
 projected:=eraProjection(c,at)
 source:=newMemoryLatest()
 beginRawWriteTrace()
 projection:=newStateTransition(source,&projected)
 writes:=finishRawWriteTrace()
 projection.Close()
 addresses:=[]common.Address{*dpos.ContractAddress(),common.HexToAddress("ee"),c.Hardforks.FicusHf.BridgeContractAddress}
 addresses=append(addresses,integrationValidators...)
 for _,k:=range integrationKeys {addresses=append(addresses,addressForPrivateKey(k))}
 // Include all registered stateless and OP-stack addresses in the complete seed
 // inventory. This closes accounts installed by the actual genesis HF path.
 for i:=1;i<=0x13;i++ {addresses=append(addresses,common.BytesToAddress(big.NewInt(int64(i)).Bytes()))}
 for i:=int64(0xf0);i<=0xff;i++ {addresses=append(addresses,common.BytesToAddress(big.NewInt(i).Bytes()))}
 addresses=append(addresses,common.HexToAddress("0100"),common.HexToAddress("fa1c"))
 for address:=range op_stack.OpPrecompiles {addresses=append(addresses,address)}
 // This is the observer's declared input list. Keep every duplicate and all
 // later CREATE append occurrences, but remove Go-map iteration randomness
 // before any retained callback or execution consumes this initial list.
 sort.SliceStable(addresses,func(i,j int)bool{return bytes.Compare(addresses[i][:],addresses[j][:])<0})
 ids:=catalogIdentities{Accounts:addresses}
 for _,w:=range writes {ids.Slots=append(ids.Slots,catalogSlot{Address:common.HexToAddress(w.Address),Key:common.HexToHash(w.Key)})}
 ids=catalogIdentityClosure(mergeCatalogs(catalogIdentities{},ids))
 reader:=state_db.ExtendedReader{Reader:source.readerAt(0)}
 declared:=captureNativeCatalog(reader,ids)
 latest:=newMemoryLatest()
 first:=at-5
 latest.descriptor.BlockNum=types.BlockNum(first-1)
 pending:=latest.BeginPendingBlock()
 sink:=new(state_transition.TrieSink).Init(nil,state_transition.TrieSinkOpts{})
 sink.SetIO(pending)
 for _,a:=range ids.Accounts {
  address:=a
  reader.GetAccount(&address,func(account state_db.Account){
   change:=state_evm.AccountChange{Account:account,RawStorageDirty:state_evm.RawStorage{}}
   change.StorageRootHash=nil
   if account.CodeHash!=nil {change.Code=reader.GetCode(account.CodeHash);change.CodeDirty=true}
   for _,slot:=range ids.Slots {if slot.Address==address {key:=slot.Key;reader.GetAccountStorage(&address,&key,func(v []byte){change.RawStorageDirty[key]=common.CopyBytes(v)})}}
   mutation:=sink.StartMutation(&address);mutation.Update(change);mutation.Commit()
  })
 }
 root:=sink.Commit();sink.Close();must(latest.Commit(root))
 if root!=source.descriptor.StateRoot {panic("declared complete seed does not reconstruct projection root")}
 history:=[]map[string]any{{"period":first,"root":hex.EncodeToString(root[:])}}
 for p:=first+1;p<=at;p++ {
  pending:=latest.BeginPendingBlock()
  writer:=new(state_transition.TrieSink).Init(&root,state_transition.TrieSinkOpts{})
  writer.SetIO(pending);root=writer.Commit();writer.Close();must(latest.Commit(root))
  history=append(history,map[string]any{"period":p,"root":hex.EncodeToString(root[:])})
 }
 check:=state_db.ExtendedReader{Reader:latest.readerAt(types.BlockNum(at))}
 result:=map[string]any{"period":at,"root":hex.EncodeToString(root[:]),"history":history,"native_catalog":captureNativeCatalog(check,ids),
  "declared_projection_catalog":declared,"projection_configuration":configurationJSON(projected,1000),
  "rows":latest.rows.exportPhysical(),"latest_rows":latest.rows.exportLatest(),"ordered_raw_writes":writes,
  "provenance":"explicit synthetic logical inputs, recomputed by real TrieSink at the retained coordinate; not historical replay"}
 return latest,ids,result,addresses
}

type eraTransaction struct { fullTransactionSpec; amount *big.Int }

func eraVote(index int, hash byte, period uint64) []byte {
 v:=slashing.Vote{BlockHash:common.Hash{hash},VrfSortition:slashing.VrfPbftSortition{Period:period,Round:2,Step:4}}
 v.VrfSortitionBytes=rlp.MustEncodeToBytes(v.VrfSortition)
 sig,err:=secp256k1.Sign(v.GetHash().Bytes(),integrationValidatorKeys[index]);must(err);copy(v.Signature[:],sig)
 return rlp.MustEncodeToBytes(v)
}

func eraWritten(ids catalogIdentities,writes []rawWrite) catalogIdentities {
 extra:=catalogIdentities{}
 for _,w:=range writes {address:=common.HexToAddress(w.Address);extra.Accounts=append(extra.Accounts,address);extra.Slots=append(extra.Slots,catalogSlot{Address:address,Key:common.HexToHash(w.Key)})}
 return mergeCatalogs(ids,extra)
}

func eraSigned(spec eraTransaction, nonce *big.Int) signedTransaction {
 sender:=addressForPrivateKey(spec.PrivateKey)
 unsigned:=unsignedLegacyTransaction{Nonce:nonce,GasPrice:new(big.Int).SetUint64(spec.GasPrice),Gas:spec.Gas,To:spec.To,
  Value:spec.amount,Input:spec.Input,ChainID:new(big.Int).SetUint64(chainID),ZeroR:new(big.Int),ZeroS:new(big.Int)}
 digest:=crypto.Keccak256Hash(rlp.MustEncodeToBytes(&unsigned))
 sig,err:=secp256k1.Sign(digest[:],spec.PrivateKey);must(err)
 decoded:=signedLegacyTransaction{Nonce:nonce,GasPrice:unsigned.GasPrice,Gas:spec.Gas,To:spec.To,Value:spec.amount,Input:spec.Input,
  V:new(big.Int).SetUint64(chainID*2+35+uint64(sig[64])),R:new(big.Int).SetBytes(sig[:32]),S:new(big.Int).SetBytes(sig[32:64])}
 body:=rlp.MustEncodeToBytes(&decoded)
 return signedTransaction{RLP:body,Hash:crypto.Keccak256Hash(body),Decoded:decoded,Sender:sender,ChainID:chainID}
}

func eraTransactions(input eraInput,p uint64) []eraTransaction {
 rows:=[]eraTransaction{}
 add:=func(who int,name string,to *common.Address,value *big.Int,data []byte) {
  spec:=fullSpec(name,0,addressForPrivateKey(integrationKeys[who]),integrationKeys[who],to,0,data)
  spec.GasPrice=1_000_000_000;spec.Gas=500_000
  rows=append(rows,eraTransaction{spec,value})
 }
 zero:=new(big.Int)
 a,b:=integrationValidators[0],integrationValidators[1]
 if p==input.FirstRetained+1 && !input.NoWarmupDelegation {add(3,"seed_delegation",dpos.ContractAddress(),eraAmount("3000000000000000000000"),integrationPack(false,"delegate",a))}
 if input.Name=="mainnet_106" {
  switch p {
  case 25706895:
   newKey:=bytes.Repeat([]byte{12},32);newValidator:=addressForPrivateKey(newKey)
   sig,err:=secp256k1.Sign(crypto.Keccak256(newValidator[:]),newKey);must(err);sig[64]+=27
   add(0,"registration",dpos.ContractAddress(),eraAmount("1000000000000000000000000"),integrationPack(false,"registerValidator",newValidator,sig,bytes.Repeat([]byte{0x77},32),uint16(100),"new","endpoint"))
   add(0,"create_storage_value_logs",nil,big.NewInt(7),initRuntime(common.FromHex("6001600055600160006000a160006000f3")))
   add(0,"create_parent_revert",nil,zero,initRuntime(common.FromHex("36600060003760006000366000600060ee620186a0f160006000fd")))
   for _,v:=range integrationValidators {add(4,"delegate",dpos.ContractAddress(),eraAmount("2000000000000000000000"),integrationPack(false,"delegate",v))}
  case 25706896:
   owner:=addressForPrivateKey(integrationKeys[0]);ordinary:=crypto.CreateAddress(&owner,big.NewInt(1))
   add(0,"ordinary_storage_value_log",&ordinary,eraAmount("100000000000000000000"),nil)
  case 25706901:
   add(4,"rewarded_redelegate",dpos.ContractAddress(),zero,integrationPack(false,"reDelegate",a,b,eraAmount("1000000000000000000000")))
  case 25706902:
   add(4,"full_source_removal",dpos.ContractAddress(),zero,integrationPack(false,"reDelegate",a,b,eraAmount("1000000000000000000000")))
  case 25706903:
   add(4,"claim",dpos.ContractAddress(),zero,integrationPack(false,"claimRewards",b))
   add(0,"claim_commission",dpos.ContractAddress(),zero,integrationPack(false,"claimCommissionRewards",a))
  case 25706904:
   add(4,"custody_v1",dpos.ContractAddress(),zero,integrationPack(false,"undelegate",b,eraAmount("1000000000000000000000")))
   add(4,"cancel_v1",dpos.ContractAddress(),zero,integrationPack(false,"cancelUndelegate",b))
   add(4,"custody_v2",dpos.ContractAddress(),zero,integrationPack(false,"undelegateV2",b,eraAmount("1000000000000000000000")))
   add(4,"cancel_v2",dpos.ContractAddress(),zero,integrationPack(false,"cancelUndelegateV2",b,uint64(1)))
  case 25706906:
   proof:=integrationPack(true,"commitDoubleVotingProof",eraVote(0,1,25706901),eraVote(0,2,25706901))
   add(4,"slash_a",slashing.ContractAddress(),zero,proof)
   add(4,"slash_duplicate",slashing.ContractAddress(),zero,proof)
   owner:=addressForPrivateKey(integrationKeys[0]);parent:=crypto.CreateAddress(&owner,big.NewInt(2))
   add(4,"parent_revert_native_slash",&parent,zero,integrationPack(true,"commitDoubleVotingProof",eraVote(2,3,25706901),eraVote(2,4,25706901)))
  case 25706907,25706911,25706912:
   add(4,"delayed_jail",slashing.ContractAddress(),zero,integrationPack(true,"getJailBlock",a))
   add(4,"delayed_jailed_list",slashing.ContractAddress(),zero,integrationPack(true,"getJailedValidators"))
   add(4,"delayed_eligibility",dpos.ContractAddress(),zero,integrationPack(false,"getValidatorEligibleVotesCount",a))
  case 25706960:
   add(0,"set_commission_mainnet_frequency",dpos.ContractAddress(),zero,integrationPack(false,"setCommission",a,uint16(150)))
   add(4,"claim_all",dpos.ContractAddress(),zero,integrationPack(false,"claimAllRewards"))
  }
 }
 return append(rows,eraDeclaredTransactions(input.Actions,p)...)
}

func eraStats(c chain_config.ChainConfig,p uint64,fees *big.Int,hashes []string) (rewards_stats.RewardsStats,map[string]any) {
 rate:=c.DPOS.BlocksPerYear
 if p>=c.Hardforks.CactiHf.BlockNum {rate=uint32(uint64(365*24*60*60*1000)/(2*1000+uint64(c.Hardforks.CactiHf.ConsensusDelay)))}
 stats:=rewards_stats.RewardsStats{BlockAuthor:integrationValidators[1],BlocksPerYear:rate,ValidatorsStats:map[common.Address]rewards_stats.ValidatorStats{},MaxVotesWeight:1000}
 validators:=[]map[string]any{}; dags:=[]map[string]any{}; certs:=[]map[string]any{}
 for i,v:=range integrationValidators {
  fee:=new(big.Int);txs:=[]string{}
  if i==1 {fee.Set(fees);txs=append([]string{},hashes...)}
  dag:=uint32(1)
  if p<c.Hardforks.AspenHf.BlockNumPartOne && len(txs)==0 {dag=0}
  rewardFee:=new(big.Int).Set(fee);if p<c.Hardforks.MagnoliaHf.BlockNum {rewardFee.SetUint64(0)}
  weight:=uint64(333);if i==0 {weight=334}
  stats.ValidatorsStats[v]=rewards_stats.ValidatorStats{DagBlocksCount:dag,VoteWeight:weight,FeesRewards:rewardFee}
  stats.TotalDagBlocksCount+=dag;stats.TotalVotesWeight+=weight
  validators=append(validators,map[string]any{"validator":hex.EncodeToString(v[:]),"dag_blocks_count":dag,"vote_weight":weight,"fees_reward":rewardFee.String(),"reward_key":hex.EncodeToString(crypto.Keccak256([]byte{0,2},v[:]))})
  dags=append(dags,map[string]any{"author":hex.EncodeToString(v[:]),"difficulty":1,"transaction_hashes":txs})
  certs=append(certs,map[string]any{"validator":hex.EncodeToString(v[:]),"period":p,"weight":weight})
 }
 return stats,map[string]any{"number":p,"block_author":hex.EncodeToString(stats.BlockAuthor[:]),"blocks_per_year":rate,
  "total_dag_blocks_count":stats.TotalDagBlocksCount,"total_votes_weight":stats.TotalVotesWeight,"max_votes_weight":stats.MaxVotesWeight,"validators":validators,
  "planner_facts":map[string]any{"dag_blocks":dags,"certificate_votes":certs,"total_eligible_vote_count":3000,"committee_size":1000}}
}

func eraLiveStats(c chain_config.ChainConfig,p uint64,fees *big.Int,hashes []string,latest *memoryLatest) (rewards_stats.RewardsStats,map[string]any) {
 requested:=[]uint64{}
 factory:=func(period types.BlockNum) contract_storage.StorageReader {requested=append(requested,uint64(period));return state_db.ExtendedReader{Reader:latest.readerAt(period)}}
 prior:=new(dpos.API).Init(c).NewDelayedReader(types.BlockNum(p-1),factory)
 stats,fact:=eraStatsFromReader(c,p,fees,hashes,prior)
 fact["planner_facts"].(map[string]any)["eligibility_reader"]=map[string]any{"kind":"actual_delayed_history","request_period":p-1,"factory_periods":requested,
  "source_chain":"Stats::getBlockStats -> FinalChain::dposEligibleTotalVoteCount -> state_api -> DPOSDelayedReader -> NewDelayedReader"}
 return stats,fact
}

func eraStatsFromReader(c chain_config.ChainConfig,p uint64,fees *big.Int,hashes []string,prior dpos.Reader) (rewards_stats.RewardsStats,map[string]any) {
 stats,fact:=eraStats(c,p,fees,hashes)
 eligible:=prior.TotalEligibleVoteCount()
 counts:=[]uint64{prior.GetEligibleVoteCount(&integrationValidators[0]),prior.GetEligibleVoteCount(&integrationValidators[1]),prior.GetEligibleVoteCount(&integrationValidators[2])}
 weights:=[]uint64{0,0,0}
 if counts[0]>0 {weights[0]=334}
 if counts[2]>0 {weights[2]=333}
 weights[1]=1000-weights[0]-weights[2]
 for i,weight:=range weights {if counts[i]<weight {panic("synthetic certificate exceeds source reader eligibility")}}
 validators:=[]map[string]any{};dags:=[]map[string]any{};certs:=[]map[string]any{}
 stats.TotalDagBlocksCount=0;stats.TotalVotesWeight=0;stats.MaxVotesWeight=eligible;if stats.MaxVotesWeight>1000 {stats.MaxVotesWeight=1000}
 for i,v:=range integrationValidators {
  row:=stats.ValidatorsStats[v]
  if weights[i]==0 {delete(stats.ValidatorsStats,v);continue}
  row.VoteWeight=weights[i];stats.ValidatorsStats[v]=row;stats.TotalDagBlocksCount+=row.DagBlocksCount;stats.TotalVotesWeight+=weights[i]
  item:=fact["validators"].([]map[string]any)[i];item["vote_weight"]=weights[i];validators=append(validators,item)
  dags=append(dags,fact["planner_facts"].(map[string]any)["dag_blocks"].([]map[string]any)[i])
  certs=append(certs,map[string]any{"validator":hex.EncodeToString(v[:]),"period":p,"weight":weights[i]})
 }
 fact["validators"]=validators;fact["total_dag_blocks_count"]=stats.TotalDagBlocksCount;fact["total_votes_weight"]=stats.TotalVotesWeight;fact["max_votes_weight"]=stats.MaxVotesWeight
 fact["planner_facts"]=map[string]any{"dag_blocks":dags,"certificate_votes":certs,"total_eligible_vote_count":eligible,"validator_eligible_vote_count":counts[1],"eligible_votes_by_validator":counts,"committee_size":1000,"lambda_ms":1000}
 return stats,fact
}

func eraRun(input eraInput,observer bool,source *eraSeedSource) (out map[string]any) {
 cfg:=eraConfig(input.Configuration)
 depth:=0;for donor:=input.Donor;donor!=nil;donor=donor.Donor {depth++;if depth>3 {panic("bounded donor hierarchy exceeded")}}
 if input.SeedActionsOnly && !strings.HasSuffix(input.Name,"_donor") {panic("only declared donor prefixes may omit workload APIs")}
 if source!=nil && input.StopBeforeRewards!=0 {panic("donor must be fully committed")}
 var latest *memoryLatest;var ids catalogIdentities;var seed map[string]any;var addresses []common.Address
 if input.Donor==nil {
  latest,ids,seed,addresses=eraSeed(cfg,input.FirstRetained)
 } else {
  if input.Donor.StopBeforeRewards!=0 || !input.Donor.SeedActionsOnly {panic("bounded committed donor source required")}
  before,_:=json.Marshal(input.Configuration);after,_:=json.Marshal(input.Donor.Configuration)
  if !bytes.Equal(before,after) {panic("donor mainnet configuration mismatch")}
  donorSource:=new(eraSeedSource)
  donor:=eraRun(*input.Donor,observer,donorSource)
  latest,ids,seed,addresses=eraRebasedSeed(cfg,input.FirstRetained,donorSource.Latest,donorSource.IDs,donorSource.Addresses,donorSource.Writes)
  seed["donor_document"]=donor
 }
 if source!=nil {defer func(){source.Latest=latest;source.IDs=ids;source.Addresses=append([]common.Address(nil),addresses...);source.Document=out;source.Writes=eraSourceWrites(out)}()}
 st:=newStateTransition(latest,&cfg)
 ordinaryNonce:=uint64(1);if input.APIOrdinaryNonce!=nil {ordinaryNonce=*input.APIOrdinaryNonce}
 ordinaryName:=input.OrdinaryAPIName;if ordinaryName=="" {ordinaryName="ordinary_storage_value_log"}
 cache,facts:=eraPendingStats(cfg,input,latest)
 pending:=[]map[string]any{}
 schedule:=map[uint64][]uint64{}
 for _,row:=range input.Schedule {schedule[row.Period]=row.Order}
 periods:=[]map[string]any{}
 for p:=input.FirstRetained+1;p<=input.Periods[len(input.Periods)-1];p++ {
  priorRoot:=latest.descriptor.StateRoot
  beginRawWriteTrace();eraAPIConfiguration=cfg;st.BeginBlock(&eraBlock(p).BlockInfo);begin:=finishRawWriteTrace()
  ids=eraWritten(ids,begin)
  txs:=[]map[string]any{};hashes:=[]string{};fees:=new(big.Int);var gas uint64
  for i,spec:=range eraTransactions(input,p) {
   sender:=addressForPrivateKey(spec.PrivateKey);nonce:=new(big.Int)
   prior:=state_db.ExtendedReader{Reader:latest.readerAt(types.BlockNum(p-1))}
   prior.GetAccount(&sender,func(a state_db.Account){nonce.Set(a.Nonce)})
   // Multiple calls in this period read the private working nonce explicitly.
   for _,row:=range txs {if row["sender"]==hex.EncodeToString(sender[:]) {nonce.Add(nonce,big.NewInt(1))}}
   signed:=eraSigned(spec,nonce)
   tx:=vm.Transaction{From:sender,To:signed.Decoded.To,Nonce:signed.Decoded.Nonce,GasPrice:signed.Decoded.GasPrice,Gas:signed.Decoded.Gas,Value:signed.Decoded.Value,Input:common.CopyBytes(signed.Decoded.Input)}
   beginRawWriteTrace();beginGasRefundTrace();result:=st.ExecuteTransaction(&tx);refund:=finishGasRefundTrace();writes:=finishRawWriteTrace();gas+=result.GasUsed
   ids=eraWritten(ids,writes)
   if tx.To==nil && result.ExecutionErr=="" && result.ConsensusErr=="" {addresses=append(addresses,result.NewContractAddr)}
   original:=tx;original.Input=common.CopyBytes(signed.Decoded.Input)
   spec.ExpectedError=string(result.ExecutionErr)
   row:=fullTransactionRow(spec.fullTransactionSpec,signed,original,result,gas);row["nonce"]=nonce.String();row["ordered_raw_writes"]=writes;row["index"]=i;row["gas_refund"]=refund
   row["executed_input_after"]=hex.EncodeToString(tx.Input)
   if observer {root,catalog,r:=observerFinalizeTransaction(st,uint64(i));ids=mergeCatalogs(ids,catalogIdentityClosure(catalog));row["observer"]=map[string]any{"root":hex.EncodeToString(root[:]),"catalog":captureCatalog(r,catalog),"catalog_identities":jsonValue(catalog)}}
   fee:=new(big.Int).Mul(new(big.Int).SetUint64(result.GasUsed),tx.GasPrice);fees.Add(fees,fee);row["fee"]=fee.String()
   txs=append(txs,row);hashes=append(hashes,hex.EncodeToString(signed.Hash[:]))
  }
  stats,fact:=eraLiveStats(cfg,p,fees,hashes,latest);facts[p]=fact;cache[p]=stats
  fact["transactions"]=txs
  if input.StopBeforeRewards==p {
   // Independent replay fork: use unchanged PrepareCommit and stop before
   // rewards/EndBlock. The computed root is a private query-view identity.
   root:=st.PrepareCommit()
   view:=eraCopiedView(latest,p,root)
   reader:=state_db.ExtendedReader{Reader:view.readerAt(types.BlockNum(p))}
   queries:=eraAPIs(view,cfg,[]uint64{p},ids,addresses,p,ordinaryNonce,ordinaryName,input.APIValidator)
   st.Close()
   return map[string]any{"schema":1,"kind":"pre_reward_fork","profile":"synthetic_mainnet_era","scenario":input.Name,"head":input.Head,
    "prefix_periods":periods,"pre_reward":map[string]any{"number":p,"root":hex.EncodeToString(root[:]),"accounts":observeAddressSet(reader,addresses),"native_catalog":captureNativeCatalog(reader,ids)},
    "seed_identity":map[string]any{"period":seed["period"],"root":seed["root"],"history":seed["history"]},"api_cases":queries,"history_rows":view.rows.exportPhysical()}
  }
  order,exists:=schedule[p]
  if !exists {panic(fmt.Sprintf("missing authenticated schedule for %d",p))}
  distributions:=[]map[string]any{};minted:=new(big.Int);allWrites:=[]rawWrite{}
  for _,sourcePeriod:=range order {
   row,ok:=cache[sourcePeriod];if !ok {panic(fmt.Sprintf("missing pending reward row %d",sourcePeriod))}
   beginRawWriteTrace();reward:=st.DistributeRewards(&row);writes:=finishRawWriteTrace();minted.Add(minted,reward.ToBig());allWrites=append(allWrites,writes...)
   ids=eraWritten(ids,writes)
   distributions=append(distributions,map[string]any{"period":sourcePeriod,"input":facts[sourcePeriod],"ordered_raw_writes":writes,"minted_reward":reward.ToBig().String()})
  }
  if len(order)>0 {cache=map[uint64]rewards_stats.RewardsStats{}}
  beginRawWriteTrace();st.EndBlock();end:=finishRawWriteTrace()
  ids=eraWritten(ids,end)
  if observer {observerRecordRewards(st)}
  root:=st.PrepareCommit()
  var periodIDs any
  if observer {catalog,_:=observerPeriodCatalog(st);ids=mergeCatalogs(ids,catalogIdentityClosure(catalog));periodIDs=jsonValue(catalog)}
  committed:=st.Commit();if root!=committed {panic("prepared/committed mismatch")}
  r:=state_db.ExtendedReader{Reader:latest.readerAt(types.BlockNum(p))}
  periods=append(periods,map[string]any{"number":p,"prior_root":hex.EncodeToString(priorRoot[:]),"seed_prefix":p<=input.Head,"rules":map[string]bool{"cornus":p>=cfg.Hardforks.CornusHf.BlockNum,"ficus":p>=cfg.Hardforks.FicusHf.BlockNum,"cacti":p>=cfg.Hardforks.CactiHf.BlockNum},"transactions":txs,"planner_facts":fact["planner_facts"],"reward_input":fact,
   "distributions":distributions,"distribution_order":order,"period_catalog_identities":periodIDs,"reward_output":map[string]any{"minted_reward":minted.String(),"actual_transaction_fees":fees.String()},
   "ordered_raw_writes":map[string]any{"begin_block":begin,"rewards":allWrites,"end_block":end},
   "final":map[string]any{"root":hex.EncodeToString(root[:]),"committed_root":hex.EncodeToString(committed[:]),"accounts":observeAddressSet(r,addresses),"native_catalog":captureNativeCatalog(r,ids)}})
  if p==25706905 {st.Close();st=newStateTransition(latest,&cfg)}
 }
 st.Close()
 heads:=[]uint64{}
 if input.Name=="mainnet_106" {heads=[]uint64{input.Head,25706900,25706905,25706906,25706911,25707000}} else {heads=input.Periods}
 if len(input.APIHeads)>0 {heads=input.APIHeads}
 apiCases:=[]map[string]any{}
 if !input.SeedActionsOnly {apiCases=eraAPIs(latest,cfg,heads,ids,addresses,input.Periods[len(input.Periods)-1],ordinaryNonce,ordinaryName,input.APIValidator)}
 config:=configurationJSON(cfg,1000)
 corrections:=[]map[string]any{};for _,r:=range cfg.Hardforks.Redelegations {corrections=append(corrections,map[string]any{"validator":hex.EncodeToString(r.Validator[:]),"delegator":hex.EncodeToString(r.Delegator[:]),"amount":r.Amount.String()})}
 config["hardforks"].(map[string]any)["redelegations"]=corrections
 freq:=[]map[string]any{};for p,f:=range cfg.Hardforks.RewardsDistributionFrequency {freq=append(freq,map[string]any{"start_period":p,"frequency":f})};sort.Slice(freq,func(i,j int)bool{return freq[i]["start_period"].(uint64)<freq[j]["start_period"].(uint64)})
 config["hardforks"].(map[string]any)["rewards_distribution_frequency"]=freq
 config["block_gas_limit"]=cfg.Hardforks.CornusHf.PbftGasLimit
 genesisLatest:=newMemoryLatest();beginRawWriteTrace();genesisTransition:=newStateTransition(genesisLatest,&cfg);genesisWrites:=finishRawWriteTrace();genesisTransition.Close()
 genesisIDs:=eraWritten(ids,genesisWrites)
 genesisReader:=state_db.ExtendedReader{Reader:genesisLatest.readerAt(0)}
 genesis:=map[string]any{"period":0,"root":hex.EncodeToString(genesisLatest.descriptor.StateRoot[:]),"native_catalog":captureNativeCatalog(genesisReader,genesisIDs)}
 allocations:=[]map[string]string{};for a,n:=range cfg.GenesisBalances {allocations=append(allocations,map[string]string{"address":hex.EncodeToString(a[:]),"balance":n.String()})};sort.Slice(allocations,func(i,j int)bool{return allocations[i]["address"]<allocations[j]["address"]})
 validators:=[]map[string]any{};for _,v:=range cfg.DPOS.InitialValidators {delegations:=[]map[string]string{};for a,n:=range v.Delegations {delegations=append(delegations,map[string]string{"delegator":hex.EncodeToString(a[:]),"amount":n.String()})};validators=append(validators,map[string]any{"address":hex.EncodeToString(v.Address[:]),"owner":hex.EncodeToString(v.Owner[:]),"vrf_key":hex.EncodeToString(v.VrfKey),"commission":v.Commission,"description":v.Description,"endpoint":v.Endpoint,"delegations":delegations})}
 pending=nil;for _,p:=range input.Pending {pending=append(pending,facts[p])}
 reopen:=uint64(0);if input.FirstRetained<25706905 && input.Periods[len(input.Periods)-1]>=25706905 {reopen=25706905}
 return map[string]any{"schema":1,"profile":"synthetic_mainnet_era","scenario":input.Name,"head":input.Head,"configuration":config,"genesis":genesis,
  "inputs":map[string]any{"genesis_allocations":allocations,"initial_validators":validators,"initial_validator":validators[0]},
  "seed":seed,"pending_reward_facts":pending,"periods":periods,"api_cases":apiCases,"history_rows":latest.rows.exportPhysical(),"go_row_model_reopen_after":reopen,
  "cpp_scheduler_seed_reopen_after":input.FirstRetained,"seed_actions_only":input.SeedActionsOnly}
}

func main() {
 inputPath:=flag.String("input","","pinned explicit synthetic inputs");observer:=flag.Bool("observer",false,"local concrete observer")
 output:=flag.String("output","","base, periods, api, fork, or fork_api; selection occurs after complete execution")
 first:=flag.Int("first",-1,"first period or API index, inclusive")
 end:=flag.Int("end",-1,"last period or API index, exclusive")
 reference:=flag.String("reference","","verified source commit")
 flag.Parse();body,err:=os.ReadFile(*inputPath);must(err);var input eraInput;must(json.Unmarshal(body,&input))
 // Validate syntax before executing. Membership is checked against the complete
 // result after execution; no selector can shorten or alter its state lifetime.
 switch *output {case "base","fork":if *first!=-1||*end!=-1 {panic("base bounds are forbidden")};case "periods","api","fork_api":if *first<0||*end<=*first {panic("invalid selector bounds")};default:panic("unknown output selector")}
 if len(*reference)!=40 {panic("missing source commit binding")}
 if (input.StopBeforeRewards!=0)!=(*output=="fork"||*output=="fork_api") {panic("inconsistent fork selector")}
 out:=eraRun(input,*observer,nil)
 codePool:=eraEncodeCodeObjects(out)
 codeBinding:=eraCodeObjectsBinding(codePool)
 for key,value:=range codeBinding {out[key]=value}
 selected:=map[string]any{}
 var rows []map[string]any
 if input.StopBeforeRewards!=0 {rows=out["prefix_periods"].([]map[string]any)} else {rows=out["periods"].([]map[string]any)}
 continuity:=[]map[string]any{}
 for _,row:=range rows {hashes:=[]string{};for _,tx:=range row["transactions"].([]map[string]any) {hashes=append(hashes,tx["hash"].(string))};continuity=append(continuity,map[string]any{"period":row["number"],"prior_root":row["prior_root"],"root":row["final"].(map[string]any)["root"],"transaction_hashes":hashes})}
 names:=[]string{};caseIdentities:=[]string{}
 for _,row:=range out["api_cases"].([]map[string]any) {
  names=append(names,row["name"].(string));identity,_:=json.Marshal(map[string]any{"head":row["head"],"request":row["request"],"trace_period":row["trace_period"]});caseIdentities=append(caseIdentities,fmt.Sprintf("%x",sha256.Sum256(identity)))
 }
 switch *output {
 case "base","fork":
  for k,v:=range out {if k!="periods"&&k!="prefix_periods"&&k!="api_cases" {selected[k]=v}}
 case "periods":
  if *end-*first>20 {panic("period shard exceeds20 periods")}
  subset:=[]map[string]any{};for _,row:=range rows {p:=int(row["number"].(uint64));if p>=*first&&p<*end {subset=append(subset,row)}}
  if len(subset)!=*end-*first {panic("period selector is outside expected history")};selected["periods"]=subset
 case "api","fork_api":
  cases:=out["api_cases"].([]map[string]any);if *end-*first>7||*end>len(cases) {panic("API selector is outside expected cases")};selected["api_cases"]=cases[*first:*end]
 }
 cfg,_:=json.Marshal(input.Configuration);inputDigest:=sha256.Sum256(body)
 semanticInput:=input;semanticInput.StopBeforeRewards=0;semantic,_:=json.Marshal(semanticInput)
 seedIdentity:=out["seed_identity"]
 if seedIdentity==nil {s:=out["seed"].(map[string]any);seedIdentity=map[string]any{"period":s["period"],"root":s["root"],"history":s["history"]}}
 history,_:=json.Marshal(out["history_rows"])
 envelope:=map[string]any{"schema":3,"reference":*reference,"scenario":input.Name,"starting_head":input.Head,
  "input_sha256":fmt.Sprintf("%x",inputDigest),"execution_input_sha256":fmt.Sprintf("%x",sha256.Sum256(semantic)),
  "configuration_sha256":fmt.Sprintf("%x",sha256.Sum256(cfg)),"history_sha256":fmt.Sprintf("%x",sha256.Sum256(history)),
  "observer":*observer,"stop_before_rewards":input.StopBeforeRewards,"seed_identity":seedIdentity,
  "continuity":continuity,"api_case_names":names,"api_case_identities":caseIdentities,"selector":map[string]any{"kind":*output,"first":*first,"end":*end},"payload":selected}
 for key,value:=range codeBinding {envelope[key]=value}
 encoded,err:=json.Marshal(envelope);must(err);fmt.Println(string(encoded))
}
