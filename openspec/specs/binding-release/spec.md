# binding-release Specification

## Purpose
Defines how both packages are built, tested and published: continuous integration on every push, a wheel and sdist workflow for PyPI, an addon-per-platform workflow for npm, and the guides that set up the registries once.

## Requirements

### Requirement: Continuous integration builds and tests both packages

The `ci` workflow SHALL build both packages and run their tests on Linux and macOS for every push and pull request: `cargo test -p tiramemsu-json`, then for Python `maturin develop`, `pytest` and `mypy --strict`, and for Node `npm run build:native`, `npm run build` and `npm test`. `cargo fmt --check` and `cargo clippy` SHALL cover the binding crates.

#### Scenario: A broken wrapper fails the build
- **WHEN** a change makes one Python or Node test fail
- **THEN** the `ci` workflow fails

### Requirement: PyPI release

A workflow `python-wheels.yml` SHALL build abi3 wheels for linux x86_64 and aarch64 (manylinux), musllinux x86_64, macOS x86_64 and arm64, and Windows x64, plus an sdist, smoke-test a wheel of each of Linux, macOS arm64 and Windows, and publish to PyPI through trusted publishing from the GitHub environment `pypi`. It SHALL publish on a `v*` tag and on a manual run only when `publish` is checked. It SHALL use no stored token.

#### Scenario: Rehearsal without publishing
- **WHEN** the workflow is run by hand with `publish` unchecked
- **THEN** every wheel and the sdist are uploaded as workflow artifacts and nothing is published

#### Scenario: Tag publishes
- **WHEN** a tag `v0.1.0` is pushed
- **THEN** the `publish` job runs in the `pypi` environment with `id-token: write`

### Requirement: npm release

A workflow `npm-publish.yml` SHALL build the addon on linux x64 (glibc) and arm64, macOS x64 and arm64, and Windows x64, collect every `tiramemsu.<platform>-<arch>.node` into the package, smoke-test the package on each platform it can run on, and publish `@tiramemsu/node` with provenance from the GitHub environment `npm`, using the secret `NPM_TOKEN`. It SHALL publish on a `v*` tag and on a manual run only when `publish` is checked.

#### Scenario: Package carries every platform
- **WHEN** the workflow's pack step runs
- **THEN** the tarball holds `dist/` and one addon for each of the five platforms

#### Scenario: Import on an unsupported platform
- **WHEN** the package is loaded on a platform with no prebuilt addon
- **THEN** the import fails with a message naming the platform and architecture

### Requirement: Publishing guides

`docs/python-publishing.md` and `docs/node-publishing.md` SHALL give the one-time registry setup (the PyPI pending trusted publisher with owner, repository, workflow name and environment; the npm organization, token and environment), the release steps, how to rehearse, and a troubleshooting table.

#### Scenario: Registry values are stated
- **WHEN** a maintainer opens `docs/python-publishing.md`
- **THEN** it lists the PyPI project name `tiramemsu`, the owner `Volland`, the repository `tiramemsu`, the workflow `python-wheels.yml` and the environment `pypi`
