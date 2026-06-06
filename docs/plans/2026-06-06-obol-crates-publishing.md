# obol — crates.io publishing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Publish `obol-core` + `obol-cli` to crates.io via a decoupled `crates-v*` tag namespace; bootstrap at `0.1.0` with a token, then move to OIDC trusted publishing and align all channels at `0.1.1`.

**Architecture:** crates version comes from `Cargo.toml` (not the git tag), so a new `crates-release.yml` fires on `crates-v*` and publishes `obol-core` then `obol-cli` in dependency order. Manifest prep adds required/listing metadata and centralizes the `obol-core` path dep with a `version`.

**Tech Stack:** Rust 1.96 (mise), crates.io, GitHub Actions, `rust-lang/crates-io-auth-action` (OIDC).

**Spec:** `docs/specs/2026-06-06-obol-crates-publishing-design.md` (PRI-2097).

**Toolchain:** Rust via `mise exec rust@1.96.0 -- cargo …`.

---

## Task 1: Manifest metadata + dependency centralization

**Files:** Modify `Cargo.toml`, `crates/obol-core/Cargo.toml`, `crates/obol-cli/Cargo.toml`, `crates/obol-ffi/Cargo.toml`

- [ ] **Step 1: Add shared metadata + the versioned `obol-core` workspace dep to the root `Cargo.toml`**

In `[workspace.package]`, add `repository`/`homepage` after the existing `license` line:
```toml
[workspace.package]
edition = "2021"
version = "0.1.0"
license = "Apache-2.0"
repository = "https://github.com/prime-radiant-inc/obol"
homepage = "https://github.com/prime-radiant-inc/obol"
```
In `[workspace.dependencies]`, add `obol-core` (this is what gives the path dep a publishable version):
```toml
obol-core = { path = "crates/obol-core", version = "0.1.0" }
```

- [ ] **Step 2: Fill in `crates/obol-core/Cargo.toml` `[package]`**

Replace the `[package]` block with:
```toml
[package]
name = "obol-core"
edition.workspace = true
version.workspace = true
license.workspace = true
repository.workspace = true
homepage.workspace = true
description = "Read AI-agent transcripts (Claude Code, Codex, Pi) and estimate their USD cost."
readme = "README.md"
keywords = ["llm", "cost", "tokens", "ai", "transcript"]
categories = ["parsing", "development-tools"]
```

- [ ] **Step 3: Fill in `crates/obol-cli/Cargo.toml` `[package]` and swap the dep to the workspace dep**

Set the `[package]` block to:
```toml
[package]
name = "obol-cli"
edition.workspace = true
version.workspace = true
license.workspace = true
repository.workspace = true
homepage.workspace = true
description = "CLI to estimate the USD cost of an AI-agent transcript (Claude Code, Codex, Pi)."
readme = "README.md"
keywords = ["llm", "cost", "tokens", "cli", "transcript"]
categories = ["command-line-utilities"]
```
In `[dependencies]`, change `obol-core = { path = "../obol-core" }` to:
```toml
obol-core.workspace = true
```
In `[dev-dependencies]`, change `obol-core = { path = "../obol-core" }` to:
```toml
obol-core.workspace = true
```

- [ ] **Step 4: Swap obol-ffi's dep to the workspace dep**

In `crates/obol-ffi/Cargo.toml` `[dependencies]`, change `obol-core = { path = "../obol-core" }` to:
```toml
obol-core.workspace = true
```
(obol-ffi isn't published, but it must keep compiling against the centralized dep.)

- [ ] **Step 5: Verify the workspace still builds and obol-core packages clean**

Run: `mise exec rust@1.96.0 -- cargo build --workspace`
Expected: builds (the dep swap is transparent).

Run: `mise exec rust@1.96.0 -- cargo publish -p obol-core --dry-run --allow-dirty 2>&1 | tail -5`
Expected: packages + verifies with **no** "no description" error path failing — only the benign warning is acceptable; the package builds. (Description is registry-enforced; locally it warns.)

Run: `mise exec rust@1.96.0 -- cargo package --list -p obol-cli 2>&1 | grep -E 'README|Cargo.toml'`
Expected: lists `README.md` and `Cargo.toml.orig`/`Cargo.toml`. (Do **not** run `obol-cli --dry-run` — it correctly fails pre-bootstrap because obol-core isn't on crates.io yet.)

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml crates/obol-core/Cargo.toml crates/obol-cli/Cargo.toml crates/obol-ffi/Cargo.toml
git commit -m "build(crates): publishable metadata + centralized obol-core dep version (PRI-2097)"
```

---

## Task 2: Per-crate READMEs

**Files:** Create `crates/obol-core/README.md`, `crates/obol-cli/README.md`

- [ ] **Step 1: Create `crates/obol-core/README.md`**

```markdown
# obol-core

Read an AI-agent transcript and estimate what it cost. `obol-core` parses Claude Code, Codex, and
Pi transcripts and computes per-message USD cost, handling the accounting naive summers get wrong:
two-layer dedup, cache buckets, and price tiers.

```rust
use obol_core::{estimate_cost, Source};

let est = estimate_cost(Source::Path("session.jsonl".into()), None)?;
println!("{} USD", est.total_usd);
```

Pricing comes from LiteLLM (and OpenRouter for Pi); `refresh_pricing_tables` updates the on-disk
snapshot. Output is a typed `CostEstimate` carrying `unpriced_models` and `approximations` — not a
JSON blob.

Part of [obol](https://github.com/prime-radiant-inc/obol). Apache-2.0.
```

- [ ] **Step 2: Create `crates/obol-cli/README.md`**

```markdown
# obol-cli

Command-line tool to estimate the USD cost of an AI-agent transcript (Claude Code, Codex, Pi).

```bash
cargo install obol-cli      # installs the `obol` binary

obol estimate session.jsonl            # auto-detects the dialect
obol estimate session.jsonl --json     # machine-readable
obol refresh                           # update the on-disk pricing snapshot
```

Built on [`obol-core`](https://crates.io/crates/obol-core). Part of
[obol](https://github.com/prime-radiant-inc/obol). Apache-2.0.
```

- [ ] **Step 3: Confirm the READMEs are packaged**

Run: `mise exec rust@1.96.0 -- cargo package --list -p obol-core 2>&1 | grep README`
Expected: `README.md`.

- [ ] **Step 4: Commit**

```bash
git add crates/obol-core/README.md crates/obol-cli/README.md
git commit -m "docs(crates): per-crate READMEs for crates.io pages (PRI-2097)"
```

---

## Task 3: The `crates-release.yml` workflow (token phase)

**Files:** Create `.github/workflows/crates-release.yml`

- [ ] **Step 1: Create the workflow**

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
      - name: Guard — tag must match the Cargo version
        run: |
          want="${GITHUB_REF_NAME#crates-v}"
          got=$(mise exec rust@1.96.0 -- cargo metadata --no-deps --format-version 1 \
                | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="obol-core"))')
          if [ "$want" != "$got" ]; then
            echo "::error::tag $GITHUB_REF_NAME does not match Cargo version $got"; exit 1
          fi
          echo "publishing version $got"
      - name: Publish obol-core then obol-cli
        env:
          CARGO_REGISTRY_TOKEN: ${{ secrets.CARGO_REGISTRY_TOKEN }}
        run: |
          mise exec rust@1.96.0 -- cargo publish --locked -p obol-core
          mise exec rust@1.96.0 -- cargo publish --locked -p obol-cli
```

- [ ] **Step 2: Lint the workflow**

Run: `/tmp/gobin/actionlint .github/workflows/crates-release.yml` (install if missing: `GOBIN=/tmp/gobin go install github.com/rhysd/actionlint/cmd/actionlint@latest`)
Expected: clean.

- [ ] **Step 3: Sanity-check the guard extraction locally**

Run: `mise exec rust@1.96.0 -- cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(next(p["version"] for p in json.load(sys.stdin)["packages"] if p["name"]=="obol-core"))'`
Expected: `0.1.0` (matches what a `crates-v0.1.0` tag would assert).

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/crates-release.yml
git commit -m "ci(crates): crates-release.yml — token-phase publish on crates-v* (PRI-2097)"
```

---

## Task 4: Documentation + Linear (prep complete; hand off to bootstrap)

**Files:** Modify `docs/RELEASING.md`; update memory; Linear.

- [ ] **Step 1: Add the crates section to `docs/RELEASING.md`** (before `## Other registries`; drop crates from that section's "not yet wired" list)

```markdown
## crates.io — `obol-core` + `obol-cli`

Decoupled from the npm/Go `v*` tags: crates take their version from `Cargo.toml`, so they publish on
their own **`crates-v*`** tag namespace via `.github/workflows/crates-release.yml`. A guard asserts
`crates-vX.Y.Z` matches the workspace version; the workflow publishes `obol-core` then `obol-cli`
(cargo ≥1.66 waits for the index between them).

**Bootstrap (one-time, token):** crates.io Trusted Publishing requires the crate to exist first.
1. Create a crates.io API token (scopes `publish-new` + `publish-update`); add it as the repo secret
   `CARGO_REGISTRY_TOKEN`.
2. Push `crates-v0.1.0` → publishes `obol-core` + `obol-cli` at `0.1.0` (creates the crates).
3. On crates.io, configure **Trusted Publishing** on each crate (Settings → Trusted Publishing →
   GitHub Actions, repo `prime-radiant-inc/obol`, workflow `crates-release.yml`). Remove
   `CARGO_REGISTRY_TOKEN`.
4. Switch the workflow to OIDC (see below) — every release after is tokenless.

**Steady state (OIDC):** the workflow mints a short-lived token via
`rust-lang/crates-io-auth-action@v1` (needs `permissions: id-token: write`). Bump
`[workspace.package] version` *and* `[workspace.dependencies] obol-core` version together, update the
two binding version tests, then push `crates-vX.Y.Z`.
```

- [ ] **Step 2: Verify the workspace builds + the full test suite still passes (nothing regressed)**

Run: `mise exec rust@1.96.0 -- cargo test --workspace -- --test-threads=1`
Expected: PASS (the dep/metadata changes don't touch behavior).

- [ ] **Step 3: Commit**

```bash
git add docs/RELEASING.md
git commit -m "docs: crates.io releasing runbook (PRI-2097)"
```

- [ ] **Step 4: Update memory + move PRI-2097 to In Review**

Add a PRI-2097 paragraph to `project_obol.md` (crates prep landed; bootstrap pending Matt's token), and a reflective `save_comment` on PRI-2097. State stays prep-complete; the bootstrap publish is gated on Matt adding `CARGO_REGISTRY_TOKEN`.

---

## Phase 2 (operational — after Matt configures Trusted Publishing)

Not code tasks for the prep run; executed once the crates exist on crates.io.

- [ ] **Switch `crates-release.yml` to OIDC:** drop the `CARGO_REGISTRY_TOKEN` env, add `permissions: { id-token: write }` at job level, insert `- uses: rust-lang/crates-io-auth-action@v1` (id `auth`) before the publish step, and set `env: { CARGO_REGISTRY_TOKEN: ${{ steps.auth.outputs.token }} }`. Commit.
- [ ] **Align at 0.1.1 (four files):** `[workspace.package] version = "0.1.1"`, `[workspace.dependencies] obol-core` `version = "0.1.1"`, `bindings/python/tests/test_obol.py` → `"0.1.1"`, `bindings/typescript/test/obol.test.ts` → `"0.1.1"`. Optionally sweep prose `0.1.0` in `docs/RELEASING.md` + Python/Go READMEs. Commit.
- [ ] **Publish:** push `crates-v0.1.1` → OIDC publish. Verify `cargo add obol-core@0.1.1` + `cargo install obol-cli` in a scratch dir, and that npm/Go/crates + `Version()` now all read `0.1.1`.

---

## Self-Review

**Spec coverage:** metadata + readme + categories/keywords (T1/T2) ✓; centralized versioned dep (T1) ✓; obol-ffi dep swap (T1) ✓; `crates-release.yml` token phase + guard + `--locked` + dep-order publish (T3) ✓; runbook (T4) ✓; OIDC switch + 0.1.1 four-file alignment incl. the two binding tests (Phase 2) ✓; decoupled `crates-v*` namespace ✓.

**Placeholder scan:** no TBD/TODO; metadata/READMEs/workflow are complete literals; commands have expected output.

**Type/consistency:** `obol-core.workspace = true` used uniformly (obol-cli deps + dev-deps, obol-ffi); the `[workspace.dependencies] obol-core` `version` (`0.1.0`) equals `[workspace.package] version` (`0.1.0`) — the invariant the guard enforces; the Phase-2 bump moves both together to `0.1.1` plus the two binding-test literals (the B1 fix).
