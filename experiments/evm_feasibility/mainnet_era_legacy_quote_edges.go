// Additive finite quote-edge probe; compiled only in a pinned disposable era harness.
package main

import (
    "bytes"
    "encoding/binary"
    "encoding/hex"
    "fmt"
    "math/big"

    "github.com/Taraxa-project/taraxa-evm/common"
    "github.com/Taraxa-project/taraxa-evm/core/types"
    "github.com/Taraxa-project/taraxa-evm/core/vm"
    "github.com/Taraxa-project/taraxa-evm/crypto"
    "github.com/Taraxa-project/taraxa-evm/taraxa/state/state_db"
    dpos "github.com/Taraxa-project/taraxa-evm/taraxa/state/contracts/dpos/precompiled"
)

// quoteEdgeComponent executes each transaction on a distinct copy of one real
// committed B-2 source. It never calls the no-argument selector. Q and offsets
// define inputs; all status, gas, effects and roots come from real execution.
func quoteEdgeComponent(input eraInput, observer bool) map[string]any {
    const boundary uint64 = 7600000
    if input.Head != boundary-2 || input.FirstRetained != boundary-6 || input.Donor == nil {
        panic("quote-edge source coordinate or donor mismatch")
    }
    if observer && !observerAvailable() { panic("local observer unavailable") }
    base := input
    base.Name = "claim_all_legacy_direct_quote_edges_basis_donor"
    base.Periods = []uint64{input.Head}
    base.Actions = nil
    base.SeedActionsOnly = true
    base.NoWarmupDelegation = true
    base.ReopenAfter = nil
    source := new(eraSeedSource)
    basisDocument := eraRun(base, observer, source)
    if source.Latest == nil || source.Latest.pending != nil || uint64(source.Latest.descriptor.BlockNum) != boundary-2 {
        panic("quote-edge source is not actual finalized B-2")
    }
    cfg := eraConfig(input.Configuration)
    if uint64(cfg.Hardforks.FixClaimAllBlockNum) != boundary || uint64(cfg.Hardforks.MagnoliaHf.BlockNum) > boundary-1 || uint64(cfg.Hardforks.CornusHf.BlockNum) <= boundary-1 {
        panic("quote-edge era mismatch")
    }
    eraAPIConfiguration = cfg
    descriptor := source.Latest.descriptor
    originalPhysical := eraSeedDigest(source.Latest.rows.exportPhysical())
    originalLatest := eraSeedDigest(source.Latest.rows.exportLatest())
    originalIDs := eraSeedDigest(jsonValue(source.IDs))
    originalAddresses := eraSeedDigest(jsonValue(source.Addresses))
    sender := addressForPrivateKey(integrationKeys[4])
    dposAddress := *dpos.ContractAddress()
    reader := state_db.ExtendedReader{Reader:source.Latest.readerAt(descriptor.BlockNum)}
    // Actual stored iterable count/order, never supplied expected poststate.
    prefix := append([]byte{2, 1}, sender[:]...)
    countKey := crypto.Keccak256Hash(prefix, []byte{1})
    var count []byte
    reader.GetAccountStorage(&dposAddress, &countKey, func(raw []byte) { count = common.CopyBytes(raw) })
    if !bytes.Equal(count, []byte{2,0,0,0}) { panic("quote-edge caller does not have two stored members") }
    members := make([]string,0,2)
    for position:=uint32(1); position<=2; position++ {
        var index [4]byte; binary.LittleEndian.PutUint32(index[:],position)
        key := crypto.Keccak256Hash(prefix, []byte{2}, index[:])
        var item []byte
        reader.GetAccountStorage(&dposAddress, &key, func(raw []byte) { item = common.CopyBytes(raw) })
        if len(item)!=20 { panic("quote-edge stored member width") }
        members=append(members,hex.EncodeToString(item))
    }
    if members[0]==members[1] { panic("quote-edge repeated member") }
    basisCatalog := captureNativeCatalog(reader,source.IDs)
    basisAccounts := observeAddressSet(reader,source.Addresses)
    basisDigest := eraSeedDigest(map[string]any{"descriptor":descriptor,"physical":originalPhysical,"latest":originalLatest,"ids":jsonValue(source.IDs),"accounts":basisAccounts,"native_catalog":basisCatalog,"members":members})
    cases:=make([]map[string]any,0,6)
    for _,batch:=range []uint32{0,1} {
        q:=uint64(90000);if batch==1 {q=45000}
        for _,offset:=range []int64{-1,0,1} {
            func() {
                clone:=eraCopiedView(source.Latest,uint64(descriptor.BlockNum),descriptor.StateRoot)
                if clone.descriptor!=descriptor || eraSeedDigest(clone.rows.exportPhysical())!=originalPhysical || eraSeedDigest(clone.rows.exportLatest())!=originalLatest { panic("cold branch basis mismatch") }
                ids:=catalogIdentities{Accounts:append([]common.Address{},source.IDs.Accounts...),Slots:append([]catalogSlot{},source.IDs.Slots...),Invocations:source.IDs.Invocations}
                addresses:=append([]common.Address{},source.Addresses...)
                restore:=eraCatalogScope(&ids);defer restore()
                st:=newStateTransition(clone,&cfg);defer st.Close()
                p:=boundary-1;block:=eraBlock(p)
                beginRawWriteTrace();st.BeginBlock(&block.BlockInfo);begin:=finishRawWriteTrace()
                payload:=append([]byte{0x09,0xb7,0x2e,0x00},make([]byte,32)...);binary.BigEndian.PutUint32(payload[32:36],batch)
                intrinsic,err:=vm.IntrinsicGas(payload,false);must(err)
                desired:=uint64(int64(q)+offset)
                action:=eraAction{Period:p,Who:4,Name:fmt.Sprintf("legacy_quote_batch%d_%+d",batch,offset),Method:"raw",To:hex.EncodeToString(dposAddress[:]),Input:hex.EncodeToString(payload),Gas:intrinsic+desired}
                specs:=eraDeclaredTransactions([]eraAction{action},p);if len(specs)!=1 {panic("quote-edge action count")};spec:=specs[0]
                nonce:=new(big.Int);branchReader:=state_db.ExtendedReader{Reader:clone.readerAt(descriptor.BlockNum)}
                branchReader.GetAccount(&sender,func(a state_db.Account){nonce.Set(a.Nonce)})
                signed:=eraSigned(spec,nonce)
                tx:=vm.Transaction{From:sender,To:signed.Decoded.To,Nonce:signed.Decoded.Nonce,GasPrice:signed.Decoded.GasPrice,Gas:signed.Decoded.Gas,Value:signed.Decoded.Value,Input:common.CopyBytes(signed.Decoded.Input)}
                original:=tx;original.Input=common.CopyBytes(signed.Decoded.Input)
                beginRawWriteTrace();beginGasRefundTrace();result:=st.ExecuteTransaction(&tx);refund:=finishGasRefundTrace();writes:=finishRawWriteTrace()
                eraCreditCComposedTransactionFee(st,block.Author,tx,result)
                ids=eraWritten(ids,append(begin,writes...))
                var observation any;var observerRoot string
                if observer {root,catalog,_:=observerFinalizeTransaction(st,0);observerRoot=hex.EncodeToString(root[:]);observation=jsonValue(catalog.Invocations)}
                var transaction any
                if signed.Decoded.Nonce.Cmp(nonce)!=0 {panic("signed nonce differs from cold basis")}
                if result.ConsensusErr=="" {spec.ExpectedError=string(result.ExecutionErr);row:=fullTransactionRow(spec.fullTransactionSpec,signed,original,result,result.GasUsed);row["nonce"]=signed.Decoded.Nonce.String();transaction=row} else {
                    transaction=map[string]any{"consensus_error":string(result.ConsensusErr),"execution_error":string(result.ExecutionErr),"signed_rlp":hex.EncodeToString(signed.RLP),"hash":hex.EncodeToString(signed.Hash[:]),"gas_limit":signed.Decoded.Gas,"gas_used":result.GasUsed,"output":hex.EncodeToString(result.CodeRetval),"receipt":nil}
                }
                // This is an isolated transaction-only checkpoint, not a full
                // EndBlock/rewards period or a retained FinalChain publication.
                prepared:=st.PrepareCommit();committed:=st.Commit();if prepared!=committed {panic("branch checkpoint mismatch")}
                if observer && observerRoot!=hex.EncodeToString(committed[:]) {panic("observer/transaction checkpoint root mismatch")}
                after:=state_db.ExtendedReader{Reader:clone.readerAt(types.BlockNum(p))}
                fee:=new(big.Int).Mul(new(big.Int).SetUint64(result.GasUsed),tx.GasPrice)
                cases=append(cases,map[string]any{"batch":batch,"offset":offset,"requested_native_gas":desired,"quote_input":q,"intrinsic":intrinsic,"signed_gas":signed.Decoded.Gas,"basis_digest":basisDigest,"transaction":transaction,"sender":hex.EncodeToString(sender[:]),"nonce_before":nonce.String(),"transaction_logs":jsonValue(result.Logs),"refund":refund,"ordered_raw_writes":writes,"begin_block_raw_writes":begin,"fee":fee.String(),"accounts_after":observeAddressSet(after,addresses),"native_catalog_after":captureNativeCatalog(after,ids),"native_invocations":observation,"native_frame_visibility":map[bool]string{true:"local_observer_actual",false:"public_unobserved"}[observer],"transaction_checkpoint_root":hex.EncodeToString(committed[:])})
                if source.Latest.pending!=nil || source.Latest.descriptor!=descriptor || eraSeedDigest(source.Latest.rows.exportPhysical())!=originalPhysical || eraSeedDigest(source.Latest.rows.exportLatest())!=originalLatest || eraSeedDigest(jsonValue(source.IDs))!=originalIDs || eraSeedDigest(jsonValue(source.Addresses))!=originalAddresses {panic("cold branch changed the shared source")}
            }()
        }
    }
    return map[string]any{"schema":1,"component":"claim_all_legacy_direct_quote_edges","mode":map[bool]string{true:"local",false:"public"}[observer],"scope":"actual signed top-level transactions on six cold copies; transaction-only checkpoint; no reward or FinalChain publication","basis_document":basisDocument,"basis_period":boundary-2,"execution_period":boundary-1,"basis_digest":basisDigest,"basis_members":members,"basis_native_catalog":basisCatalog,"basis_accounts":basisAccounts,"source_physical_digest":originalPhysical,"source_latest_digest":originalLatest,"cases":cases}
}
