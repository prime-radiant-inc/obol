# obol — crates.io publishing (design spec)

> 2026-06-06 · Shevek@7998e83e · draft for Bob review · Linear PRI-2097
> Fifth publishing slice. Publish `obol-core` (the library) and `obol-cli` (the `obol` CLI) to
> crates.io. Independent of npm/Go because the crate version comes from `Cargo.toml`, not the tag.

## Goal

`cargo add obol-core` gives the Rust library; `cargo install obol-cli` gives the `obol` binary.
Bootstrap with a token, then move to tokenless OIDC trusted publishing, ending with all channels
(npm/Go/crates + `Version()`) aligned at `0.1.1`.

## Scope

- **Publish** `obol-core` (lib) and `obol-cli` (bin `obol`).
- **Do not publish** `obol-ffi` — it's a cdylib/staticlib for FFI consumers; nobody `cargo add`s it,
  and the prebuilt dylibs already ship via GitHub Releases + the npm/Go bindings. It stays in the
  workspace, just not published.
- Names verified free: `obol-core`, `obol-cli` → 404 (available); bare `obol` → 200 (squat, avoided).

## Why a decoupled `crates-v*` tag namespace

npm and Go stamp their version **from the git tag**; crates.io takes it **from `Cargo.toml`** and
versions are immutable (yank-only). The `v*` tags are already consumed by npm/Go — `v0.1.0` and
`v0.1.1` exist, and npm's `0.1.1` is immutable — so the `v*` flow cannot also drive crates. Crates
therefore gets its **own tag namespace `crates-v*`**, handled by a **separate
`.github/workflows/crates-release.yml`**. `crates-vX.Y.Z` publishes whatever `Cargo.toml` says, and
a guard asserts the tag matches the Cargo version so the two can't drift.

## Manifest changes (the prep)

### 1. Required + listing metadata

`cargo publish` only *warns* locally on missing `description`, but **crates.io's registry rejects an
upload without one** (verified the dep error below; description is a registry-side requirement). Add
metadata — shareable fields go in `[workspace.package]`, per-crate fields in each crate:

`Cargo.toml` `[workspace.package]` (inherited via `field.workspace = true`):
```toml
repository = "https://github.com/prime-radiant-inc/obol"
homepage   = "https://github.com/prime-radiant-inc/obol"
```

`crates/obol-core/Cargo.toml` `[package]`:
```toml
description = "Read AI-agent transcripts (Claude Code, Codex, Pi) and estimate their USD cost."
readme      = "README.md"
keywords    = ["llm", "cost", "tokens", "ai", "transcript"]   # ≤5, each ≤20 chars
categories  = ["parsing", "development-tools"]                 # valid crates.io slugs
repository.workspace = true
homepage.workspace   = true
```

`crates/obol-cli/Cargo.toml` `[package]`:
```toml
description = "CLI to estimate the USD cost of an AI-agent transcript (Claude Code, Codex, Pi)."
readme      = "README.md"
keywords    = ["llm", "cost", "tokens", "cli", "transcript"]
categories  = ["command-line-utilities"]
repository.workspace = true
homepage.workspace   = true
```

`keywords`/`categories` are best-effort for discovery; invalid category slugs only warn, so the set
above uses known-valid slugs (`parsing`, `development-tools`, `command-line-utilities`).

### 2. Per-crate READMEs

`readme` must point inside the crate dir (cargo can't reference the workspace README). The top-level
`README.md` describes the whole project; each crate gets a **focused** README that renders on its
crates.io page:
- `crates/obol-core/README.md` — library: what it does, `estimate_cost` / `refresh_pricing`, a short
  example, link to the repo.
- `crates/obol-cli/README.md` — `cargo install obol-cli` → `obol estimate` / `obol refresh` usage.

### 3. Centralize the `obol-core` path dep with a version

`cargo publish -p obol-cli` **errors today**: *"all dependencies must have a version requirement
… `obol-core` does not specify a version."* Fix once, in `[workspace.dependencies]`:
```toml
obol-core = { path = "crates/obol-core", version = "0.1.0" }
```
Then replace every `obol-core = { path = "../obol-core" }` (in obol-cli `[dependencies]` +
`[dev-dependencies]`, and obol-ffi `[dependencies]`) with `obol-core.workspace = true`. On publish,
cargo strips `path` and keeps `version`, so the published `obol-cli` depends on `obol-core = "0.1.0"`
from crates.io. The `version` here **must** track the workspace version (bumped together — see
Alignment).

## Release workflow: `.github/workflows/crates-release.yml`

Triggered on `crates-v*`. Publishes in dependency order — `obol-core` first, then `obol-cli`
(modern cargo, ≥1.66, *waits* for the just-published crate to appear in the index before returning,
so no manual sleep between them). A guard asserts the tag matches the Cargo version.

**Phase 1 — token (bootstrap), committed first:**
```yaml
name: Crates Release
on:
  push:
    tags: ["crates-v*"]
jobs:
  publish:
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@v4
      - uses: jdx/mise-action@v4
      - name: Guard — tag matches Cargo version
        run: |
          want="${GITHUB_REF_NAME#crates-v}"
          got=$(mise exec rust@1.96.0 -- cargo metadata --no-deps --format-version 1 \
                | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="obol-core"))')
          [ "$want" = "$got" ] || { echo "::error::tag $GITHUB_REF_NAME != Cargo version $got"; exit 1; }
      - name: Publish obol-core then obol-cli
        env:
          CARGO_REGISTRY_TOKEN: ${{ secrets.CARGO_REGISTRY_TOKEN }}
        run: |
          mise exec rust@1.96.0 -- cargo publish --locked -p obol-core
          mise exec rust@1.96.0 -- cargo publish --locked -p obol-cli
```

**Phase 2 — OIDC (after trusted publishing is configured), a small committed edit:** drop the
`CARGO_REGISTRY_TOKEN` env, add `permissions: { id-token: write }`, and mint a short-lived token via
the official action before publishing:
```yaml
      - uses: rust-lang/crates-io-auth-action@v1
        id: auth
      - name: Publish obol-core then obol-cli
        env:
          CARGO_REGISTRY_TOKEN: ${{ steps.auth.outputs.token }}
        run: |
          mise exec rust@1.96.0 -- cargo publish --locked -p obol-core
          mise exec rust@1.96.0 -- cargo publish --locked -p obol-cli
```
(The exact action ref `rust-lang/crates-io-auth-action@v1` is verified during the OIDC step of the
build; it exchanges the GitHub OIDC token for a temporary crates.io token scoped to the trusted
publisher.)

## Operational sequence (the bootstrap→OIDC→align runbook)

1. **Prep** — land the manifest + README changes (Cargo stays `0.1.0`) and `crates-release.yml`
   (token phase). Verify with `cargo publish -p obol-core --dry-run` / `-p obol-cli --dry-run`.
2. **Token secret** — Matt creates a crates.io API token (scoped `publish-new` + `publish-update`)
   and adds it as the repo secret `CARGO_REGISTRY_TOKEN`.
3. **Bootstrap 0.1.0** — push `crates-v0.1.0` → workflow publishes `obol-core` + `obol-cli` at
   `0.1.0` via the token. This *creates* both crates.
4. **Trusted Publishing** — Matt configures TP on each now-existing crate (crates.io → crate
   Settings → Trusted Publishing → GitHub: repo `prime-radiant-inc/obol`, workflow
   `crates-release.yml`). Remove `CARGO_REGISTRY_TOKEN`.
5. **Switch to OIDC** — land the Phase-2 workflow edit.
6. **Align at 0.1.1** — bump to `0.1.1` and push `crates-v0.1.1` → publishes `0.1.1` via OIDC. Now
   npm/Go/crates and `Version()` (which reads the core crate version via the FFI) all read `0.1.1`.
   The bump moves `obol_version()` to `0.1.1`, so the edit set is **four files, not two** (the
   binding tests assert the literal and go red otherwise):
   - `Cargo.toml` `[workspace.package] version = "0.1.1"`
   - `Cargo.toml` `[workspace.dependencies] obol-core` `version = "0.1.1"` (must equal the above)
   - `bindings/python/tests/test_obol.py` — `assert obol.version() == "0.1.1"`
   - `bindings/typescript/test/obol.test.ts` — `assert.equal(await obol.version(), "0.1.1")`
   The Rust FFI test (`env!("CARGO_PKG_VERSION")`) and the CLI `--version` (clap) self-track — no
   change. Prose `0.1.0` examples (`docs/RELEASING.md`, the Python/Go READMEs) are a not-CI-gated
   nicety to sweep at the same time.

## Versioning & alignment

The crate version is `Cargo.toml`'s `[workspace.package] version`, currently `0.1.0`. The bump to
`0.1.1` (step 6) is the **deliberate alignment point**: it also moves `obol-ffi`'s
`obol_version()` to `0.1.1`, so the next npm/Go release would embed core `0.1.1`. The
`[workspace.dependencies] obol-core` `version` must always equal the workspace version, or
`cargo publish -p obol-cli` resolves a non-existent `obol-core` version — the guard plus a
publish-order test catch this.

## Testing & verification

- **Local checks (after prep, before any tag):** `cargo publish -p obol-core --dry-run` succeeds,
  and `cargo package --list -p obol-cli` succeeds (lists files without resolving registry deps).
  **`cargo publish -p obol-cli --dry-run` will NOT succeed pre-bootstrap** — with `path` stripped it
  resolves `obol-core = "0.1.0"` from crates.io, which isn't published yet (`no matching package
  named obol-core found`). That's *correct* (it proves the path→version swap worked); obol-cli's
  real validation happens **during** the bootstrap, where the workflow publishes obol-core first and
  cargo ≥1.66 waits for the index before publishing obol-cli. So **do not gate prep on an obol-cli
  dry-run.** Note: `--dry-run` also doesn't hit registry-side validation (description, name
  collision) — exercised by the real bootstrap publish.
- **Packaged contents:** `cargo package --list -p obol-core` / `-p obol-cli` to confirm the README
  and `Cargo.toml` (with stripped path dep) are in the `.crate`, and no stray files.
- **Post-publish (bootstrap):** after `crates-v0.1.0`, in a scratch dir `cargo new t && cd t &&
  cargo add obol-core@0.1.0 && cargo build` (library resolves from crates.io), and
  `cargo install obol-cli --version 0.1.0` then `obol --help` (binary installs + runs). This is the
  real acceptance — like the Go `go get` consumer test.
- **CI unaffected:** `ci.yml` is untouched (it builds from the workspace, not the registry).

## Out of scope

- Publishing `obol-ffi`; PyPI; changelog automation; `cargo-release` tooling; auto-bumping the
  workspace version from a tag (crates version stays `Cargo.toml`-driven by design — the guard only
  asserts equality, it doesn't rewrite).

## Open threads

None. The crates.io OIDC action ref + output (`rust-lang/crates-io-auth-action@v1`, output `token`,
`id-token: write`) is confirmed current (Mishima review). crates.io Trusted Publishing requires the
crate to **already exist** — no PyPI-style pending publisher — so the token bootstrap is mandatory,
not avoidable. `--locked` is on the publish commands (a committed `Cargo.lock` makes CI publishes
reproducible).

---

## Appendix A — probe (2026-06-06)

- Names: `obol` → HTTP 200 (taken); `obol-core` / `obol-cli` / `obol-ffi` → 404 (available). (crates.io
  API needs a `User-Agent` or it 403s.)
- `cargo publish -p obol-core --dry-run`: succeeds with a *warning* "manifest has no description …".
  So description is registry-enforced, not packaging-enforced.
- `cargo publish -p obol-cli --dry-run`: **errors** — "all dependencies must have a version
  requirement specified when publishing. dependency `obol-core` does not specify a version."
- Manifests use `version.workspace = true` (workspace version `0.1.0`); `obol-core` is a bare `path`
  dep in obol-cli (deps + dev-deps) and obol-ffi. Top-level `README.md` exists; no per-crate READMEs.
