# Bounded value1 claim-all nonpayable gas guards

Both pinned Go revisions execute literal H1 DryRunner/reader1 and independent Main2/reader1 over the same canonical positive92-row P1. Each of seven transaction caps is run twice with a fresh disposable owner. The committed complete source remains unchanged after every call. Reduced-observation controls produce identical execution and post-state.

Cap21271 fails intrinsic admission before quote or Run. All six admitted caps quote0 and run the active Cornus nonpayable guard, which returns `Method is not payable`. Transaction gas is21272 and native gas is0. Quote frame accounts show the ordinary value1 transfer (caller2999/native4801); final accounts show rollback (caller3000/native4800), while the envelope nonce remains2 for DryRunner or1 for Main. Native observations contain only four logical/backend bootstrap pairs, no caller-count access, writes, balance effects or logs. All23 raw keys and all three account states are checked.

Run `rtk proxy python3 experiments/evm_feasibility/native_reward_claim_all_nonpayable_reference.py --scratch /home/fry/artifacts/claim-all-nonpayable-producer-scratch --evidence <new-persistent-directory>` to reproduce. The manifest binds both revisions, exporter and support, reused copy instrumentation, guarded original copy targets, helper bytes, baseline and output hashes. `--record` is for an intentional reviewed fixture update.

The ordinary envelope transfer and native callback lanes are distinct. Qualified Rust omission of Go lazy bootstrap reads is not callback-fault timing parity. No general value support, publication, production routing, roots/windows or recovery acceptance follows.
