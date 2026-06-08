# Releasing

**One tag releases everything.** A single `vX.Y.Z` tag on `main` drives all four channels —
npm + Go (`release.yml`), crates.io (`crates-release.yml`), and PyPI (`pypi-release.yml`) all
trigger on `v*`. So a release is:

```bash
# bump [workspace.package] version + [workspace.dependencies] obol-core + the two binding
# version tests (bindings/python/tests/test_obol.py, bindings/typescript/test/obol.test.ts),
# commit, merge, then:
git tag vX.Y.Z && git push origin vX.Y.Z
```

Every publish step is **idempotent**: a re-fired tag (e.g. to fix one channel) skips the
channels already published at that version instead of failing the run. crates.io is skipped on
prerelease tags (`vX.Y.Z-rc.N`) since `Cargo.toml` carries the release version; npm routes
prereleases to the `next` dist-tag and PyPI accepts them as `rcN`.

## npm — `@primeradianthq/obol` (the TypeScript binding)

Publishing is **tag-driven**. Push a tag `vX.Y.Z` on `main` and `.github/workflows/release.yml`:

1. builds the release cdylib on all four native runners (macOS arm64/x64, Linux x64/arm64) and
   strips it (`strip -x` on macOS, `strip` on Linux);
2. assembles `bindings/typescript/native/<platform>-<arch>/libobol_ffi.{dylib,so}`;
3. builds `dist/` with tsup and stamps the package version from the tag (`v1.2.3` → `1.2.3`);
4. publishes `@primeradianthq/obol` to npm (with provenance);
5. cuts a GitHub Release for the tag with the four dylibs attached.

Prerelease tags (`v1.2.3-rc.1`) publish to the `next` dist-tag, not `latest`.

### One-time bootstrap (first publish only)

npm can't attach a trusted publisher to a package that doesn't exist yet, and npm has deprecated
classic (2FA-bypassing automation) tokens — so the **first** publish is a one-time **manual**
publish from a maintainer's machine. Every release after is tokenless OIDC via the CI workflow.

1. **Build the complete first-release tarball** (it needs all four platforms' dylibs, so it's
   assembled from a CI run, not one machine). Either run `release.yml` once to get the dylibs as
   artifacts, then assemble + `npm pack` locally with `publishConfig.provenance` removed (a manual
   publish has no CI OIDC to sign provenance) — or ask the assistant to produce it. Result:
   `primeradianthq-obol-<version>.tgz` containing `dist/`, `native/{darwin,linux}-{arm64,x64}/`,
   `package.json` (version set, `publishConfig: {access: public}`), `README.md`.
2. **Publish it manually**, logged into npm (answer the 2FA OTP interactively):
   `npm publish /path/to/primeradianthq-obol-<version>.tgz --access public`. This creates the
   package on the registry.
3. **Configure the trusted publisher** on npmjs.com for the now-existing `@primeradianthq/obol`:
   Package → Settings → Trusted Publisher → GitHub Actions, repo `prime-radiant-inc/obol`,
   workflow `release.yml`.
4. **From now on, releases are tokenless:** `git tag vX.Y.Z && git push origin vX.Y.Z` → the
   workflow builds the 4 dylibs, assembles, and `npm publish`es via OIDC (with provenance). No
   secret needed; delete any leftover `NPM_TOKEN`.

### Notes

- Trusted publishing + provenance require **npm ≥ 11.5.1**; the workflow pins it (`npm@^11.5.1`).
  Provenance is generated on the CI/OIDC releases (public repo + `id-token: write`); the one-time
  manual bootstrap publish has no provenance (no CI), which is why its tarball drops
  `publishConfig.provenance`.
- `version()` (the binding API) returns the **Rust core** version (`obol_version()`, e.g. `0.1.1`),
  which is intentionally decoupled from the npm package version stamped from the tag.

## Go — `github.com/prime-radiant-inc/obol-go` (the Go binding)

Go has no registry; "publishing" is pushing a git tag to the **separate** `obol-go` repo, which
`proxy.golang.org` caches automatically. The same `vX.Y.Z` tag that drives npm also drives Go: the
`publish-go` job in `release.yml` builds nothing new — it reuses the four release dylibs, runs
`scripts/assemble-obol-go.sh` to generate the module (flattened source + per-platform `go:embed`
files + `go.mod`/`go.sum`), smoke-tests it, then commits and tags `obol-go`.

- **Auth:** a fine-grained PAT `OBOL_GO_TOKEN` (secret on this repo) with **Contents: Read and write**
  on `obol-go` only. Deploy keys are disabled org-wide; the default `GITHUB_TOKEN` can't reach a
  second repo. Keep `obol-go` workflow-free so Contents-only suffices. The PAT expires — rotate it
  (a GitHub App is the no-rotation upgrade if that becomes a chore).
- **Immutability:** once the proxy serves `vX.Y.Z` it's cached forever; the job refuses a tag that
  already exists, and the smoke test gates a broken assembly before the tag is pushed.
- **No C toolchain for consumers:** the binding is purego (`CGO_ENABLED=0`); the published module
  embeds the platform dylib and extracts+`dlopen`s it at first use. `version()` returns the Rust
  core version, decoupled from the module tag (same as npm).

## crates.io — `obol-core` + `obol-cli`

Triggered by the same `v*` tag via `.github/workflows/crates-release.yml`. crates take their version
from `Cargo.toml`, so a guard asserts the `vX.Y.Z` tag matches the workspace version; the workflow
publishes `obol-core` then `obol-cli` (cargo ≥1.66 waits for the index between them), skipping any
version already on crates.io (versions are immutable). Prerelease tags are skipped. `obol-ffi` is not
published.

**Bootstrap (one-time, token).** crates.io Trusted Publishing requires the crate to exist first.
1. Create a crates.io API token (scopes `publish-new` + `publish-update`); add it as the repo secret
   `CARGO_REGISTRY_TOKEN`.
2. Push `crates-v0.1.0` → publishes `obol-core` + `obol-cli` at `0.1.0` (creates the crates).
3. On crates.io, configure **Trusted Publishing** on each crate (Settings → Trusted Publishing →
   GitHub Actions, repo `prime-radiant-inc/obol`, workflow `crates-release.yml`). Remove
   `CARGO_REGISTRY_TOKEN`.
4. Switch the workflow to OIDC (below) — every release after is tokenless.

**Steady state (OIDC).** The workflow mints a short-lived token via
`rust-lang/crates-io-auth-action@v1` (needs `permissions: id-token: write`). To release: bump
`[workspace.package] version` **and** `[workspace.dependencies] obol-core` version together, update
the two binding version tests (`bindings/python/tests/test_obol.py`,
`bindings/typescript/test/obol.test.ts`), commit, then push `vX.Y.Z`.

## PyPI — `primeradianthq-obol` (import `obol`)

Triggered by the same `v*` tag (version stamped from the tag). `.github/workflows/pypi-release.yml`
builds four wheels — macOS arm64/x64 on the runners, Linux x64/arm64 in `manylinux_2_28` containers
(`scripts/build-pypi-wheel-linux.sh` + `auditwheel repair`) — each bundling that platform's prebuilt
`libobol_ffi`, then publishes via **tokenless OIDC** (`pypa/gh-action-pypi-publish` with
`skip-existing: true`, so a re-fired tag skips already-published wheels).

- **Trusted Publishing (pending publisher, no token ever).** Configured once on PyPI: project
  `primeradianthq-obol`, owner `prime-radiant-inc`, repo `obol`, workflow `pypi-release.yml`, **no
  environment** (the workflow must stay environment-less to match). PyPI's pending-publisher flow
  creates the project on the first publish — no bootstrap, unlike crates/npm.
- **Wheels only** (no sdist — a source build needs Rust + the cdylib). `import obol` stays; the
  distribution name `primeradianthq-obol` differs because bare `obol` is a taken PyPI name.
- The wheel is abi-agnostic (`py3-none-<plat>`): a `setup.py` forces an impure, platform-tagged
  wheel (`OBOL_WHEEL_PLAT` per macOS arch; `auditwheel` sets the manylinux tag on Linux).
- To release: push `vX.Y.Z` (the version is stamped from the tag into `pyproject.toml`).
