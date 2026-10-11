## Context

SPARQL expressions compile to SQL (`crates/tm-exec/src/sqlgen/expr.rs`). Each computed value has a static class (`VClass::Int`, `Double`, `Bool`, `Iri`, `Str`, `Dynamic`); stored terms are ObjectIds decoded by UDFs (`tm_num`, `tm_lit`, `tm_lit_lex`, `tm_lit_lang`, …). Decimals are dictionary terms with a canonical `lex` and a `REAL` `num` (`object-encoding`); arithmetic reads `num`, so it is a double computation. Computed strings are plain `TEXT`, so a language tag cannot survive a function. Literals that must carry a datatype or tag through SQL already exist as "boxed" text (`lex` plus a separator and the datatype or `@tag`, `crates/tm-exec/src/udf_fn.rs#unbox`).

`xsd:date` encodes as inline `DATE` with signed days (`crates/tm-core/src/codec.rs`), and a timezone suffix is dropped. Tags are exhausted (tag 15 is reserved for `SEALED`), and the existing `DATE` payload cannot be reinterpreted without rewriting rows, which never-forget forbids.

Property paths lower to `tm_ir::PathExpr` and run on the automaton path engine (`crates/tm-exec/src/path/`) for recursive forms; `!p` is rejected in `crates/tm-sparql/src/lower/path.rs`. The W3C runner's `CATEGORIES` exclude `property-path` ("until M3"), and the category is not vendored.

## Goals / Non-Goals

**Goals:**
- Correct result types and values for the four groups, without slowing integer and double arithmetic.
- Keep one canonical id per value, so idempotent assert keeps working.
- No row rewrite.

**Non-Goals:**
- Arbitrary precision, canonical lexical changes for existing decimal terms, Cypher numeric changes.

## Decisions

### D1. 128-bit decimal with XPath promotion, through UDFs only where needed

Decimal arithmetic uses `rust_decimal` (96-bit mantissa, 28 significant digits). The static class gains `VClass::Decimal`; promotion follows XPath (integer → decimal → float → double). Codegen emits native SQL when both operands are statically `Int` or `Double`, and decimal UDFs (`tm_dec_add`, `tm_dec_sub`, `tm_dec_mul`, `tm_dec_div`, `tm_dec_neg`, `tm_dec_round`, `tm_dec_ceil`, `tm_dec_floor`, `tm_dec_abs`) on canonical decimal text when either may be a decimal or is `Dynamic`. Aggregates `SUM` and `AVG` use aggregate UDFs that promote across their inputs. `integer / integer` is `tm_dec_div`. Division rounds to 28 significant digits; overflow is an expression error (unbound in `BIND`, false in `FILTER`).

*Alternatives:* `bigdecimal` arbitrary precision (unbounded memory on adversarial input; division needs an arbitrary cut-off anyway); fixed-point `i128` scaled integers (simple but loses the decimal's own scale); keeping `REAL` and formatting results as decimals (still inexact).

### D2. `xsd:float` becomes numeric

Several decimal tests mix `xsd:float` operands. A `TYPED` `xsd:float` literal is read as a number (via its lexical form) in arithmetic and comparison, computed in double precision and returned typed `xsd:float`. Storage is unchanged (`TYPED`).

*Alternatives:* leave floats non-numeric (keeps 6 of the 17 decimal deviations).

### D3. Tagged string results are boxed literals

`UCASE`, `LCASE`, `SUBSTR`, `STRBEFORE`, `STRAFTER`, `REPLACE` and `CONCAT` return boxed text (`lex` + `@tag`) when their first argument is language-tagged (for `CONCAT`, when all arguments share one tag), following SPARQL 1.1 §17.4.3. The value class `VClass::Str` splits into plain and boxed (`VClass::Lit`), so plain strings stay native `TEXT` and cost nothing extra. `LANG`, `STR`, `DATATYPE`, `=` and result encoding already understand boxed values.

*Alternatives:* resolving the tag in Rust after SQL (breaks `FILTER(LANG(UCASE(?x)) = "fr")` inside SQL).

### D4. Zoned dates as canonical `TYPED` terms, with a format bump

A date with a timezone encodes as `TYPED` with datatype `xsd:date` and canonical lexical form `YYYY-MM-DD` followed by `Z` or `±HH:MM` (so `+00:00` and `Z` are one term, as for date-times). A date without a timezone stays inline `DATE`. The ordering key of a date is its starting instant (midnight at its offset; UTC when it has none), so comparisons and `ORDER BY` work across both encodings. Because an older build would encode the same zoned date as `DATE`, mixing builds on one file would give one value two ids; the storage format version is therefore bumped (the next free number at implementation time), with a migration that changes no row, so older builds refuse the file.

*Alternatives:* a new inline layout `(days << 11) | tz` (needs a tag or a reinterpretation of existing `DATE` payloads; neither is available); keep dropping the zone (the status quo, 4 deviations, silent data loss).

### D5. Negated property sets: filter for one step, automaton transition for recursion

A one-step negated set lowers to `TriplePattern(?s, ?p_fresh, ?o)` with `FILTER(?p_fresh NOT IN (…))` (inverse members swap subject and object, and a mixed set is a `UNION` of the two directions). Inside `*`, `+` and `?`, `PathExpr::NegatedSet { forward, inverse }` becomes a transition that the fetch step answers with `p NOT IN (…)` over the subject (or object) index for the current frontier. Virtual predicates (`sys:subject`, `tm:validFrom`, …) are never matched by a negated set, exactly as they are never enumerated by a variable predicate.

*Alternatives:* expanding the set to the complement of known predicates (unbounded, changes with data, wrong under `asOf`).

### D6. Vendor the property-path category

`sparql11/property-path` is copied from the same upstream commit as the other categories (`tests/w3c/data/SOURCE.md`) and added to `CATEGORIES`. Any test that fails after this change is listed with a precise reason in the same commit, so the closed runner stays closed.

## Risks / Trade-offs

- [Decimal UDFs are slower than native `REAL`] → Only used when an operand may be a decimal; the SPARQL benchmark is re-run with an all-integer and an all-decimal shape.
- [Result types change for existing queries] → Documented as a behavior change; values compare equal across numeric types, so most filters are unaffected.
- [Pre-bump zoned dates stay zone-less] → Documented; never-forget forbids repairing them in place. Users can supersede affected statements.
- [The vendored category may expose unrelated path gaps] → Listed with reasons rather than fixed here, unless trivial.

## Migration Plan

One storage format bump with a no-op migration (version number only). Older builds refuse migrated files, as for formats 2–4. No row is rewritten. The CHANGELOG lists the result-type changes and the zoned-date behavior.

## Open Questions

- Should `json-res#jsonres01/02` be addressed by rendering decimals in XSD 1.1 canonical form (`5` rather than `5.0`)? That would change the lexical form of stored decimal terms and is out of scope here.
- Should `CONCAT` of mixed tags return a plain string (SPARQL 1.1) or the first tag? (Proposed: SPARQL 1.1.)
