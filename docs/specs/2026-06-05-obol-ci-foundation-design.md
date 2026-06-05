# obol — CI foundation (design spec)

> 2026-06-05 · Shevek@7998e83e · draft for Bob review · Linear PRI-2089
> First step toward packaging. Scope (decided with Matt): **CI foundation only** — build the
> cdylib and run the full test + five-language equivalence matrix on every push/PR, across a
> platform matrix. Publishing (PyPI/npm/Go release) is deferred to follow-on tickets.

## Goal

Turn the verification I've been running by hand (and in one-off containers) into a standing
GitHub Actions gate: every push to `main` and every PR builds `obol-ffi` and runs the Rust
tests + clippy + fmt, the Python/Go/TypeScript(Bun+Node) binding suites, and the five-language
equivalence gate (`rust == python == go == ts(bun) == ts(node)`), on **macOS arm64 + x64** and
**Linux x64 + arm64 (glibc)**. Green across all four = the whole stack holds on both arches of
both OSes — and it finally closes the x86-64 coverage gap (only arm64 was verified so far).

## Key simplification: native per runner, no cross-compilation

GitHub hosts all four target runner types natively, so each runner builds and tests on its own
architecture. There is **no cross-compilation** — which sidesteps the `ring`/`rustls`
cross-compile pain entirely (the one real toolchain risk for this kind of work). The matrix:

| runner label      | target        |
|-------------------|---------------|
| `macos-14`        | macOS arm64   |
| `macos-13`        | macOS x64 (Intel) |
| `ubuntu-24.04`    | Linux x64     |
| `ubuntu-24.04-arm`| Linux arm64   |

**To confirm during build, not assume:** that `ubuntu-24.04-arm` hosted runners are enabled for
the org. They are GA and free for public repos, but if the job can't be scheduled, the fallback
is a QEMU-emulated arm64 leg (slower) — documented, not blocking.

## Workflow shape

One workflow, `.github/workflows/ci.yml`, triggered on `push` to `main` and `pull_request`
(the safe `pull_request`, **not** `pull_request_target`). Top-level:

```yaml
permissions:
  contents: read          # this workflow only reads the repo
concurrency:
  group: ci-${{ github.ref }}
  cancel-in-progress: true # superseded PR pushes cancel their stale ×4 matrix (incl. macOS)
```

Two jobs:

### Job `lint` (single runner, `ubuntu-24.04`)

After `actions/checkout@v4` + `jdx/mise-action@v2`, platform-independent checks run once (not ×4):
- `cargo fmt --check` (the tree must be fmt-clean — see "Pre-work" below; it isn't today).
- `cargo clippy --all-targets --workspace -- -D warnings`.

The cbindgen header-drift test (`header_matches_source`) already runs inside `cargo test`, so it
is covered by the `test` job per platform — no separate step.

### Job `test` (matrix ×4)

Each runner runs the complete pipeline, in order (the cdylib must exist before any binding test
loads it):

0. **`actions/checkout@v4`** — first step of *both* jobs (lint and every test leg). Without it
   the jobs check out nothing and do nothing.
1. **Toolchains:**
   - Rust 1.96 via `jdx/mise-action@v2` (reads `mise.toml`), so the existing
     `mise exec rust@1.96.0 -- cargo …` in the repo's scripts works unchanged.
   - Node 24 via `actions/setup-node@v4` (`node-version: 24` ⇒ native TS type-stripping, no flag;
     do not drop below 22.18).
   - Bun via `oven-sh/setup-bun@v2`, pinned `bun-version: 1.3.11` (matches local dev, so a Bun
     point release can't shift `bun:ffi` behavior under us).
   - Go via `actions/setup-go@v5`, `go-version: '1.23'`, **`cache: false`** — there is no
     `go.sum` (the Go binding has zero external deps), and setup-go's default caching errors with
     no checksum file to hash.
   - Python via `actions/setup-python@v5` (`python-version: '3.12'`), then
     `python -m pip install pytest`.
   - `Swatinem/rust-cache@v2` after mise-action installs the toolchain and before the build.
2. **Build the cdylib:** `mise exec rust@1.96.0 -- cargo build -p obol-ffi` (produces
   `target/debug/libobol_ffi.{dylib,so}`, which the bindings discover via their `target/debug`
   fallback / baked rpath).
3. **Rust:** `mise exec rust@1.96.0 -- cargo test --workspace -- --test-threads=1` (the
   `--test-threads=1` is mandatory — several tests mutate the global `OBOL_PRICING_DIR`).
4. **Python:** `cd bindings/python && PYTHONPATH=. python -m pytest tests -q` (5 tests).
5. **Go:** `cd bindings/go && CGO_ENABLED=1 go test ./...` (cgo needs a C compiler — present on
   all four runners: clang on macOS, gcc on ubuntu). **On Linux, prefix with
   `LD_LIBRARY_PATH="$GITHUB_WORKSPACE/target/debug"`.** The cgo binding bakes an rpath
   (`-Wl,-rpath,…/target/debug`) which on macOS resolves the `.dylib` at runtime, and on Linux
   *should* resolve the `.so` via `DT_RUNPATH` — but Go reads neither `OBOL_LIB` nor
   `LD_LIBRARY_PATH` on its own, so setting `LD_LIBRARY_PATH` explicitly on the Linux legs is a
   zero-risk guarantee against any `--enable-new-dtags`/RUNPATH edge. macOS legs need nothing
   extra (rpath suffices).
6. **TypeScript:** `cd bindings/typescript && bun install` then `bun test` **and**
   `node --test test/obol.test.ts` (the same `node:test` file under both runtimes). (Note: the
   gate in step 7 also runs `bun install` — harmless, cached.)
7. **Five-language equivalence gate:** `./scripts/validate_bindings.sh` — the acceptance check,
   asserting `rust == python == go == ts(bun) == ts(node)` for the fixture. It rebuilds
   `obol-ffi`+`obol-cli` (cached/no-op after step 2) and runs all five consumers. **Run with
   `LD_LIBRARY_PATH="$GITHUB_WORKSPACE/target/debug"` on Linux** for the same Go reason (the
   gate invokes the Go `cmd/total`); the script already exports `OBOL_LIB` for Python/TS.

### Caching

- `Swatinem/rust-cache@v2` keyed per-runner (caches the cargo registry + `target/`), so repeat
  runs are fast and don't recompile `ring`/`clap`/etc. each time.
- `setup-node`/`setup-go`/`setup-python` provide their own dependency caches; the TS `bun
  install` is small (koffi prebuild).

## Pre-work (in this ticket, before CI can go green)

- **`cargo fmt` the tree once.** It is not fmt-clean today (`crates/obol-cli/src/main.rs` has
  several spots), so `cargo fmt --check` would fail on day one. Run `cargo fmt`, commit the
  result, so the `lint` job passes. This is a no-behavior-change formatting commit.
- Nothing else: clippy is already clean, all suites already pass locally, and the gate is green.

## README

Add a CI **status badge** at the top:
`![CI](https://github.com/prime-radiant-inc/obol/actions/workflows/ci.yml/badge.svg)`.

## Verification (how we know the workflow works)

A CI workflow can't be fully exercised locally, so the proof is on GitHub itself:

1. `actionlint` the YAML locally first (catches syntax/expression errors cheaply).
2. Push the branch and open a PR; `gh run watch` the run.
3. Iterate on the YAML until **all four `test` legs + `lint` are green**.
4. Only then merge. The merged state is a `main` with a passing required check.

This is a legitimate verify loop — the live Actions run is the test, observed via `gh`.

## Risks / notes

- **`ubuntu-24.04-arm` availability** — GA since 2025-08-07 and **free for public repos**, so it
  will schedule (obol being public is load-bearing here — arm64 Linux labels fail in *private*
  repos on free plans). QEMU fallback documented if ever needed.
- **koffi prebuilds (risk retired)** — koffi 2.16.2 ships prebuilt binaries for `darwin_arm64`,
  `darwin_x64`, `linux_arm64`, `linux_x64`, so `npm`/`bun install` never builds from source or
  fetches at runtime on any of the four legs. This was the biggest silent-failure candidate and
  it's clean.
- **macOS runner minutes** — macOS runners are the slow/expensive leg; for a public repo Actions
  minutes are free, so acceptable. Native (not emulated) keeps them reasonable.
- **`macos-13` (Intel) longevity** — GitHub will eventually retire Intel macOS runners; when that
  happens, macOS-x64 coverage moves to cross-compile or is dropped. Fine for now; noted.
- **Flake surface** — the suites are deterministic (seeded pricing fixture, no network in tests;
  `refresh`'s network path is not exercised by the gate). Low flake risk.

## Out of scope (this cut)

Publishing / release automation (PyPI wheels, npm publish, Go module tags), Windows, Linux musl/
Alpine, self-hosted runners, release artifacts/uploads, branch-protection config (Matt can flip
the required-check toggle in repo settings once the workflow is green).

## Open threads (small)

None blocking. Versions are pinned above (checkout@v4, mise-action@v2, setup-node@v4/node 24,
setup-bun@v2/bun 1.3.11, setup-go@v5/go 1.23 + `cache:false`, setup-python@v5/3.12,
rust-cache@v2). The only genuine confirm-at-runtime item is `ubuntu-24.04-arm` scheduling (see
Risks). `lint` runs once, not in the matrix (clippy lints here are platform-independent).

> Resolved during spec review (Bender@a186bff3, verified against the live repo/scripts): runner
> labels all current; `LD_LIBRARY_PATH` added on the Linux Go/gate legs (Go uses only the baked
> rpath); `actions/checkout` made explicit; `setup-go` `cache:false` (no `go.sum`); koffi
> prebuilds cover all four legs; fmt pre-work confirmed (only `obol-cli/src/main.rs`); clippy
> already clean; `--test-threads=1` genuinely required.
