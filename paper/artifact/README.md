# Revision 4 verification artifact

Retained finite-model checks, workflow policy, engine repair probes and PDF QA. Revision 4 repairs the four engine gaps; earlier counterexample results remain in `review-v2/` and `review-v3/`, and the pre-repair probe is preserved in `revisions/v3/artifact/`.

Run from the repository root:

```sh
python3 paper/artifact/check_model.py paper/review-v4/model-checks.json
python3 paper/artifact/check_workflow.py paper/review-v4/workflow-check.json
cargo run --offline --locked --manifest-path paper/artifact/Cargo.toml --target-dir /tmp/tiramemsu-paper-v4-target
cargo test --offline -p tm-core --test conformance
cargo test --offline --workspace --no-fail-fast
python3 paper/artifact/build.py --revision=4 --publish
lat check
```

Rust dependencies are pinned in `Cargo.lock`; offline execution requires the local Cargo cache. Omit `--offline` if dependencies must be downloaded. The finite checker needs only Python's standard library. PDF checks need TeX Live, BibTeX, Poppler and PyMuPDF; packages are listed in `paper/README.md`.

The finite checker uses seed 20261003 and tests reference interiors, cascades, effective validity, admissible corrections, general metagraph roundtrips, snapshots and bounded journeys. The workflow check exercises exclusion of old confidence/evidence and foreign membership annotations, followed by fresh revalidated annotations. It is a policy-model check rather than an end-to-end agent evaluation.

The Rust probe creates a temporary database and verifies prevention of retracted references, missing references, replay on dropped memberships and direct-SQL backdating. Successful output is four `PREVENTED` lines. The conformance integration tests add recursive exclusion, patched-reference substitution, invalid-patch rollback, cardinality side effects and format-2 migration, on both SQLite hosts.

Format 3 preserves old rows and installs date guards. Existing invalid references are not sanitized automatically. Raw writers must keep schema and metadata intact and follow the engine's transaction protocol; SQL date validation does not enforce all reference/schema rules.

The build script uses a fresh temporary directory, stabilizes citations, checks keys and all six arXiv identifiers, extracts DOI links and renders all pages under `review-v4/builds/`. Publication uses a new filename; the selected output is recorded in `review-v4/build-check.json`. Source hashes and tool metadata are retained in `review-v4/artifact-manifest.json`.

These checks are correctness evidence, not universal proof certificates, performance benchmarks or independent peer review. A public permanent archival release remains pending.
