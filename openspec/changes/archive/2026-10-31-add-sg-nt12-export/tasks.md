## 1. Export

- [x] 1.1 `crates/tiramemsu-cli/src/sg.rs`: `export_sg` with LeanRDF's order and canonical escapes; annotate `@tg: implements:: [[openspec:tm-cli#Statement graph export]]`
- [x] 1.2 `ExportFormat::SgNt12` (`sg-nt12`), `--as-of`, `--history`, `--valid-at` on `tm export`, refused with other formats
- [x] 1.3 `RDF_REIFIES`, `TM_VALID_FROM_MS`, `TM_VALID_TO_MS` in `tm-core` vocab

## 2. Tests

- [x] 2.1 `tests/sg.rs` with recorded outputs in `tests/sg/`; each scenario annotated `@tg: verifies::`
- [x] 2.2 Cross-check: LeanRDF decodes every recorded file in its view and re-exports it byte for byte; `routes` equals LeanRDF's goldens

## 3. Docs

- [x] 3.1 lat.md: `api#CLI`, a `Statement Graph Export` section, a decision row in `overview`
- [x] 3.2 README, `docs/statement-graph-export.md`, the N3 minibook chapter, and two articles on the site
- [x] 3.3 `lat check`, `tg check`, `tg ontology check`, `tg trace tm-cli --gaps`
