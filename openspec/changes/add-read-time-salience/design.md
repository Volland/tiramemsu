## Context

Split from `add-derived-beliefs` on 2026-10-28 so that change could land on its own as the prerequisite of `add-n3-reasoning`. The decisions are unchanged.

Volatile values are upserted per `(s, key)` with `updated_at` = the transaction instant and are visible only under the now view.

## Decisions

### D1. Salience is a pure function of counter, recency and policy

`salience(target) = (1 + ln(1 + count)) · 2^(−age / half_life)`, where `count` is the integer volatile value of `(target, policy.key)` (0 when absent, non-integer, or the view is not now), `age = max(0, now − last)`, and `last` is the latest of the counter's `updated_at` and the newest commit instant of the target's live statements in the view (for a statement target, its own `tm:addedAt`). `now` defaults to the view's instant (now, or the as-of instant) and can be fixed in the policy for reproducibility. The score is computed by one Rust function used by `View::salience`, the `tm_salience` UDF and the Cypher function, and every result can be explained by returning its inputs (`count`, `last`, `age`).

*Alternatives:* a learned or configurable formula (not explainable, not deterministic across versions); storing scores (writes on read, and history of scores no one asked for).

### D2. `increment_volatile` for access counting

Read-modify-write through `set_volatile` races between transactions only in the sense of lost updates inside one writer, which cannot happen with a single writer, but it costs two calls; `Tx::increment_volatile(s, key, by)` upserts `value = value + by` in one statement and sets `updated_at`. Counters stay outside the graph, so access tracking never creates history.

## Risks / Trade-offs

- [Salience depends on the clock] → `now` can be fixed in the policy; tests fix it.

## Open Questions

- Should salience also count `sys:confirmedBy` confirmations as accesses?
