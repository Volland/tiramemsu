## 1. SPARQL

- [x] 1.1 Classify `tm:validFrom` / `tm:validTo` templates as valid-time writes; refuse transaction-time `tm:` predicates in templates with `Unsupported` at plan time.
- [x] 1.2 Runner: assert reified and annotated statements with their bounds; merge bounds for stored statements into one supersede per statement and operation; delete clears a matching bound.
- [x] 1.3 Value mapping (`xsd:dateTime`, `xsd:date`, `xsd:integer`) and `InvalidQuery` for other values, non-statement subjects and conflicting values.

## 2. Cypher

- [x] 2.1 `tiramemsu.statementId`, `tiramemsu.transaction`, `tiramemsu.retraction` in the registry and executor.
- [x] 2.2 Transaction terms as nodes: property access (metadata, `number`, `instant`), `labels`, `elementId`, `keys`/`properties`; `SET` refused.
- [x] 2.3 A transaction IRI in a pattern map binds the transaction node.

## 3. Tests

- [x] 3.1 SPARQL: insert with annotation bounds, idempotent re-insert, close an interval, clear a bound, delete+insert one supersede, date and integer values, errors.
- [x] 3.2 Cypher: statement id, transaction metadata, retraction in history, transaction IRI in a map.
- [x] 3.3 SPARQL and Cypher agree on valid time written by either and on id and transaction reads.

## 4. Documentation

- [x] 4.1 `lat.md/query.md` (Temporal Syntax, SPARQL updates, Cypher), `lat.md/tests.md`; `lat check`; `openspec validate`.
- [x] 4.2 CTEG article: write valid time in SPARQL, read the transaction of a causal edge in Cypher.
