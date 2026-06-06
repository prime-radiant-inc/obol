# obol — PyPI publishing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: superpowers:subagent-driven-development or superpowers:executing-plans. Checkbox (`- [ ]`) steps.

**Goal:** Publish the Python binding to PyPI as `primeradianthq-obol` (import `obol`) — four per-platform wheels bundling the prebuilt dylib, tokenless OIDC, first release `pypi-v0.1.1`.

**Architecture:** Pure-Python ctypes package + a platform-specific prebuilt `libobol_ffi` bundled in each wheel. A `setup.py` forces an impure `py3-none-<plat>` wheel; macOS legs build on the runner, Linux legs build the `.so` + wheel inside a `manylinux_2_28` container (via `docker run` from the host) and `auditwheel repair`. The existing loader finds the bundled lib — no loader change.

**Tech Stack:** setuptools/wheel/build, auditwheel, `manylinux_2_28` images, `pypa/gh-action-pypi-publish` (OIDC), Rust 1.96.

**Spec:** `docs/specs/2026-06-06-obol-pypi-publishing-design.md` (PRI-2098). Verified hands-on by Brunelleschi (wheels + auditwheel in a real manylinux container).

---

## Task 1: Packaging files

**Files:** Create `bindings/python/pyproject.toml`, `bindings/python/setup.py`; modify `bindings/python/README.md`, `.gitignore`.

- [ ] **Step 1: Create `bindings/python/pyproject.toml`**

```toml
[build-system]
requires = ["setuptools>=64", "wheel"]
build-backend = "setuptools.build_meta"

[project]
name = "primeradianthq-obol"
version = "0.0.0"
description = "Estimate the USD cost of an AI-agent transcript (Claude Code, Codex, Pi)."
readme = "README.md"
requires-python = ">=3.9"
license = { text = "Apache-2.0" }
authors = [{ name = "Prime Radiant, Inc." }]
keywords = ["llm", "cost", "tokens", "transcript", "ai-agents"]
classifiers = [
  "License :: OSI Approved :: Apache Software License",
  "Programming Language :: Python :: 3",
  "Programming Language :: Rust",
  "Topic :: Software Development :: Libraries",
]

[project.urls]
Homepage = "https://github.com/prime-radiant-inc/obol"
Repository = "https://github.com/prime-radiant-inc/obol"

[tool.setuptools]
packages = ["obol"]

[tool.setuptools.package-data]
obol = ["libobol_ffi.dylib", "libobol_ffi.so"]
```

- [ ] **Step 2: Create `bindings/python/setup.py`**

```python
# The package is pure-Python ctypes but carries a platform-specific prebuilt dylib, so the
# wheel must be impure + platform-tagged, yet abi-agnostic (py3-none-<plat>). BinaryDistribution
# (has_ext_modules → True) keeps the files at the wheel root; get_tag forces py3/none and takes
# the platform from OBOL_WHEEL_PLAT so each CI leg stamps its exact arch.
import os
from setuptools import setup
from setuptools.dist import Distribution

try:  # modern path first (wheel >=0.46 deprecates its own bdist_wheel shim)
    from setuptools.command.bdist_wheel import bdist_wheel as _bdist_wheel
except ImportError:
    from wheel.bdist_wheel import bdist_wheel as _bdist_wheel


class BinaryDistribution(Distribution):
    def has_ext_modules(self):
        return True


class bdist_wheel(_bdist_wheel):
    def get_tag(self):
        _py, _abi, plat = super().get_tag()
        return ("py3", "none", os.environ.get("OBOL_WHEEL_PLAT", plat))


setup(distclass=BinaryDistribution, cmdclass={"bdist_wheel": bdist_wheel})
```

- [ ] **Step 3: Add a PyPI-user lead to `bindings/python/README.md`**

Insert directly under the `# obol — Python binding` title (keep the existing dev/build content below):

```markdown
> **Installing from PyPI:** `pip install primeradianthq-obol`, then `import obol`. The native
> library is bundled in the wheel — no Rust toolchain, no `cargo build`, no `OBOL_LIB`. The
> sections below are for in-repo development.
```

- [ ] **Step 4: Ignore build artifacts**

Append to `.gitignore`:
```
/bindings/python/build/
/bindings/python/dist/
/bindings/python/*.egg-info/
/bindings/python/obol/libobol_ffi.*
```

- [ ] **Step 5: Build a wheel locally end-to-end (macOS arm64) and verify install + estimate**

Run:
```bash
mise exec rust@1.96.0 -- cargo build --release -p obol-ffi
cp target/release/libobol_ffi.dylib bindings/python/obol/
python3 - <<'PY'
import re, pathlib
p = pathlib.Path("bindings/python/pyproject.toml")
p.write_text(re.sub(r'^version = "0.0.0"', 'version = "0.1.1"', p.read_text(), flags=re.M))
PY
cd bindings/python && python3 -m venv /tmp/bw && /tmp/bw/bin/pip install -q build && \
  OBOL_WHEEL_PLAT=macosx_11_0_arm64 MACOSX_DEPLOYMENT_TARGET=11.0 /tmp/bw/bin/python -m build --wheel
ls dist/
```
Expected: `primeradianthq_obol-0.1.1-py3-none-macosx_11_0_arm64.whl` (note the **arm64** tag, not universal2).

Run (install into a clean venv + real estimate against the fixture):
```bash
python3 -m venv /tmp/iw && /tmp/iw/bin/pip install /Users/mw/Code/prime/obol/bindings/python/dist/*.whl
SEED=$(mktemp -d); cp bindings/testdata/prices.json "$SEED/current.json"
OBOL_PRICING_DIR="$SEED" /tmp/iw/bin/python -c "import obol; print(obol.version()); print(obol.estimate_path('$(pwd)/bindings/testdata/claude-mini.jsonl', dialect='claude').total_usd)"
```
Expected: `0.1.1` then `0.000995` — the bundled dylib loaded with no `OBOL_LIB`.

- [ ] **Step 6: Restore the working tree (the version stamp + copied dylib are build-time only)**

Run: `git checkout bindings/python/pyproject.toml; rm -f bindings/python/obol/libobol_ffi.*`
Expected: `git status` shows only the new `pyproject.toml`/`setup.py`, README, `.gitignore` (pyproject back to `0.0.0`).

- [ ] **Step 7: Commit**

```bash
git add bindings/python/pyproject.toml bindings/python/setup.py bindings/python/README.md .gitignore
git commit -m "build(pypi): packaging for primeradianthq-obol (platform wheels, import obol) (PRI-2098)"
```

---

## Task 2: Linux manylinux build script

**Files:** Create `scripts/build-pypi-wheel-linux.sh`

- [ ] **Step 1: Create the script** (runs *inside* a `manylinux_2_28` container; the repo is mounted at `/io`)

```bash
#!/usr/bin/env bash
# Build a manylinux wheel for the Python binding, INSIDE a manylinux_2_28 container.
# Usage (from host): docker run --rm -v "$PWD:/io" -w /io <image> bash scripts/build-pypi-wheel-linux.sh <version> <arch>
#   <version>  e.g. 0.1.1   <arch>  x86_64 | aarch64
set -euxo pipefail
VERSION="$1"; ARCH="$2"
PY=/opt/python/cp311-cp311/bin/python

# Rust toolchain (manylinux images have no rust).
curl -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.96.0 --profile minimal
. "$HOME/.cargo/env"

# Build the cdylib against the container's glibc 2.28 → manylinux_2_28-compatible.
cargo build --release -p obol-ffi
strip target/release/libobol_ffi.so
cp target/release/libobol_ffi.so bindings/python/obol/libobol_ffi.so

sed -i "s/^version = \"0.0.0\"/version = \"$VERSION\"/" bindings/python/pyproject.toml

cd bindings/python
"$PY" -m pip install --quiet --upgrade build
"$PY" -m build --wheel                                  # → dist/*-linux_<arch>.whl
auditwheel repair --plat "manylinux_2_28_$ARCH" -w /io/wheelhouse dist/*.whl

# Smoke: install the repaired wheel in a clean venv and assert the exact version.
"$PY" -m venv /tmp/venv
/tmp/venv/bin/pip install --quiet /io/wheelhouse/*.whl
/tmp/venv/bin/python -c "import obol; v=obol.version(); assert v=='$VERSION', v; print('loaded', v)"
```

- [ ] **Step 2: Make it executable**

Run: `chmod +x scripts/build-pypi-wheel-linux.sh`

- [ ] **Step 3: Verify it in a real manylinux container (the Linux-container-verify standard)**

Run:
```bash
docker run --rm -v "$(pwd):/io" -w /io quay.io/pypa/manylinux_2_28_$(uname -m | sed 's/arm64/aarch64/') \
  bash scripts/build-pypi-wheel-linux.sh 0.1.1 $(uname -m | sed 's/arm64/aarch64/') 2>&1 | tail -20
ls wheelhouse/
```
Expected: ends with `loaded 0.1.1`; `wheelhouse/` holds `primeradianthq_obol-0.1.1-py3-none-manylinux_2_28_aarch64.whl` (host is arm64). The `auditwheel repair` retags `linux_*` → `manylinux_2_28_*` and the smoke import passes inside the container.
(If the sandbox blocks docker/network, re-run with `dangerouslyDisableSandbox: true`.)

- [ ] **Step 4: Restore the tree + commit the script**

Run: `git checkout bindings/python/pyproject.toml; rm -rf wheelhouse bindings/python/obol/libobol_ffi.* bindings/python/dist bindings/python/build`

```bash
git add scripts/build-pypi-wheel-linux.sh
git commit -m "build(pypi): manylinux_2_28 wheel build script (PRI-2098)"
```

---

## Task 3: The `pypi-release.yml` workflow

**Files:** Create `.github/workflows/pypi-release.yml`

- [ ] **Step 1: Create the workflow**

```yaml
name: PyPI Release

on:
  push:
    tags: ["pypi-v*"]

jobs:
  wheels-macos:
    strategy:
      fail-fast: false
      matrix:
        include:
          - { os: macos-14,        plat: macosx_11_0_arm64,  target: "11.0" }
          - { os: macos-15-intel,  plat: macosx_10_12_x86_64, target: "10.12" }
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: jdx/mise-action@v4
      - name: Build wheel
        env:
          OBOL_WHEEL_PLAT: ${{ matrix.plat }}
          MACOSX_DEPLOYMENT_TARGET: ${{ matrix.target }}
        run: |
          VERSION="${GITHUB_REF_NAME#pypi-v}"
          mise exec rust@1.96.0 -- cargo build --release -p obol-ffi
          strip -x target/release/libobol_ffi.dylib
          cp target/release/libobol_ffi.dylib bindings/python/obol/
          sed -i '' "s/^version = \"0.0.0\"/version = \"$VERSION\"/" bindings/python/pyproject.toml
          cd bindings/python
          python3 -m venv /tmp/bw && /tmp/bw/bin/pip install --quiet build
          /tmp/bw/bin/python -m build --wheel
          python3 -m venv /tmp/iw && /tmp/iw/bin/pip install dist/*.whl
          /tmp/iw/bin/python -c "import obol; v=obol.version(); assert v=='$VERSION', v"
      - uses: actions/upload-artifact@v4
        with:
          name: wheel-${{ matrix.plat }}
          path: bindings/python/dist/*.whl
          if-no-files-found: error

  wheels-linux:
    strategy:
      fail-fast: false
      matrix:
        include:
          - { os: ubuntu-24.04,     arch: x86_64 }
          - { os: ubuntu-24.04-arm, arch: aarch64 }
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - name: Build manylinux wheel
        run: |
          VERSION="${GITHUB_REF_NAME#pypi-v}"
          docker run --rm -v "$PWD:/io" -w /io \
            quay.io/pypa/manylinux_2_28_${{ matrix.arch }} \
            bash scripts/build-pypi-wheel-linux.sh "$VERSION" ${{ matrix.arch }}
      - uses: actions/upload-artifact@v4
        with:
          name: wheel-manylinux-${{ matrix.arch }}
          path: wheelhouse/*.whl
          if-no-files-found: error

  publish:
    needs: [wheels-macos, wheels-linux]
    runs-on: ubuntu-24.04
    permissions:
      id-token: write # OIDC for PyPI Trusted Publishing (pending publisher)
    steps:
      - uses: actions/download-artifact@v4
        with:
          path: dist
          pattern: wheel-*
          merge-multiple: true
      - name: List + check
        run: |
          ls -l dist
          pipx run twine check dist/*.whl
      - uses: pypa/gh-action-pypi-publish@release/v1
        # No token, no repository-url (defaults to PyPI), no environment — matches the
        # pending publisher (project primeradianthq-obol, repo prime-radiant-inc/obol,
        # workflow pypi-release.yml, no environment).
```

- [ ] **Step 2: Lint**

Run: `/tmp/gobin/actionlint .github/workflows/pypi-release.yml`
Expected: clean. (Install actionlint if missing: `GOBIN=/tmp/gobin go install github.com/rhysd/actionlint/cmd/actionlint@latest`.)

- [ ] **Step 3: Commit**

```bash
git add .github/workflows/pypi-release.yml
git commit -m "ci(pypi): pypi-release.yml — 4 platform wheels + OIDC publish on pypi-v* (PRI-2098)"
```

---

## Task 4: Runbook + memory + Linear

**Files:** Modify `docs/RELEASING.md`; update memory; Linear.

- [ ] **Step 1: Add the PyPI section to `docs/RELEASING.md`** (replace the `## Other registries` stub)

```markdown
## PyPI — `primeradianthq-obol` (import `obol`)

Decoupled `pypi-v*` tag namespace (version from the tag, like crates). `pypi-vX.Y.Z` →
`.github/workflows/pypi-release.yml` builds four wheels — macOS arm64/x64 on the runners, Linux
x64/arm64 in `manylinux_2_28` containers (`scripts/build-pypi-wheel-linux.sh` + `auditwheel`) — each
bundling that platform's prebuilt `libobol_ffi`, then publishes via **tokenless OIDC**
(`pypa/gh-action-pypi-publish`).

- **Trusted Publishing (pending publisher, no token ever).** Configured once on PyPI: project
  `primeradianthq-obol`, owner `prime-radiant-inc`, repo `obol`, workflow `pypi-release.yml`, **no
  environment** (the workflow must stay environment-less to match). PyPI's pending-publisher flow
  creates the project on the first publish — no bootstrap, unlike crates/npm.
- **Wheels only** (no sdist — a source build needs Rust + the cdylib). `import obol` stays; the
  distribution name `primeradianthq-obol` differs because bare `obol` is a taken PyPI name.
- To release: push `pypi-vX.Y.Z` (the version is stamped from the tag into `pyproject.toml`).
```

- [ ] **Step 2: Commit**

```bash
git add docs/RELEASING.md
git commit -m "docs: PyPI releasing runbook (PRI-2098)"
```

- [ ] **Step 3: Update memory + move PRI-2098 to In Review**

Add a PRI-2098 paragraph to `project_obol.md` (PyPI packaging + `pypi-release.yml` landed; publish pending Matt's pending-publisher confirmation + the `pypi-v0.1.1` tag) and a reflective `save_comment`. Update the `MEMORY.md` index (PyPI prep done; all four channels now wired). State: prep complete, publish gated on the tag.

---

## Phase 2 (operational — after the pending publisher is confirmed)

Not a code task. Once Matt confirms the PyPI pending publisher is saved:
- [ ] Merge to `main`, then push `pypi-v0.1.1` → the workflow builds the 4 wheels and OIDC-publishes.
- [ ] Watch the run (first real OIDC publish + first manylinux CI build — the bits to eyeball).
- [ ] Verify: `pip install primeradianthq-obol` in a clean venv on macOS, `import obol`,
      `obol.estimate_path(...)` → `0.000995`; confirm the installed wheel's arch tag matches the host.
      All four channels (npm/Go/crates/PyPI) then live at `0.1.1`.

---

## Self-Review

**Spec coverage:** packaging (pyproject + setup.py, BinaryDistribution + get_tag/OBOL_WHEEL_PLAT) — T1 ✓; README pip-user note — T1 ✓; loader unchanged (bundle-ready) ✓; manylinux build + auditwheel — T2 ✓; 4-leg matrix with correct per-arch tags + macOS targets — T3 ✓; version stamp from `pypi-v*` (anchored sed) — T2/T3 ✓; tokenless OIDC publish, no environment, twine check — T3 ✓; wheels-only ✓; runbook — T4 ✓; first release `0.1.1` — Phase 2 ✓.

**Placeholder scan:** no TBD; all code/commands complete with expected output.

**Type/consistency:** `OBOL_WHEEL_PLAT` set in macOS legs, read in `setup.py get_tag`; Linux legs rely on `auditwheel` to tag (no `OBOL_WHEEL_PLAT`); version-stamp sed anchored `^version = "0.0.0"` in both T2 script and T3 macOS leg; artifact names `wheel-*` matched by the publish job's `pattern: wheel-*` + `merge-multiple`; smoke asserts exact stamped version on every leg.
