## Why

Tiramemsu is one SQLite file per agent. Agents will want to merge memories: two files that learned different things, or one file that was copied and diverged. The data model is already close to mergeable. Statement content never changes, eids are never reused, and a retraction only sets `t_ret` once, from NULL. Statements under those rules form a grow-only set with a once-only tombstone, which is a 2P-set CRDT, so a merge is a union of statements plus the earliest retraction of each.

The obstacle is identity. `STMT`, `NODE`, `BNODE` and `TX` payloads are per-file counters, so eid 5 in file A and eid 5 in file B are different statements. Any skolem IRI (`urn:tiramemsu:stmt:5`) that an agent stored, cited or sent elsewhere names a different statement in every file.

The options differ in what they cost later. Reserving bits in the payload now costs one bounds check. Reserving them after files exist in the wild costs a migration that rewrites every eid, which [[time-model#Never Forget]] and the invariant triggers (`eids are never reused`, no rewrite of `eid`) forbid. This change only reserves; it does not specify merging.

**Status: decided (Option A, reserve origin bits) and implemented.** See design.md, Decision, for the options that were weighed.

## What Changes

- The 60-bit payload of `STMT`, `NODE`, `BNODE` and `TX` ids is split into a 12-bit **origin** (high bits) and a 48-bit **counter**. Format 1 files always write origin 0, so every id allocated today is unchanged: same value, same varint size.
- The `NODE`, `BNODE`, `STMT` and `TX` counters are bounded: the largest number ever allocated is 2⁴⁸ − 1 (about 2.8 × 10¹⁴ allocations per kind), so `next_*` stops at 2⁴⁸. Allocating past the bound fails with a typed error instead of spilling into the origin bits.
- Decoding, skolem IRIs and the codec are unchanged for origin 0. A payload with a non-zero origin is rejected by format 1 on input (skolem IRI parse, `Value::Stmt`, bundle import), as tag 15 `SEALED` is today.
- Nothing about merging is specified here. A later change (`add-memory-merge`) would define the origin registry (`meta.origin`), how foreign statements keep their ids, conflict rules for `sys:one` and `sys:unique` (they become read-time policies across origins), and the skolem form `urn:tiramemsu:stmt:<origin>.<n>`.

## Capabilities

### New Capabilities
<!-- none -->

### Modified Capabilities
- `object-encoding`: the ObjectId layout reserves the high 12 payload bits of `STMT`, `NODE`, `BNODE` and `TX` as the origin, which format 1 fixes to 0.
- `storage-format`: the engine counters allocate at most 2⁴⁸ − 1 per kind.

## Impact

- **`tm-core`:** `id.rs` (payload accessors, origin mask), the counter allocators in `engine/mod.rs`, the skolem parser in `mapping.rs`, and one new error variant `IdSpaceExhausted { kind }`. Around 50 lines of code.
- **Format:** no DDL change and no version bump. Existing files are valid as they are, because every stored id has origin 0.
- **Not affected:** query engines, front ends, bindings and storage size.
- **Alternative with no reservation:** merge by translation. Imports mint fresh local ids and record `(e sys:origin "<file-uuid>:<n>")` as a layer. That is what `add-fact-bundles` already does for one fact. It needs no format decision, but the stable global name becomes a layer lookup instead of the id itself. See design.md.
