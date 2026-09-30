## Context

The ObjectId is `(payload << 4) | tag` with a 60-bit payload ([[data-model#ObjectId]]). Ids are allocated from per-kind counters in `meta` ([[storage#Triple Table]]), and they stay small so that SQLite stores them as 1–3 byte varints ([[storage#Measured Footprint]]). Imports already exist in one form: `add-fact-bundles` moves one fact plus its layers between files by minting new local ids.

## Goals / Non-Goals

**Goals:**
- Keep a merge of two agents' files possible later without rewriting stored ids.
- Cost nothing today: no size change, no format bump, no behaviour change for single-file use.

**Non-Goals:**
- Specifying merge, sync or replication. That is a later change, and it depends on this decision.
- Global uniqueness of term ids. Terms are merged by value (IRIs and literals are their own names), so dictionary ids stay local in every option.

## Decision

This needs the owner's decision. Two options:

### Option A: reserve origin bits (recommended)

The high 12 bits of the 60-bit payload of `STMT`, `NODE`, `BNODE` and `TX` hold an origin; the low 48 bits hold the counter.

- **Size:** origin 0 ids are identical to today's, so a single-file store pays nothing. Only foreign ids (origin ≠ 0) are large: `(1 << 48) << 4` needs an 8-byte varint, against 1–3 bytes for local ids. A merged store pays roughly +5 bytes per foreign id occurrence across the row and its nine index entries.
- **Order:** local ids stay dense and ordered. Foreign ids sort after every local id of the same tag. Nothing in the engine relies on cross-origin order.
- **Identity:** a statement keeps one id forever, in every file that holds it. A skolem IRI cited by an agent keeps working after a merge.
- **Bound:** 4 096 origins and 2.8 × 10¹⁴ allocations per kind per origin. The allocation bound is far beyond the design scale (D1: 10⁴–10⁷ statements).
- **Reversible:** if merging is never built, the reservation is a bound nobody reaches. Dropping it later is free, while adding it later requires a rewrite that the invariant triggers forbid.

### Option B: merge by translation (no reservation)

Imported statements get fresh local ids, and the foreign identity is kept as a layer `(e sys:origin "<file-uuid>:<eid>")`.

- **Size:** local ids stay small, and each foreign statement costs one extra statement (the origin layer, about 150 bytes).
- **Identity:** a cited skolem IRI from another file resolves through a lookup on `sys:origin`, not directly.
- **Cycles:** layer cycles are allowed ([[data-model#Layers]]), so translation needs pre-allocated ids, as supersede's replay does ([[time-model#Operations#Supersede]]).
- **No format decision:** it can be decided at any time.

### Recommendation

Option A. It is cheap now and impossible to add later, and the origin bits are what make "one fact, one id, in every agent's file" true. Option B stays available for one-off imports (bundles) under either choice.

## Risks / Trade-offs

- [Foreign ids are 8-byte varints] → Only merged stores pay, and only for foreign rows. Measure with the `bench/` size benchmark when merge is built.
- [4 096 origins may be too few for large fleets] → A fleet can reuse origins for files that never merge with each other. Keep 12 bits unless someone has a concrete fleet size.
- [A reserved bound can be hit by a runaway allocator] → `IdSpaceExhausted` fails the transaction instead of corrupting identity. 2⁴⁸ allocations would take about 90 years at 10⁵ per second.

## Open Questions

- Should `TX` carry an origin? Merged transactions from another file have their own `t` and `instant`, so a single gap-free `t` sequence per file conflicts with importing foreign transactions. The alternative is to keep foreign tx metadata as statements about a foreign `TX` id.
- What should the skolem IRI form be for origin ≠ 0: `urn:tiramemsu:stmt:<origin>.<n>`, or a file UUID instead of the origin number?
