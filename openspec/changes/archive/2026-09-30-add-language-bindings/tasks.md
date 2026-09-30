## Dependencies

- The facade, both query front ends, the path engine and named graphs are archived and are the base of this change. No core crate changes.
- Test code references `lat.md/` specs only for the bridge; the wrapper suites are described in `lat.md/bindings.md#Test Strategy`.

## 1. JSON bridge

- [x] 1.1 Create `bindings/json` (`tiramemsu-json`) with `Database::open`, `call` and `call_text`, and the error type with codes. Test unknown operations and options (spec `json-bridge` "One call surface").
- [x] 1.2 Implement the term conversions in both directions, including `{"$int"}`, `{"lex", "datatype" | "lang"}` and times from numbers or RFC 3339 strings. Test the round trips (spec "Terms are exact in both directions").
- [x] 1.3 Implement the views on both clocks and the read operations `sparql`, `cypher`, `triples`, `path`, `events`, `graphs`, `graphMembers` and `values`. Test the Alice story on all six views (spec "Views select both clocks", "Query results", "Paths and graphs").
- [x] 1.4 Implement `transact` with named references, `cypherWrite` and `with`, and the failure rules. Test refs, dry run, failure commits nothing, cascade and speculation (specs "Transactions are op lists with named references", "Speculation keeps nothing", "Errors carry a code").
- [x] 1.5 Accept `{"ref"}` in subject and object positions, not only as an eid. Found by the first test run: a layer needs a statement as its subject.

> Notes (group 1): SPARQL over the history view returns a set of `(s, p, o)`, so two episodes of one fact are one row there and two in Cypher and `triples`. The time-travel article's table was corrected to say so. `TxReport.existing` excludes matches that the same transaction inserted, so it is empty when an assert repeats a statement created earlier in the same transaction.

## 2. Node.js package

- [x] 2.1 Create `bindings/node` following oxilite's layout: `Cargo.toml`, `build.rs`, `src/lib.rs` (`Native` with `call`), `scripts/build-native.mjs`, `package.json`, `tsconfig.json`.
- [x] 2.2 Write `lib/index.ts`: `Database`, `View`, `Tx`, `Ref`, term helpers and decoding, `TiramemsuError`. `tsc --strict` passes (spec `node-binding` "Package layout and native surface").
- [x] 2.3 Write the vitest suite: the Alice story, layers by reference, cascade and events, speculation, dry run, error codes, big integers, dates, paths, graphs, persistence (specs "Database and views", "Transactions with references", "Terms map to JavaScript", "Errors have codes", "Events, paths and graphs").
- [x] 2.4 Write the package README and set `files`, `engines` and `publishConfig` in `package.json`.

## 3. Python package

- [x] 3.1 Create `bindings/python` following oxilite's layout: `Cargo.toml`, `build.rs`, `pyproject.toml` (maturin, dynamic version), `src/lib.rs` (`Native` with `call`, GIL released), licences.
- [x] 3.2 Write the wrapper: `Database`, `View`, `Tx` as a context manager and a list, `Ref`, term dataclasses and decoding, `TiramemsuError`, `_native.pyi`, `py.typed`. `mypy --strict` passes (spec `python-binding` "Package layout and native surface").
- [x] 3.3 Write the pytest suite: the Alice story, layers by reference, cascade and events, speculation, dry run, error codes and a raising block, big integers, datetimes, paths, graphs, persistence, threads (specs "Database and views", "Transactions as a context manager or a list", "Terms map to Python", "Errors have codes", "Query results are typed").
- [x] 3.4 Write the package README.

## 4. Release engineering

- [x] 4.1 Add CI jobs to `.github/workflows/ci.yml` for Python and Node on Linux and macOS, and make `cargo fmt --check` and `cargo clippy` cover the binding crates (spec `binding-release` "Continuous integration builds and tests both packages").
- [x] 4.2 Add `.github/workflows/python-wheels.yml` (six wheels, sdist, smoke tests, trusted publishing) (spec "PyPI release").
- [x] 4.3 Add `.github/workflows/npm-publish.yml` (five addons, pack, smoke test, provenance publish) and make the loader name the platform when no addon matches (spec "npm release").
- [x] 4.4 Write `docs/python-publishing.md` and `docs/node-publishing.md` (spec "Publishing guides").
- [x] 4.5 Build a wheel and an sdist locally, install the wheel in a clean virtual environment and run the smoke test; run `npm pack` and inspect the tarball.

> Notes (group 4): the workflows are written but have not run on GitHub, so runner labels (`ubuntu-24.04-arm`, `macos-15-intel`) and the aarch64 and musllinux wheel builds are unverified until the first manual run. Task 4.3 stays open until the Node loader names the platform and the package holds the five addons.

> Notes (groups 2 and 3): the first suites of both packages passed but left several spec scenarios untested. Adding those found a real bug: the Python wrapper wrote `datetime` and `date` as integer milliseconds, so they came back as numbers, and the Node wrapper wrote a `Date` correctly but read an `xsd:dateTime` back as a plain object. Both now round-trip. The Python annotation test no longer falls back to `xfail`, so a failure there is visible. The Node suite has 13 tests and the Python suite 42.
>
> Notes (task 4.5): a release wheel installs and runs from a clean virtual environment, and the sdist, which carries the bridge and the facade crates, builds from source in a clean environment. `npm pack` holds `dist/`, the licences, the README and one addon; the release workflow adds the other four.

## 5. Docs and archive

- [x] 5.1 Update `lat.md/bindings.md` and `lat.md/api.md`, the site (an article or a page section), and the README (install lines, status). `lat check` passes.
- [x] 5.2 Run `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings` and `cargo test --workspace`.
- [ ] 5.3 Archive the change (`openspec archive add-language-bindings`) so its specs merge into `openspec/specs/`.
