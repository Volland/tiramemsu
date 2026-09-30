# Building and publishing the Python package

This guide takes you from a fresh checkout to a published `tiramemsu` release on PyPI. The package lives in
[`bindings/python`](../bindings/python).

It covers five things:
1. Setting up a development environment.
2. Building and testing the package.
3. Building release artifacts locally.
4. The one-time PyPI setup.
5. Releasing, either automatically from a tag or by hand.

## How the package is put together

| Piece | Where | Role |
|---|---|---|
| Native module `tiramemsu._native` | `bindings/python/src/lib.rs` (crate `tiramemsu-python`, PyO3) | One class, `Native`, with a `call(op, args)` method over JSON text |
| JSON bridge | `bindings/json` (crate `tiramemsu-json`) | Every operation, term conversion and error, shared with the Node.js package |
| Python package `tiramemsu` | `bindings/python/python/tiramemsu/` | `Database`, `View`, `Tx`, term types, `TiramemsuError` |
| Type stubs | `python/tiramemsu/_native.pyi`, `py.typed` | Strict type checking for users |
| Build configuration | `bindings/python/pyproject.toml` | maturin build backend; the version comes from the Cargo workspace |
| Tests | `bindings/python/tests/` | pytest suite |
| CI | `.github/workflows/ci.yml`, job `python` | Builds, tests and type-checks on Linux and macOS for every push |
| Release | `.github/workflows/python-wheels.yml` | Builds every wheel and the sdist; publishes on `v*` tags |

The wheels use CPython's stable ABI (`abi3`, Python 3.9 and later), so each platform needs one wheel,
whatever the Python version. SQLite is compiled into the wheel, so a wheel needs no system library.

## 1. Set up a development environment

You need three things:
- **Rust**: the toolchain pinned in `rust-toolchain.toml` (1.91.1), installed with [rustup](https://rustup.rs).
- **Python**: 3.9 or later. Use a recent one for development, such as 3.12.
- **A C compiler**, for the bundled SQLite: Xcode command-line tools on macOS, `build-essential` on
  Debian or Ubuntu, or the Visual Studio Build Tools on Windows.

```bash
git clone https://github.com/Volland/tiramemsu.git
cd tiramemsu/bindings/python
python3 -m venv .venv
source .venv/bin/activate            # Windows: .venv\Scripts\activate
pip install maturin pytest mypy
```

> **Two `rustc`s on one machine.** If another `rustc` is also installed (for example Homebrew's), put
> rustup's first on your `PATH` with `export PATH="$HOME/.cargo/bin:$PATH"`. Otherwise the build can fail
> with `rustc … is not supported`.

## 2. Build and test

1. Build the package into the virtual environment in editable mode:

   ```bash
   maturin develop            # debug build: fast to compile
   maturin develop --release  # optimized build
   ```

   After changing Python files you need not rebuild. After changing `src/lib.rs` or the bridge in
   `bindings/json`, run `maturin develop` again.

2. Run the tests:

   ```bash
   python -m pytest
   ```

3. Type-check:

   ```bash
   mypy --strict python/tiramemsu
   ```

4. Lint the Rust side from the repository root:

   ```bash
   cargo fmt -p tiramemsu-json -p tiramemsu-python
   cargo clippy -p tiramemsu-json -p tiramemsu-python --all-targets -- -D warnings
   cargo test -p tiramemsu-json
   ```

CI runs the same steps in the `python` job of `.github/workflows/ci.yml`.

## 3. Build release artifacts locally

```bash
cd bindings/python
maturin build --release --out dist     # the wheel for this machine: dist/tiramemsu-X.Y.Z-cp39-abi3-<platform>.whl
maturin sdist --out dist               # the source distribution: dist/tiramemsu-X.Y.Z.tar.gz
```

Check the wheel in a clean environment, preferably on the oldest supported Python:

```bash
python3.9 -m venv /tmp/tiramemsu-check
/tmp/tiramemsu-check/bin/pip install dist/tiramemsu-*.whl
/tmp/tiramemsu-check/bin/python - <<'PY'
import os, tempfile, tiramemsu
db = tiramemsu.Database(os.path.join(tempfile.mkdtemp(), "check.db"))
with db.transact() as tx:
    tx.assert_(tiramemsu.Iri("urn:a"), tiramemsu.Iri("urn:b"), tiramemsu.Iri("urn:c"))
print(len(db.now().triples()), tiramemsu.__version__)
PY
```

Wheels for other platforms are built by the release workflow. maturin can cross-compile (`--target`,
`--zig`), but the workflow is the tested path.

## 4. One-time PyPI setup

The release workflow publishes with PyPI **trusted publishing**: PyPI trusts this repository's workflow,
and no API token is stored anywhere.

1. **Create the accounts.** You need an account on [pypi.org](https://pypi.org/account/register/) with
   two-factor authentication. For rehearsals, create one on
   [test.pypi.org](https://test.pypi.org/account/register/) too.
2. **Register the trusted publisher.** The project does not exist yet, so add a *pending* publisher. On
   PyPI, go to *Your account → Publishing → Add a new pending publisher* and choose GitHub, then fill in:

   | Field | Value |
   |---|---|
   | PyPI Project Name | `tiramemsu` |
   | Owner | `Volland` |
   | Repository name | `tiramemsu` |
   | Workflow name | `python-wheels.yml` |
   | Environment name | `pypi` |

3. **Create the environment.** In the GitHub repository, go to *Settings → Environments → New
   environment* and create `pypi`. Under *Deployment branches and tags*, choose *Selected branches and
   tags* and allow the tag pattern `v*` and the branch `main`, so only release tags and manual runs on
   `main` can publish. As an extra safety catch, add yourself as a required reviewer: each publish then
   waits for your approval in the Actions tab.

   The same with the GitHub CLI:

   ```bash
   gh api -X PUT repos/Volland/tiramemsu/environments/pypi \
     --input - <<< '{"deployment_branch_policy": {"protected_branches": false, "custom_branch_policies": true}}'
   gh api -X POST repos/Volland/tiramemsu/environments/pypi/deployment-branch-policies -f name='v*' -f type=tag
   gh api -X POST repos/Volland/tiramemsu/environments/pypi/deployment-branch-policies -f name=main -f type=branch
   ```

After the first successful upload, the pending publisher becomes a normal publisher of the `tiramemsu`
project. The name `tiramemsu` was free on PyPI when this guide was written; a pending publisher does not
reserve it, so register the publisher and release soon after.

## 5. Release

### Automatically, from the release tag

1. Bump the workspace version in the root `Cargo.toml` (`[workspace.package] version`) and the
   `version` of the three internal dependencies there and in `bindings/node/package.json`.
2. Commit "Release X.Y.Z".
3. Create an annotated tag `vX.Y.Z` and push it.

The package version is `dynamic` in `pyproject.toml`: maturin reads it from the Cargo workspace, so there
is no Python version to bump.

Pushing the tag starts `python-wheels.yml`:
1. **`wheels`** builds six wheels: manylinux x86_64 and aarch64, musllinux x86_64, macOS x86_64 and arm64,
   and Windows x64. It installs the Linux x86_64, macOS arm64 and Windows wheels and runs a smoke test
   with each.
2. **`sdist`** builds the source distribution.
3. **`publish`** downloads every artifact and uploads it to PyPI through trusted publishing. If you added
   a required reviewer, approve the deployment in the Actions tab.

Check the result at <https://pypi.org/project/tiramemsu/>, then run `pip install tiramemsu==X.Y.Z` in a
clean environment. The same tag also starts `npm-publish.yml`, described in
[node-publishing.md](node-publishing.md).

### When the tag run fails

A tag cannot be rebuilt with a fix, and moving a pushed tag is bad practice. If a wheel fails on the tag
(so nothing was published), fix it on `main`, then run the workflow by hand from `main` (*Actions → Python
wheels → Run workflow*, branch `main`) with **Publish** checked, or:

```bash
gh workflow run python-wheels.yml --ref main -f publish=true
```

The version still comes from the Cargo workspace, so this publishes the tagged version with the fix. Do it
only while `main` has not moved on to the next version's changes in the Python package or its crates.

### Rehearse on TestPyPI

Run the workflow by hand (*Actions → Python wheels → Run workflow*) with **Publish** unchecked. It builds
and uploads every wheel as a workflow artifact without publishing. To rehearse the upload itself:
1. Download the artifacts.
2. Upload them to TestPyPI with twine and a TestPyPI token:

   ```bash
   pip install twine
   twine upload --repository testpypi dist/*
   pip install --index-url https://test.pypi.org/simple/ tiramemsu
   ```

### By hand, without the workflow

This path is for emergencies, or for a platform the workflow does not build.
1. Create an API token under *Account settings → API tokens* on PyPI, scoped to the `tiramemsu` project.
2. Run:

   ```bash
   cd bindings/python
   export MATURIN_PYPI_TOKEN=pypi-…        # never commit it
   maturin publish                          # builds this platform's wheel and the sdist, then uploads both
   ```

   Or upload wheels you built elsewhere:

   ```bash
   twine upload dist/*
   ```

PyPI refuses a second upload of the same file name. A version cannot be replaced, only yanked, so a fix
means a new version.

## Troubleshooting

| Symptom | Cause and fix |
|---|---|
| `rustc 1.87.0 is not supported` | An older `rustc` is first on `PATH`. Use rustup's (`export PATH="$HOME/.cargo/bin:$PATH"`) |
| `maturin develop` says it needs a virtualenv | Activate the venv first, or use `maturin build` and `pip install` the wheel |
| `ImportError: … _native` after pulling | The native module is stale. Run `maturin develop` again |
| `readme path … does not exist` during `maturin sdist` | Every workspace crate needs the README its manifest names. The sdist includes the workspace crates it depends on |
| The `publish` job waits, then fails with `Branch "…" is not allowed to deploy to pypi` | The run is not on a `v*` tag or `main`. Run it from `main` |
| The `publish` job fails with `invalid-publisher` | The trusted publisher on PyPI does not match. Check the owner, repository, workflow file name and environment (`pypi`) |
| `File already exists` on upload | That version is already on PyPI. Bump the version |
