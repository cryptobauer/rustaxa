# N4 next real-window dependency contract

Status: reviewed input contract. This does not establish N4 execution acceptance.

The independent review accepted the corrected artifact at SHA-256
`783ee5b357b19485cf895d0d561ed36163a1a2e41437cbea2b772227da5024d6`.
The [review record](/home/fry/artifacts/evm-run-01a12614-b9fe-7293-a303-2bdefab52645-20261010/n4-next-contract-review.md) retains the first finding and report hash.
Lead runtime evidence confirms Astra medium for the source contract and Sol medium
for independent review. These facts do not imply billing or execution acceptance.

Base supplied by lead: `f80ed246985fd476c371b605ad96ad1813a21d4f`, with a lead-owned pending P0 prefix delta. Source ranges below refer to the working tree inspected on 2026-10-10. No repository files were changed. Requested model/reasoning: Astra medium; the agent ran, but runtime confirmation is owned by the lead. No routing failure was observed by this agent. Lead allowance decision: baseline 58%, floor 48%, current 57%, observed 14:01:10 UTC. This task used that bounded assignment; no new work was started outside it.

## Result

There is no ready existing command that can execute the real H=25,706,949 reward/system transition through the current bound native-session owner from the inspected immutable evidence alone. The next dependency is a qualified complete native-state and policy input, plus a historical execution owner that can bind parent P=25,706,948. Repeating transaction preflight, sparse vote probes, or the empty-effects diagnostic will not supply it.

Two source constraints must be explicit in the next handoff:

1. `begin_native_session_bound` requires the stored application head to equal P. The retained copy is at H. Selecting a historical concrete reader does not change or satisfy that owner check. Do not rewrite `LAST_NUMBER` on the copy to bypass it.
2. With candidate delay five, this session loads eligibility at H−5=25,706,944. The certificate-weight diagnostic used Q=25,706,947 and D=Q−5=25,706,942. Its 59 authenticated point facts cannot supply the session's complete 25,706,944 snapshot, or the complete P snapshot.

The pending P0 trace work is a separate fixture-bound simulation origin. It neither returns a rewards-capable owner nor removes these H/P constraints.

## Evidence that can be reused

| Evidence | Qualified fact | Limit |
| --- | --- | --- |
| `doc/evm_research/n4_replay_preflight.md` | The 19 ordered chain-841 transfers use 20 authenticated prior accounts, no demanded code/slot reads, exact five-field receipts and cumulative gas 399,000. Transaction-only trie preparation derives retained root `b12e770de99e3ca011ea30d63b1321d4d7a7ca7dfa4ba189fc4e8ca4c73d0227`. | No reward EndBlock, complete native snapshot, adoption or publication. Root equality alone does not prove the omitted transition inert. |
| `n4_independent_reward_inputs.md` | At D=25,706,942/root `dc4c6771a9ed227db8179b68783eefc78bb0f6a443c573d71457c8d516f8d041`, 59 distinct account/key pairs have raw reads and authenticated proofs. Total votes 534,879; 19 independently verified weights sum to 714. | Strict VRF diagnostic policy is not verified producer policy. D is not session eligibility H−5. NonMember, unavailable physical history, and zero are separate facts. |
| Same record | Repository-decoded lambda 1,500 ms and candidate consensus delay 400 ms derive annualized rate 9,275,294. A retained 13-field DPoS configuration row corroborates threshold, vote step, maximum stake and delay five. | The configuration metadata is not root-authenticated. Original selected lambda key/raw bytes, producer settings and dynamic runtime provenance are not established by this report. |
| `n4_independent_reward_plan.md` | Under the candidate settings, independent inputs reproduce all typed reward statistics. The plan caches H, does not clear cache, and emits empty distribution RLP `c0`. | Raw validator-map order differs. Historical reward-cache closure and producer policy remain open. This is not an owner-issued request/generation-bound plan or an EndBlock result. |
| `n4_empty_native_effects.md` | At P/root `926d41bdd76e2815dff741a33d66142546c57ae6a041e5d8d8cd5445aaf712e2`, 0xfe account exists with code size 3,000; 0xee logical key leftpad32(02) is physical/member `c0`. | Three parent calls prove these facts only. The candidate non-pillar branch H+5=25,706,954, remainder 2,954 mod 4,000, needs qualified schedule policy. Bridge fields were unread placeholders. |
| `n4_rust_native_transition_blocked.md` | Four rejecting raw/account helper tests exist. The pinned Go cold fixture recorded zero mutation attempts for its supplied inputs. | No Rust lifecycle ran. The Go fixture is unauthenticated and omits Rust's unconditional origin reads. It is not a complete-state or real-root witness. |

The cited records contain original hashes and historical paths. This task did not reopen their input databases or rerun their diagnostics. It did not authenticate additional facts.

## Current owner/API contract

Paths in this table are relative to `rust/crates/rustaxa-consensus/src/`.

| Source range | Required authority or behavior |
| --- | --- |
| `final_chain.rs:1334–1391` | `FinalChain` owns `Arc<Storage>`, complete semantic snapshots, policy, reward runtime and scheduler. A sparse read adapter does not construct it. |
| `final_chain.rs:1401–1475` | `DposSnapshot` contains stakes, reward pools, metadata/VRFs, votes, delegation principal/order/cursors, reward indices, Aspen supply state, V1/V2 undelegations and IDs, slashing state/proofs, reward graph and history-completeness markers. Preserve complete maps and their authority; no missing entry may become an invented zero. |
| `final_chain/native_session.rs:379–396, 415–502` | Bound constructors retain an exact request; the StateAPI-epoch constructor also obtains an owner-issued scheduler basis. `FinalChainNativeSimulation` has no terminal rewards surface. |
| `final_chain/native_session.rs:682–755` | Construction checks current finalized parent, successor period and post-Cornus policy; loads P; advances the reward graph; loads complete eligibility at pending−delay; derives initial total votes and total stake. |
| `final_chain.rs:5054–5063, 6333–6348` | Reward graph advances through the existing graph owner. Snapshot lookup clones a present owned snapshot and fails when absent; it does not reconstruct from point reads. |
| `final_chain/native_session/concrete_checkpoint.rs:94–155` | Reader selection binds one accepted period/root. It expressly does not assert catalog completeness. Reads propagate concrete failures. It does not grant complete semantic state, plan or publication authority. |
| `final_chain.rs:10104–10166, 10171–10196` | `plan_external_evm_rewards_stats` requires fact.period=current head+1, derives eligibility total through FinalChain, clones runtime at its durable head/generation and issues an opaque plan. Validation rechecks head, period and generation. JSON `c0` is not this plan. |
| `final_chain.rs:10825–10888` | Reward runtime reconstruction reads a stable stored head plus persisted reward-stat rows and applies frequency rules. The historical cache at P is required; an arbitrary empty cache is not equivalent. Startup cleanup can mutate stale boundary rows when enabled. |
| `final_chain/native_session/rewards.rs:64–196` | `finish_rewards` checks lifecycle/request/plan, excludes unsupported boundary cases, verifies committed slashing, validates origins, reconstructs distributions, compares minted/supply/full semantic state, then returns unpublished effects. Failure exposes no successful result. |
| Same file, `555–598` | Every terminal call reads 0xfe raw logical keys 04 and 05 (through `concrete_storage_key`), and compares their bytes to period-start total vote count and total delegated amount. These reads occur even with `c0` distributions. |
| Same file, `609–677, 680–699` | Slashing cleanup uses owner-issued scheduler state or narrower standalone guards. Empty semantic jail list returns without a raw cleanup write. Complete committed parent slashing state must match staged state; an authenticated empty list alone is not that complete snapshot. |
| `final_chain_execution.rs:237–247, 1088–1128` | System planner takes period/request, pillar decision, bridge facts, epoch decision, nonce and gas. Qualified non-pillar policy selects zero transactions without needing active-branch bridge inputs. The planner does not derive or authenticate that policy. |
| `experiments/evm_feasibility/snapshot_qualifier/src/bin/empty_native_effects.rs:211–249` (repo-relative) | Existing conditional system witness pins candidate Ficus/Aspen/Cornus/fix heights and interval 4,000; its bridge values are dead-branch placeholders, not proven absence. |

## Smallest missing facts for this exact real window

These are minimum categories imposed by the current owner, not permission to enumerate all state.

1. **Complete semantic source at P and session eligibility H−5.** Supply exact root/period-bound native catalogs and all values needed to reconstruct the `DposSnapshot` units above, including reward graph/history authority. Preserve ordering and complete membership/absence evidence. Sparse inverse-key coverage is insufficient. If policy changes delay, recompute the eligibility identity before sourcing state. The certificate total at D remains a separate planner dependency. The current owner-issued planner overwrites the recovered total through `dpos_eligible_total_vote_count(Q)`, which loads a complete owned D snapshot (`final_chain.rs:10118–10120, 2468–2473, 6282–6308`). It therefore also requires owner-authoritative complete D=25,706,942 semantics, or a separately reviewed narrow planner boundary that retains authenticated input authority. The 59 point facts cannot construct that owned snapshot.
2. **Parent deferred origins.** Authenticate exact 0xfe logical keys 04 and 05 at P, preserving physical bytes and membership classification. Reconcile them with the complete snapshot's calculated totals. The D total 534,879 is not automatically P's total. Two extra reads alone would still not solve complete-state or ownership gaps.
3. **Effective policy for H and its dependencies.** Qualify the execution/native hardfork branches, reward frequency/committee/yield and supply rules, delegation delay, relevant slashing rules, Ficus pillar schedule and any exceptional transition height. Preserve candidate labels until provenance is established. The likely producer revision is owner-reported; binary/capture identity remains unknown. The existing mainnet config and retained DPoS row do not upgrade it.
4. **Reward runtime at P and exact period fact.** Bind P's cache rows and rules to the independently recovered H transaction/DAG/certificate/rate facts. The owner must issue the request/head/generation-bound plan. The existing typed offline equality can serve as comparison evidence, not as a fabricated plan token.
5. **Historical owner execution boundary.** Supply an authorized isolated, coherent P owner state, or separately design/review an unpublished historical owner that retains all state/policy/plan checks without altering the H copy. A `ConcreteCheckpointNativeStateRead` at P is only the raw/account read port. Production publication additionally requires its real StateAPI epoch/scheduler and pairing authority; that is a separate N5/adoption dependency.

For this non-pillar, empty-distribution, empty-jail branch, do not request irrelevant bridge nonce/code/epoch rows or per-distribution validator reward writes merely to fill placeholders. Conversely, do not bypass the existing constructor's complete-state needs because those branches happen to be inert.

## Next action and exact restart condition

No database command, replay command or new code step is ready to close this real-window dependency from the present immutable inputs. All executed work for this assignment was source/document inspection and this report. No tests, gates, probes, database opens, inventory scans, producer requests or delegation were performed.

Restart real transition implementation only after a handoff identifies: (a) the qualified policy and exact P/session-eligibility/certificate-D roots; (b) complete semantic reconstruction authority for P, H−5 and D (or a separately reviewed narrow planner boundary), and P origin bytes; (c) P reward-cache/runtime input; and (d) the reviewed owner construction method that does not change the retained H input. Then keep the first execution to one request, the same 19 transfers, zero system transactions only if the qualified branch proves this, one `finish_rewards`, and unpublished output comparison. Stop on any unexpected read, unsupported branch, unknown history or identity mismatch. Compare receipts, total reward, ordered/semantic effects as appropriate, and an independently calculated final root; never inject the expected root.

A complete synthetic cold fixture with an isolated database could test the lifecycle and raw-origin difference, but it would not obtain any missing historical facts or close N4. A new database-free terminal owner would be an architectural change requiring a separate authority contract, not an existing executable shortcut. No broad replay, new inventory or fault campaign is proposed: without the missing data and owner design, exact safe bounds and a useful acceptance claim cannot yet be set.

M10 N4 still requires at least one contiguous real window with exact receipts, period outputs and roots, qualified prior-state dependencies and reproducible identities, through the existing ownership/adoption path. This report grants no real-window acceptance. Its independent review confirms the input contract; dependent implementation must still meet the restart conditions above.

Independent review correction: explicitly include the planner owner’s complete D snapshot requirement. The first review and original report hash remain in n4-next-contract-review.md. This correction does not authenticate new inputs.
