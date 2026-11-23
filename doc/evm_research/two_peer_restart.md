# Two-peer EVM cutover qualification

Use the production daemon from the revision under review and the retained cutover
evidence directory. The runner needs Bubblewrap and a new output path outside the
evidence directory. It checks the hashes of the driver, fixture editor, and
transfer input before it copies either store.

```sh
python3 tests/rust/fixtures/run_cutover_peers.py "$DAEMON" "$CUTOVER_EVIDENCE" "$NEW_OUTPUT"
```

The peers must agree on the synced receipt before and after restart, serve
historical Debug and OpenEthereum RPCs, and finalize the successor period.
