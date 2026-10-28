## 1. Upstream

- [x] 1.1 Prepare the eyeron patch (`cli` feature gating `ureq` and binaries; `ReasonerOptions::now`, with tests): `upstream/0001-now-option-and-cli-feature.patch` against eyeron `c4adb8c`.
- [x] 1.2 Propose it to the maintainer: https://github.com/eyereasoner/eyeron/pull/25 (2026-10-28, rebased on eyeron `804488d`).
- [ ] 1.3 eyeron published on crates.io.

## 2. Here

- [x] 2.0 Interim: pin the workspace to the PR branch on `Volland/eyeron` (`e762581`) with `default-features = false`, so no build links `ureq`/`ring` (the 0.9.0 linux-aarch64 wheel failed on `ring`).

- [ ] 2.1 Switch the workspace dependency to the registry with `default-features = false`; remove `publish = false` from `tm-n3`.
- [ ] 2.2 Replace the `time:localTime` rewrite with `ReasonerOptions::now`; annotate with `@tg: implements:: [[openspec:eyeron-dependency#Clock from options]]`.
- [ ] 2.3 Tests for both scenarios, annotated with `@tg: verifies::`.
- [ ] 2.4 `tg check`, `tg ontology check`, `tg trace eyeron-dependency --gaps`.
