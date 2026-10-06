# Bounded source-current full removal and append

Base f705eed27; [actual oracle](n2_redelegate_current_source_oracle.md) is unchanged.
Fresh Astra placement and observation contracts bind this runtime extension.
The target follows real partial300 source31->existing33, then full700 source31
into absent32. Caller[31,33]->[33,32], aggregate1700/1000/2300 ->1000/1700/2300,
total5000, no rewards/accounts. Existing source currentnode count2 is loaded
before removal; cursor deletion reduces it to1 and Go writes the earlier copy2.
The repaired kernel restores that same original node before destination work.

Private FinalChain bounded_redelegate_loaded_source_node supplies both kernel
capture and staged guard. It returns the exact original NodeKey/Node or None for
structural/nonmatching scope; required graph/ledger errors propagate. No mutation
or raw snapshot rewriting occurs in the predicate. Original before remains reward,
pair/absence/cursor authority for both serializers; one shared raw trace retains
every intermediate expected value. The destination membership uses borrowed[33].
Existing serializers already emit source2->1->2 when semantic after is correct.
Old pre-fix same-validator capture/restoration and both-current-absent paths remain.

Eligibility: post-fix/Magnolia/Ficus active/preAspen2, full positive sourcecaller
principal, source retained/destination positive, exactly two positive callerpairs
and complete source-first distinct order. Both histories and principalledger are
complete; source/destination/retained legacy markers excluded. Graph current equals
invocation block; source non-stale head=cursor=current, currentcount2/zeroindex,
present zero source index/cursor mirrors. Destination non-stale headolder/current
nodeabsent, oldheadcount2/zeroindex, both callercursor views absent; destination
index absent or zero. Both affected pools zero. Existing is_stale_head becomes a
crate-visible production read-only query; incomplete history remains a hard error.
restore_loaded_node docs name the exact additional count2->1->2 use.

Four [tests](../../rust/crates/rustaxa-consensus/src/final_chain/native_session/custody/redelegate_current_source_tests.rs)
execute independent real semantic prefix/target, compare kernel/staged state to
actual facts, exact18ordered writes/log/output, all42frozen/finalpresencebytes,
old/currentgraphnodes/cursors, preservedother/third and committedhead/snapshot.
Coldtarget state derives from successful real staged prefix; livewarm keeps that
session and sequence. Every raw apply checks each intermediate expected value.
Rust targetcold/warm18reads, prefix14reads; Go target9/prefix14 are separatecounts.
All14prefix+18coldtarget+18warmtarget keys independently corrupt/error, giving100
hardfailures and100fresh retries. Successfulprefix state/sequence stays unchanged
on targetfailure, prepare clears/sessionpoisons, exactStateRead/RawIntegrity and
committedstate preserved. Last3targetkeys are moved33position/item1/item2 after
local source effects. Twenty-five predicate exclusions cover stale/different
heads/cursors, count/index/pools/mirrorpresence, destinationcurrent/cursor, caller
order/set/history/marker and block mismatch; general kernel rejection is not
claimed for nonmatches. Wrong-membership serializer failure separately drops
local node restoration and source effects, without ownerpublication.

Initial newcheck used a set map method; testhelpers assumed Default and incorrect
pool/node constructors. Corrected to existing typed APIs; first failurelogs remain.
Independent review found displaced original kernel documentation/Clippy allowance
on the new helper. Both moved back to the kernel; helper retains only capture
docs. Executable behavior is unchanged by this correction. All4targettests pass. Package/check/Clippy,43redelegation and1491consensus
regressions, EVM package, ONbridge build12/all15 and serialfast/whitespace pass.
Corrected Clippy/serialfast/whitespace also pass after the documentation fix.
Independent same-profile Sol medium accepted all6 corrected frozen files; no
functional finding remains. The original gates remain valid because the correction
changes documentation and the Clippy allowance only.
No actual frames/history for this currentnode profile, source-last/longerorders,
destinationcurrent/nonzero rewards/sourcevalidator removal/generalcurrentrepair,
productionrouting/fallback/C++/storage/protocol/broadgate/push is accepted.
N1–N6/Milestone10 remain open. Records prefix current-source-runtime- under
/home/fry/artifacts/evm-round-01a10f48-83ee-7a32-8bff-722d10c36359/; prior targeted
records current-source-authority-target etc are retained. Sol medium implements;
fresh same-profile independent Sol medium review route already confirmed.
