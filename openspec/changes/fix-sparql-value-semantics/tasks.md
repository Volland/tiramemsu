## 1. Exact decimals

- [ ] 1.1 Add `rust_decimal` to `tm-exec`; implement decimal UDFs (`tm_dec_add`, `tm_dec_sub`, `tm_dec_mul`, `tm_dec_div`, `tm_dec_neg`, `tm_dec_abs`, `tm_dec_round`, `tm_dec_ceil`, `tm_dec_floor`) and aggregate UDFs for `SUM`/`AVG` with promotion.
- [ ] 1.2 `VClass::Decimal` and XPath promotion in `crates/tm-exec/src/sqlgen/expr.rs`; native SQL stays for `Int`/`Double` pairs; `integer / integer` is decimal; overflow is an expression error.
- [ ] 1.3 `xsd:float` operands numeric (double precision, typed `xsd:float` results).
- [ ] 1.4 Register the UDFs in `tm-rusqlite` and `tm-wasm`; extend the WASM capability probe.

## 2. Language tags

- [ ] 2.1 Split the string class into plain and boxed; tag-preserving `UCASE`, `LCASE`, `SUBSTR`, `REPLACE`; new tag-aware `STRBEFORE`, `STRAFTER`, `CONCAT` UDFs following SPARQL 1.1 argument compatibility.

## 3. Zoned dates

- [ ] 3.1 Encode a date with a timezone as canonical `TYPED` `xsd:date` (`crates/tm-core/src/codec.rs`, `src/value.rs`); keep zone-less dates inline.
- [ ] 3.2 Date ordering key (starting instant, UTC when zone-less) in comparison and `ORDER BY` (`crates/tm-exec/src/udf.rs`).
- [ ] 3.3 Storage format bump with a no-op migration (`crates/tm-core/src/storage/migrate.rs`); test that an older format build refuses the migrated file and that every row is unchanged.

## 4. Negated property sets

- [ ] 4.1 `PathExpr::NegatedSet { forward, inverse }` in `crates/tm-ir/src/path.rs` with display and validation.
- [ ] 4.2 Lowering in `crates/tm-sparql/src/lower/path.rs`: one step as a variable-predicate pattern with `NOT IN` (union for mixed directions); recursive forms to the path IR; remove `NEGATED_PROPERTY_SET`.
- [ ] 4.3 Automaton transition and `NOT IN` fetch in `crates/tm-exec/src/path/{automaton,fetch,syntax}.rs`; virtual predicates never match.

## 5. Conformance and tests

- [ ] 5.1 Remove the 31 deviation lines listed in the proposal from `crates/tm-sparql/tests/w3c/expected-deviations.toml`.
- [ ] 5.2 Vendor `sparql11/property-path` from the upstream commit in `SOURCE.md`; add it to `CATEGORIES`; list any remaining failure with a precise reason.
- [ ] 5.3 Unit tests: `0.1 + 0.2 = 0.3` as `xsd:decimal`; `SUM` of ten `0.1`; `1/3` is a decimal; overflow is unbound in `BIND`; `UCASE("chat"@fr)` keeps `@fr`; `CONCAT` with mixed tags is plain; zoned date round trip and ordering against zone-less dates; `!p`, `!(p|q)`, `!^p`, `(!p)+` under now and `asOf`, and inside `GRAPH`.
- [ ] 5.4 Re-run the SPARQL benchmark with integer-only and decimal-only arithmetic shapes; record the difference.

## 6. Documentation

- [ ] 6.1 `lat.md/query.md#Front Ends#SPARQL` (limits and deviations), `lat.md/data-model.md#ObjectId#Canonical Encoding`, `lat.md/storage.md` format history, CHANGELOG; `lat check`; validate OpenSpec.
