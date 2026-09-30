# Building and publishing the Node.js package

This guide takes you from a fresh checkout to a published `@tiramemsu/node` release on npm. The package
lives in [`bindings/node`](../bindings/node).

## How the package is put together

| Piece | Where | Role |
|---|---|---|
| Native addon | `bindings/node/src/lib.rs` (crate `tiramemsu-node`, napi-rs) | One class, `Native`, with a `call(op, args)` method over JSON text |
| JSON bridge | `bindings/json` (crate `tiramemsu-json`) | Every operation, term conversion and error, shared with the Python package |
| TypeScript wrapper | `bindings/node/lib/index.ts` | `Database`, `View`, `Tx`, term helpers, `TiramemsuError` |
| Build scripts | `bindings/node/scripts/` | `build-native.mjs` builds the addon and copies it to `tiramemsu.<platform>-<arch>.node` |
| Tests | `bindings/node/test/` | vitest suite |
| CI | `.github/workflows/ci.yml`, job `node` | Builds, tests and type-checks on Linux and macOS for every push |
| Release | `.github/workflows/npm-publish.yml` | Builds the addon on five platforms, packs one package, publishes on `v*` tags |

A Node addon is a native library for one operating system and CPU, so the published package carries one
prebuilt file per platform and loads the one that matches. The release workflow builds these on:

| Platform | Runner |
|---|---|
| `linux-x64` | `ubuntu-latest` |
| `linux-arm64` | `ubuntu-24.04-arm` |
| `darwin-arm64` | `macos-14` |
| `darwin-x64` | `macos-15-intel` |
| `win32-x64` | `windows-latest` |

A platform not in this list gets an import error that names it. Runner labels change over time; if a
runner label is retired, GitHub reports it on the first run and you replace it in `npm-publish.yml`.

## 1. Set up and build

You need Rust (the toolchain in `rust-toolchain.toml`), Node 18 or later (22 recommended), and a C
compiler for the bundled SQLite.

```bash
cd bindings/node
npm ci
npm run build:native      # cargo build --release, then copies the addon next to package.json
npm run build             # tsc: dist/index.js and dist/index.d.ts
npm test                  # vitest
```

Use `npm run build:native -- --debug` for a faster, unoptimized build.

## 2. One-time npm setup

1. **The organization.** The scope `@tiramemsu` is an npm organization. Add your account as an owner, and
   check it with `npm org ls tiramemsu`.
2. **A token.** On npmjs.com, under *Access Tokens*, create a *granular access token* with read and write
   access to packages in the `@tiramemsu` scope, and an expiry you can live with. If your account requires
   two-factor authentication for writes, choose a token type that is allowed to bypass it for automation.
3. **The GitHub environment.** In the repository, go to *Settings → Environments → New environment* and
   create `npm`. Under *Deployment branches and tags* allow the tag pattern `v*` and the branch `main`.
   Add the token as an *environment secret* named `NPM_TOKEN`. As an extra safety catch, add yourself as
   a required reviewer.

   The same with the GitHub CLI:

   ```bash
   gh api -X PUT repos/Volland/tiramemsu/environments/npm \
     --input - <<< '{"deployment_branch_policy": {"protected_branches": false, "custom_branch_policies": true}}'
   gh api -X POST repos/Volland/tiramemsu/environments/npm/deployment-branch-policies -f name='v*' -f type=tag
   gh api -X POST repos/Volland/tiramemsu/environments/npm/deployment-branch-policies -f name=main -f type=branch
   gh secret set NPM_TOKEN --env npm
   ```

Publishing with `--provenance` needs the workflow to run on a public repository with `id-token: write`,
which `npm-publish.yml` sets. The published package then shows a provenance badge linking to the run.

## 3. Release

1. Bump the workspace version in the root `Cargo.toml` and the `version` in `bindings/node/package.json`.
   The Python package reads its version from the same workspace, so both stay in step.
2. Commit "Release X.Y.Z", create an annotated tag `vX.Y.Z` and push it.

The tag starts `npm-publish.yml`:
1. **`addon`** runs on each of the five platforms, builds the addon, and runs the whole test suite on it,
   which is the smoke test. It uploads `tiramemsu.<platform>-<arch>.node`.
2. **`publish`** downloads the five files into `bindings/node`, builds the TypeScript, checks that all five
   are present, runs `npm pack --dry-run`, and runs `npm publish --provenance --access public`. If you
   added a required reviewer, approve the deployment in the Actions tab.

Check <https://www.npmjs.com/package/@tiramemsu/node>, then run `npm install @tiramemsu/node` in a clean
project.

### Rehearse

Run the workflow by hand (*Actions → npm publish → Run workflow*) with **Publish** unchecked. It builds and
tests the addon on every platform and uploads the files as artifacts, without publishing. To look at the
tarball yourself, download the artifacts into `bindings/node`, run `npm run build`, then `npm pack` and
list it with `tar tzf tiramemsu-node-*.tgz`.

### When the tag run fails

Do not move the tag. Fix the problem on `main`, then run the workflow by hand from `main` with **Publish**
checked, or `gh workflow run npm-publish.yml --ref main -f publish=true`.

## Troubleshooting

| Symptom | Cause and fix |
|---|---|
| `Cannot find module './tiramemsu.<platform>-<arch>.node'` | No addon for this platform is in the package. Build it with `npm run build:native` |
| `npm error 403 … You cannot publish over the previously published versions` | That version is already on npm. Bump the version |
| `npm error 404 … '@tiramemsu/node' is not in this registry` on publish | The token cannot publish to the `@tiramemsu` scope, or the organization does not exist. Check step 2 |
| `npm error need auth` or `EOTP` | The token is missing from the environment, or it requires a one-time password. Use an automation-capable token |
| The `publish` job waits, then fails with `Branch "…" is not allowed to deploy to npm` | The run is not on a `v*` tag or `main` |
| A runner label is not found | GitHub retired it. Replace the label in `.github/workflows/npm-publish.yml` |
