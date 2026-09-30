## REMOVED Requirements

### Requirement: Non-time graphs are unsupported
**Reason**: Named graphs are now supported. `FROM`, `FROM NAMED` and `GRAPH` with a non-`tm:` IRI name graphs, and `GRAPH ?g` ranges over graphs, as the `named-graphs` capability specifies. `GRAPH` with a `tm:` IRI still fails with a `Parse` error that names `SERVICE` ("GRAPH does not carry time" is unchanged), and a `GRAPH` block inside a time `SERVICE` group is evaluated in that group's view.
**Migration**: Queries that were rejected with `Unsupported { feature: "named graph" }` or `Unsupported { feature: "GRAPH variable" }` now run. No client change is needed.
