# Signature recovery compatibility

The pinned default Go recovery backend treats bit 4 of the recovery byte as compressed-key metadata. Recovery bytes 0 through 7 select the corresponding recovery coordinates after that bit is cleared. Bytes 8 and above remain invalid.

This rule does not add an activation height. Existing Magnolia checks and proof ordering remain in effect. Existing canonical histories retain their roots. A proof with changed signature bytes uses its own transaction and state fixtures. It does not authorize a state migration or database repair.

The pinned default Go backend also accepts valid high-S signature aliases. Recovery normalizes the local signature and toggles the recovery point parity. The original signed bytes remain unchanged. High-S fixtures retain their own transaction hashes, gas, receipts, and roots.

This note covers the pinned default Go backend. The separate tagged backend has a different qualification scope.
