# Defect and interruption recovery

A platform block is not evidence that a vulnerability exists. Record the observed
error separately from any source-level finding. Do not bypass a block, change
models to evade it or promise that instructions can prevent a forced interruption.

For a concrete suspected defect in authorized project scope:

1. Save a compact record with affected paths, broken invariant, evidence, status
   and next action. Use suspected/confirmed/fixed/verified accurately. Keep technical
   details in local evidence; user updates can state status and impact briefly.
2. Establish a bounded local regression test for the invariant. Demonstrate failure
   before the fix and success after it when feasible. Use synthetic fixtures and
   local components; exploitation or external-system testing is not required.
3. Fix the cause through the project's required implementation and ownership paths.
   Preserve valid behavior and adjacent rejection/rollback boundaries. Run required
   focused checks and independent review before calling the defect verified.
4. Retain the regression test in the normal relevant suite. Link finding, fix,
   test and accepted commit so later slices cannot silently drop the protection.

Continue permitted independent work while a finding is blocked. Do not accept
dependent behavior whose required invariant remains broken. Missing authority
does not expand the task; request only the concrete exception needed.

Maintain checkpoints at acceptance boundaries and before long gates. If a turn is
forcibly interrupted, the next authorized turn inspects the last checkpoint, live
delta and retained outputs before continuing permitted work. Do not infer that
in-flight commands completed. Preserve the saved budget on same-session resume;
fresh-session budget rules still apply. Stop when quota or authorization requires.

If validation evidence is lost or a capture path collides, inspect runner status
before retrying. A child can import modules before any test runs. Preserve the
original streams or loss record and all process charges; a later captured run is
a separate occurrence. Where the project limits attempts or resources, obtain
prospective admission for that occurrence. Do not turn recovery into retroactive
acceptance of the lost run. For serial work, one command owner reduces duplicate
launches; delegate bounded diagnosis separately from test execution.
