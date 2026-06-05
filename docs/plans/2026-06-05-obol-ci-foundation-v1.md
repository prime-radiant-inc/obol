# obol CI foundation — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans (inline) — the
> verify loop is watching live GitHub Actions runs, which the orchestrator drives. Steps use
> checkbox (`- [ ]`) syntax.

**Goal:** A GitHub Actions workflow that builds `obol-ffi` and runs the full test + five-language
equivalence matrix on every push to `main` and every PR, across macOS arm64+x64 and Linux
x64+arm64 (glibc), natively per runner.

**Architecture:** One workflow `.github/workflows/ci.yml` with a `lint` job (fmt + clippy, once)
and a `test` job (matrix ×4 native runners) running the exact verification done by hand. No
cross-compilation. Spec: `docs/specs/2026-06-05-obol-ci-foundation-design.md`.

**Tech stack:** GitHub Actions (hosted runners), `jdx/mise-action` (Rust 1.96), `setup-node@24`,
`oven-sh/setup-bun@1.3.11`, `setup-go@1.23`, `setup-python@3.12`, `Swatinem/rust-cache`.

---

## Task 1: Format the tree (CI pre-work)

The `lint` job runs `cargo fmt --check`, which fails today (`crates/obol-cli/src/main.rs`). Make
the tree fmt-clean first — a pure formatting change, no behavior.

**Files:**
- Modify: whatever `cargo fmt` touches (currently `crates/obol-cli/src/main.rs`).

- [ ] **Step 1: Format.** `mise exec rust@1.96.0 -- cargo fmt`
- [ ] **Step 2: Confirm clean.** `mise exec rust@1.96.0 -- cargo fmt --check` → no output, exit 0.
- [ ] **Step 3: Confirm no behavior change** — tests still pass:
  `mise exec rust@1.96.0 -- cargo test --workspace -- --test-threads=1` → all green.
- [ ] **Step 4: Commit.**

```bash
git add -A
git commit -m "style: cargo fmt the tree (CI fmt gate pre-work)"
```

---

## Task 2: The CI workflow

**Files:**
- Create: `.github/workflows/ci.yml`

- [ ] **Step 1: Write `.github/workflows/ci.yml`** with exactly this content:

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:

permissions:
  contents: read

concurrency:
  group: ci-${{ github.ref }}
  cancel-in-progress: true

jobs:
  lint:
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@v4
      - uses: jdx/mise-action@v4
      - name: rustfmt
        run: mise exec rust@1.96.0 -- cargo fmt --check
      - name: clippy
        run: mise exec rust@1.96.0 -- cargo clippy --all-targets --workspace -- -D warnings

  test:
    name: test (${{ matrix.name }})
    runs-on: ${{ matrix.os }}
    strategy:
      fail-fast: false
      matrix:
        include:
          - { os: macos-14,         name: macos-arm64 }
          - { os: macos-15-intel,   name: macos-x64 }
          - { os: ubuntu-24.04,     name: linux-x64 }
          - { os: ubuntu-24.04-arm, name: linux-arm64 }
    steps:
      - uses: actions/checkout@v4

      - uses: jdx/mise-action@v4
      - uses: Swatinem/rust-cache@v2
      - uses: actions/setup-node@v4
        with:
          node-version: "24"
      - uses: oven-sh/setup-bun@v2
        with:
          bun-version: "1.3.11"
      - uses: actions/setup-go@v5
        with:
          go-version: "1.23"
          cache: false
      - uses: actions/setup-python@v5
        with:
          python-version: "3.12"
      - name: Install pytest
        run: python -m pip install --quiet pytest

      - name: Build cdylib
        run: mise exec rust@1.96.0 -- cargo build -p obol-ffi

      - name: Rust tests
        run: mise exec rust@1.96.0 -- cargo test --workspace -- --test-threads=1

      - name: Python binding tests
        working-directory: bindings/python
        run: PYTHONPATH=. python -m pytest tests -q

      - name: Go binding tests
        working-directory: bindings/go
        env:
          CGO_ENABLED: "1"
        run: |
          if [ "$RUNNER_OS" = "Linux" ]; then
            export LD_LIBRARY_PATH="$GITHUB_WORKSPACE/target/debug"
          fi
          go test ./...

      - name: TypeScript binding tests (Bun + Node)
        working-directory: bindings/typescript
        run: |
          bun install
          bun test
          node --test test/obol.test.ts

      - name: Five-language equivalence gate
        run: |
          if [ "$RUNNER_OS" = "Linux" ]; then
            export LD_LIBRARY_PATH="$GITHUB_WORKSPACE/target/debug"
          fi
          ./scripts/validate_bindings.sh
```

- [ ] **Step 2: Lint the YAML locally** if `actionlint` is available
  (`brew install actionlint` or `go install github.com/rhysd/actionlint/cmd/actionlint@latest`):
  `actionlint .github/workflows/ci.yml` → no errors. If actionlint isn't installed, skip — the
  push (Task-after) is the real check.
- [ ] **Step 3: Commit.**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: build + full test/5-language equivalence matrix (macOS+Linux, both arches)"
```

---

## Task 3: README status badge

**Files:**
- Modify: `README.md` (top, under the `# obol` title)

- [ ] **Step 1: Add the badge** right after the `# obol` heading line:

```markdown
# obol

[![CI](https://github.com/prime-radiant-inc/obol/actions/workflows/ci.yml/badge.svg)](https://github.com/prime-radiant-inc/obol/actions/workflows/ci.yml)
```

- [ ] **Step 2: Commit.**

```bash
git add README.md
git commit -m "docs: CI status badge"
```

---

## Execution: push, watch, iterate (orchestrator, inline)

The workflow can only be truly tested on GitHub. Drive the loop:

- [ ] **Push the branch:** `git push -u origin matt/pri-2089-ci-foundation`
- [ ] **Open a PR** (so `pull_request` triggers the matrix) — or rely on the push trigger if
  pushing toward `main`. `gh pr create --fill` then `gh run watch` (or
  `gh run list --branch matt/pri-2089-ci-foundation`).
- [ ] **Watch all five legs** (`lint` + `test ×4`). Iterate on `ci.yml` for any red leg and
  re-push. Likely first-run watch-items (from the spec/review):
  - `ubuntu-24.04-arm` schedules (GA + public repo — expected fine).
  - `mise`-installed Rust includes `rustfmt` + `clippy` (works locally; if a runner's mise install
    is minimal, add a `mise exec rust@1.96.0 -- rustup component add rustfmt clippy` step or
    equivalent — but expected fine since it matches local).
  - `Swatinem/rust-cache` finds `rustc` on PATH after mise-action (expected; mise activates).
  - The Linux `LD_LIBRARY_PATH` makes the Go leg + gate resolve `libobol_ffi.so`.
- [ ] **Green ×5** → done. (Branch-protection / required-check is a repo-settings toggle Matt
  flips later; out of scope for the YAML.)

---

## Self-review notes (plan author)

- **Spec coverage:** lint job (fmt+clippy) = Task 2; test matrix ×4 with the full pipeline = Task
  2; fmt pre-work = Task 1; badge = Task 3; concurrency/permissions/checkout/cache/versions/
  LD_LIBRARY_PATH all in the YAML per the reviewed spec. Verification loop = Execution section.
- **Placeholder scan:** the YAML is complete and literal; no TBDs. The only conditional guidance
  (rustfmt/clippy component, rust-cache rustc) is framed as first-run watch-items with the exact
  remediation, not a hand-wave — these are genuinely runtime-confirm items for a CI workflow.
- **Consistency:** runner labels, action versions, and step order match the spec exactly; the
  `LD_LIBRARY_PATH` guard appears on both the Go leg and the gate (the two Go-touching steps),
  Linux-only.
