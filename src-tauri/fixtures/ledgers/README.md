# Ledger fixtures

Frozen bytes that pin the ledger's line format across versions. Tests read
them as they are; nothing here is rewritten by a normal test run.

| file | what it is |
| --- | --- |
| `v1.jsonl` | a ledger in line format 1: what every build up to v0.98 wrote (no `schemaVersion` member) |
| `mixed-v1-v2.jsonl` | the same ledger after a build that writes format 2 appended to it |
| `v1.proofpack.json` | a signed packet of the v1 case |
| `mixed.proofpack.json` | a signed packet of the case spanning both formats |
| `make-v1.py` | writes `v1.jsonl` without calling the app |

What these are not: copies of a real install. The events are made up, and
`v1.jsonl` is produced by a second implementation of the format rather than by
an old binary. The line envelope (member names, order, hashing) is the same in
the v0.90, v0.94 and v0.98 sources, which is why there is one v1 file and not
one per release. Payload shapes are illustrative only.

The packets are signed with a throwaway key that exists only in the test
source; it signs nothing else.

Regenerate (only when a format is added, never to make a test pass):

    python3 make-v1.py > v1.jsonl
    cargo test --lib regenerate_ledger_fixtures -- --ignored

Used by `src/services/ledger_schema_tests.rs` and `src/dev/ledgerSchema.test.ts`.
