# SPARQL value semantics: decimals, language tags, zoned dates, negated property sets

Target release: **0.7 — Time in every language**. Closes review finding M5 (`docs/project-review.md`); source: `docs/feature-roadmap.md#0.7` ("SPARQL value fixes").

## Why

147 of 781 in-scope W3C tests are listed deviations in `crates/tm-sparql/tests/w3c/expected-deviations.toml`. Most are by design (relative IRIs, union default graph, every reifier is an eid). Four groups are not design choices but wrong-type or lossy results that users hit silently, which is worse than a loud error:

- **Decimal arithmetic returns `xsd:double`.** `0.1 + 0.2` is not `0.3`, `SUM` of decimals drifts, and `1 / 3` on integers is a double where SPARQL says decimal. SQL codegen treats decimals as `VClass::Double` and computes in SQLite `REAL` (`crates/tm-exec/src/sqlgen/expr.rs`, `class_of`, `native`, `PExpr::Arith`).
- **String functions drop language tags.** `UCASE("chat"@fr)` returns a plain `"CHAT"`, so a query that filters by `LANG` after a string function loses rows.
- **`xsd:date` drops its timezone.** The canonical encoding stores `"2006-08-23Z"^^xsd:date` as the same `DATE` id as `"2006-08-23"^^xsd:date` (`object-encoding`, "Canonical encoding of booleans, dates and date-times"), so the zone is lost on write.
- **Negated property sets are `Unsupported`.** `!p`, `!(p|q)` and `!^p` fail in `crates/tm-sparql/src/lower/path.rs` (`NEGATED_PROPERTY_SET`), although the path automaton can express "any predicate except".

## What Changes

- **Exact decimals.** Decimal values and decimal-producing arithmetic and aggregates are computed exactly in a 128-bit decimal (28 significant digits) and returned as `xsd:decimal`, with XPath numeric type promotion (integer → decimal → float → double); `integer / integer` returns `xsd:decimal`; `xsd:float` operands become numeric (computed in double, typed `xsd:float`). Overflow is an expression error. SQL codegen gains a decimal value class and emits decimal UDFs (`tm_dec_add`, `tm_dec_sub`, `tm_dec_mul`, `tm_dec_div`, `tm_dec_sum`, `tm_dec_avg`, `tm_dec_round` …) only where an operand may be a decimal; integer and double arithmetic stays native SQL.
- **Language-tag-preserving string functions.** `UCASE`, `LCASE`, `SUBSTR`, `STRBEFORE`, `STRAFTER`, `REPLACE` and `CONCAT` return a language-tagged literal when SPARQL 1.1 says so (same tag on all `CONCAT` arguments; `STRBEFORE`/`STRAFTER` with compatible arguments), as boxed literal values that survive projection, `FILTER`, `LANG` and result encoding.
- **Zoned `xsd:date`.** A date with a timezone is stored as a `TYPED` `xsd:date` term in canonical form (`Z` for a zero offset), keeping its zone; a date without a timezone stays the inline `DATE`. Comparison and ordering use the date's starting instant, with a date without a timezone compared as UTC, the rule date-times already follow. This changes the canonical encoding, so it ships with a storage format bump and no rewrite of existing rows.
- **Negated property sets.** `!p`, `!(p1|…|pn)`, `!^p` and mixed `!(p|^q)` lower to a variable-predicate pattern with a `NOT IN` filter when they are one step, and to a new "any predicate except" transition of the path automaton inside `*`, `+` and `?`. A negated set matches exactly the stored statements a variable-predicate pattern would match with those predicates filtered out.
- **Conformance:** the W3C `sparql11/property-path` category is vendored and brought into scope; the deviation lists shrink.

### Deviations removed from `expected-deviations.toml`

- **Decimal (17):** `aggregates/manifest#agg-sum-01`, `#agg-sum-02`, `#agg-avg-01`, `#agg-avg-02`, `#agg-avg-distinct`, `#agg-sum-distinct`; `functions/manifest#ceil01`, `#floor01`, `#round01`, `#plus-1-corrected`, `#seconds`; `data-r2/expr-ops/manifest#add-numbers-cast`, `#subtract-numbers-cast`, `#multiply-numbers-cast`, `#divide-numbers-cast`, `#unplus-2`, `#unminus-2`.
- **Language tags (10):** `functions/manifest#concat02`, `#substring01`, `#substring02`, `#ucase01`, `#lcase01`, `#strbefore01a`, `#strbefore02`, `#strafter01a`, `#strafter02`, `#replace01`.
- **Zoned dates (4):** `data-r2/open-world/manifest#date-1`, `#date-2`, `#date-3`, `#date-4`.

31 lines in total. Not removed: `json-res#jsonres01`/`#jsonres02` (canonical decimal lexical form `5.0`, a canonicalisation choice) and the canonical-literal groups, which stay by design. Negated property sets have no current deviation because the property-path category was out of scope; the newly vendored category starts with only the deviations that remain by design (if any), each with a reason.

### Compatibility

- Results change type: decimal arithmetic and aggregates return `xsd:decimal` instead of `xsd:double`; `integer / integer` returns `xsd:decimal`; string functions on language-tagged input return tagged literals. Queries that compared such results with `xsd:double` constants still compare by value; code that read the datatype of a result changes. Called out in the CHANGELOG.
- Zoned dates written before the format bump stay as stored (zone already dropped); only new writes keep the zone. A pattern constant `"…Z"^^xsd:date` matches only rows written after the bump; `FILTER` comparisons match by value across both.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `sparql-query`: ADDED requirements for exact decimal arithmetic and numeric promotion, language-tag-preserving string functions, and zoned dates in comparisons.
- `object-encoding`: "Canonical encoding of booleans, dates and date-times" keeps a date's timezone as a `TYPED` term from the new storage format.
- `path-lowering`: "Unsupported SPARQL path forms" is removed and replaced by an ADDED requirement for negated property sets.

## Impact

- **`crates/tm-core`:** `src/value.rs` (zoned date canonical form and `Value` handling), `src/codec.rs` (encode a zoned date as `TYPED`), `src/storage/migrate.rs` (format bump step that changes no row), `src/storage/meta.rs` (format version).
- **`crates/tm-exec`:** `src/sqlgen/expr.rs` (`VClass::Decimal`, promotion, decimal UDF calls, boxed language-tagged results), `src/udf.rs` and `src/udf_fn.rs` (decimal UDFs, tag-preserving `tm_ucase`/`tm_lcase`/`tm_substr`/`tm_replace`, new `tm_strbefore`/`tm_strafter`/`tm_concat`, date ordering keys), `src/decode.rs`, `src/path/` (`automaton.rs`, `fetch.rs`, `syntax.rs`: negated transitions).
- **`crates/tm-ir`:** `src/path.rs` (`PathExpr::NegatedSet { forward, inverse }`), `src/display.rs`, `src/validate.rs`.
- **`crates/tm-sparql`:** `src/lower/path.rs` (lower negated sets), `src/lower/expr.rs`, `src/lower/agg.rs` (decimal aggregates), `src/error.rs` (`NEGATED_PROPERTY_SET` removed).
- **Hosts:** `crates/tm-rusqlite/src/register.rs` and `crates/tm-wasm` register the new UDFs; the WASM probe lists them.
- **Dependencies:** `rust_decimal` in `tm-exec` (no default features beyond `std`).
- **Tests:** `crates/tm-sparql/tests/w3c/expected-deviations.toml` (31 removals), `crates/tm-sparql/tests/w3c.rs` (`sparql11/property-path` in `CATEGORIES`), vendored `tests/w3c/data/sparql11/property-path/` from the same upstream commit; unit tests in `tm-exec`.
- **Docs:** `lat.md/query.md#Front Ends#SPARQL` (Known limits, Property paths, Known deviations), `lat.md/data-model.md#ObjectId#Canonical Encoding`, `lat.md/storage.md` (format history), CHANGELOG.

## Non-goals

- No arbitrary-precision decimals; 28 significant digits is the documented limit.
- No change to canonical literal collapsing of numbers (`01` = `1`), relative IRI rejection, or the union default graph.
- No Cypher change (Cypher Float semantics for decimals stay).
