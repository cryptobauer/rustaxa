# Owner scope update: pruning deferred

Owner instruction: defer pruning implementation,including the Taraxa lite-node
pruning approach. This is a current scope overlay on the broader E1-E12/S0-S8
roadmap;historical accepted evidence and limits remain unchanged.

Pruning implementation and pruning-specific campaigns do not block the agreed
non-pruning backend closeout. Do not add pruning work to the current branch-run.
Existing tests are not deleted,weakened or skipped merely because pruning is deferred.

Still required: strict missing/unavailable-history distinctions,logical membership
and state identity,recovery and retention for the declared supported state scope,
compatible reopen/import,interrupted publication/import,storage-fault and power-loss
validation,reference rollback/catch-up as applicable,and API errors for unavailable
history. Never substitute missing/pruned state with zero or an invented absent value.
The role72/75history-unavailable report shapes remain acceptance inputs.

E10keeps fault/import acceptance;pruning-specific portions are deferred. E5/E8keep
correct unavailable-history behavior. E9/E11keep applicable persistence/import/
recovery/retention guarantees. Production routing and performance acceptance remain
separate. This deferral is not evidence that those remaining requirements passed.
