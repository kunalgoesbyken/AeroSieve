# aerosieve-sink

Zero-copy file sink with atomic commit semantics.

- Writes audio and text to a **staging directory**, then **hard-links** (or renames) into a clean directory
- `Null` mode for benchmarking (no disk I/O)
- UUID-based naming for collision-free storage
- Staging cleanup on demand

---

**Status: superseded by `aerosieve-shard` for the v2 pipeline.** Shard commits
happen at tar-shard granularity rather than per sample, so `aerosieve-shard`
implements the same staging -> hard_link -> rename pattern over shards. This
crate remains as the per-file implementation, still tested.

Note one v1 behaviour NOT carried forward: this crate reports
`committed: false` on the successful `rename` fallback path, which reads as a
failure. `aerosieve-shard` treats both commit mechanisms as success.
