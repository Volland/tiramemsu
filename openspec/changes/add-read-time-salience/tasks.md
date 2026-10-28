## 1. Salience

- [ ] 1.1 `src/salience.rs`: `SaliencePolicy { key, half_life_ms, now }`, `Salience { score, count, last, age }`, one pure function.
- [ ] 1.2 `Tx::increment_volatile(s, key, by)` in both engines.
- [ ] 1.3 `View::salience(targets, &policy)` in the facade; `Func::Salience` in the IR, `tm_salience` UDF and SQL join for SPARQL `tm:salience`; native runner; Cypher `tiramemsu.salience`.

## 2. Surfaces

- [ ] 2.1 JSON bridge `incrementVolatile`, `salience` (read); Node and Python wrappers.

## 3. Tests

- [ ] 3.1 Salience: formula with a fixed `now`; absent counter; as-of view ignores volatile counts; SPARQL, Cypher and Rust give equal scores; nothing is written by a salience read.

## 4. Documentation

- [ ] 4.1 `lat.md/storage.md#Volatile Table` (counters), `lat.md/query.md` (functions); `tg check`; validate OpenSpec.
