## 1. Upstream

- [ ] 1.1 Prepare the eyeron patch (`cli` feature gating `ureq` and binaries; `ReasonerOptions::now`) and propose it to the maintainer (only with the project owner's go-ahead).
- [ ] 1.2 eyeron published on crates.io.

## 2. Here

- [ ] 2.1 Switch the workspace dependency to the registry with `default-features = false`; remove `publish = false` from `tm-n3`.
- [ ] 2.2 Replace the `time:localTime` rewrite with `ReasonerOptions::now`; annotate with `@tg: implements:: [[openspec:eyeron-dependency#Clock from options]]`.
- [ ] 2.3 Tests for both scenarios, annotated with `@tg: verifies::`.
- [ ] 2.4 `tg check`, `tg ontology check`, `tg trace eyeron-dependency --gaps`.
