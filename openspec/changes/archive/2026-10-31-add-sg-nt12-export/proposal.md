## Why

The paper *When Is a Temporal Statement Store a Statement Graph?* says when tiramemsu's records export to a Statement Graph. LeanRDF v0.3.0 made that concrete: a statement-graph format in RDF 1.2, with an exact decoder proven in Lean (`decode_toRDF`, `toRDF_decode`). `tm export` writes only the live, flat statements, which loses statement ids, retractions and both clocks. With an export in LeanRDF's format, a tiramemsu file can leave the store whole, and LeanRDF can check what it wrote.

## What Changes

- `tm export --format sg-nt12` writes every statement as an RDF 1.2 reifier `stmt:e rdf:reifies <<( s p o )>>` with its clocks. It asserts the base triple of the statements live in the view (`now`, `--as-of T`, `--history`, with `--valid-at MS`).
- The output is canonical RDF 1.2 N-Triples in LeanRDF's order, so LeanRDF decodes it and reproduces it byte for byte.
- Valid time is exported as `tm:validFromMs` and `tm:validToMs` (`xsd:integer` epoch ms). The virtual `tm:validFrom` and `tm:validTo` that queries use stay `xsd:dateTime`, so no IRI has two meanings. LeanRDF v0.3.1 renames its predicates to match.
- Left out of the export:
  - `sys:` bookkeeping;
  - statements added and retracted in one transaction, which no view ever shows.
- The format is export-only.
- New vocabulary constants: `RDF_REIFIES`, `TM_VALID_FROM_MS`, `TM_VALID_TO_MS`.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `tm-cli`: `Import and export` names the new format, and `Statement graph export` is added.

## Impact

- **Code:** `crates/tiramemsu-cli/src/{sg.rs,rdfio.rs,main.rs}`, `crates/tm-core/src/vocab.rs`, and the tests in `crates/tiramemsu-cli/tests/sg.rs`, with recorded outputs in `tests/sg/`.
- **lat.md:**
  - `api#CLI` gets the new format;
  - a `Statement Graph Export` section is added;
  - a decision row is added to `overview`.
- **Docs:** README, `docs/statement-graph-export.md`, two articles, and a chapter in the N3 minibook.
- **External:** LeanRDF (`Volland/rdf-lean`) v0.3.1 records the files in `tests/sg/` and checks each one.
- **Requirements:**
  - added: [[openspec:tm-cli#Statement graph export]];
  - modified: [[openspec:tm-cli#Import and export]].
