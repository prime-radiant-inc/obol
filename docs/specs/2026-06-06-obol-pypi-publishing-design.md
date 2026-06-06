# obol — PyPI publishing (design spec)

> 2026-06-06 · Shevek@7998e83e · draft for Bob review · Linear PRI-2098
> Sixth and final publishing slice. Publish the Python binding to PyPI as `primeradianthq-obol`,
> import name `obol`, as per-platform wheels bundling the prebuilt dylib, via tokenless OIDC.

## Goal

`pip install primeradianthq-obol` then `import obol` gives a working, typed binding on macOS
(arm64/x64) and Linux (x64/arm64), with no C toolchain and no manual library install — the right
platform's `libobol_ffi` is bundled in the wheel and loaded by the existing ctypes loader.

## Names & decoupling

- **Distribution: `primeradianthq-obol`** (PyPI is a flat namespace; bare `obol` is a taken LDAP
  tool). Verified free.
- **Import: `obol`** — unchanged. The wheel ships the `obol/` package; the distribution name is
  separate (`pyproject` `name` ≠ package dir).

## The loader needs no change

`bindings/python/obol/_lib.py` already resolves `OBOL_LIB` → `here.parent / <libname>` (the dylib
beside the package) → dev `target/`. A per-platform wheel that drops one dylib into `obol/` is found
by the middle branch. **Proven** (Appendix A): a wheel bundling `obol/libobol_ffi.dylib` installs and
`obol.version()` returns `0.1.1` with no `OBOL_LIB`.

## Packaging: `bindings/python/pyproject.toml` + `setup.py`

The package is pure-Python (ctypes) but carries a **platform-specific** prebuilt dylib, so the wheel
must be impure and platform-tagged (NOT `py3-none-any`), yet **abi-agnostic** (`py3-none-<plat>` — no
CPython-version dependency). `pyproject.toml`:

```toml
[build-system]
requires = ["setuptools>=64", "wheel"]
build-backend = "setuptools.build_meta"

[project]
name = "primeradianthq-obol"
version = "0.0.0"                      # stamped from the tag at build (see Versioning)
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

`readme = "README.md"` renders on the PyPI page, but `bindings/python/README.md` currently leads with
in-tree dev steps (`mise exec … cargo build -p obol-ffi`) that confuse a `pip install` audience (the
lib is bundled — they never build it). Add a short "**PyPI users:** just `pip install
primeradianthq-obol`; the native library is bundled" lead and keep the dev/build details below it.

`setup.py` forces an impure, platform-but-abi-agnostic wheel and keeps the package files at the wheel
**root** (not under `.data/purelib`, which `root_is_pure=False` alone would do — Appendix A). The
clean way is a `Distribution` that reports it has ext modules, plus a `get_tag` override; the
platform tag is taken from an **env override** so each matrix leg stamps the *correct* arch (the
probe's naive tag was `universal2` for an arm64-only dylib — wrong):

```python
import os
from setuptools import setup
from setuptools.dist import Distribution
try:  # modern path first (wheel ≥0.46 deprecates its own bdist_wheel shim)
    from setuptools.command.bdist_wheel import bdist_wheel as _bdist_wheel
except ImportError:
    from wheel.bdist_wheel import bdist_wheel as _bdist_wheel

class BinaryDistribution(Distribution):
    def has_ext_modules(self):   # → impure wheel, files stay at root
        return True

class bdist_wheel(_bdist_wheel):
    def get_tag(self):
        _py, _abi, plat = super().get_tag()
        plat = os.environ.get("OBOL_WHEEL_PLAT", plat)  # force exact arch per leg
        return ("py3", "none", plat)

setup(distclass=BinaryDistribution, cmdclass={"bdist_wheel": bdist_wheel})
```

For Linux, `OBOL_WHEEL_PLAT` is **not** set in `get_tag`; instead the plain `linux_*` wheel is run
through `auditwheel repair`, which validates the dylib against the manylinux policy and rewrites the
tag to `manylinux_2_28_*` (the authoritative way to get a PyPI-acceptable Linux tag).

## Per-platform wheel matrix (4 legs)

`.github/workflows/pypi-release.yml`, triggered on `pypi-v*`. Each leg builds the cdylib for its
platform, assembles the package, and emits one correctly-tagged wheel:

| Leg | Runner | Build env | Wheel tag |
|---|---|---|---|
| macOS arm64 | `macos-14` | host; `MACOSX_DEPLOYMENT_TARGET=11.0` | `macosx_11_0_arm64` |
| macOS x64 | `macos-15-intel` | host; `MACOSX_DEPLOYMENT_TARGET=10.12` | `macosx_10_12_x86_64` |
| Linux x64 | `ubuntu-24.04` | container `quay.io/pypa/manylinux_2_28_x86_64` | `manylinux_2_28_x86_64` |
| Linux arm64 | `ubuntu-24.04-arm` | container `quay.io/pypa/manylinux_2_28_aarch64` | `manylinux_2_28_aarch64` |

Per leg:
1. **Build the cdylib.** macOS: `cargo build --release -p obol-ffi` on the runner (mise/rustup),
   then `strip -x`. Linux: *inside the manylinux container* install rust (rustup) and
   `cargo build --release -p obol-ffi` → the `.so` links against the container's glibc 2.28, so it's
   `manylinux_2_28`-compatible (this is **why** we don't reuse the ubuntu-24.04 release `.so`).
2. **Assemble** `bindings/python/obol/libobol_ffi.{dylib,so}` (copy the freshly built lib beside the
   package), and stamp the version into `pyproject.toml`.
3. **Build the wheel.** macOS: `OBOL_WHEEL_PLAT=macosx_<tgt>_<arch> python -m build --wheel`. Linux:
   `python -m build --wheel` (plain `linux_*`) → `auditwheel repair --plat manylinux_2_28_<arch>` →
   the repaired `manylinux_*` wheel.
4. **Upload** the one wheel as an artifact.

The manylinux images already ship Python + `pip` + `auditwheel`; macOS legs `pip install build`.

## Versioning

The wheel version is stamped from the tag: `pypi-vX.Y.Z` → `X.Y.Z`. Each leg rewrites
`pyproject.toml`'s version before building, **anchored to the line** so it can't hit a stray `0.0.0`
elsewhere: `sed -i 's/^version = "0.0.0"/version = "X.Y.Z"/' pyproject.toml`. All four legs check out
the same tag and stamp the same version independently (no race — the version lives in the tag). Decoupled `pypi-v*` namespace (like crates' `crates-v*`)
— the `v*` tags stay npm/Go. First release is **`pypi-v0.1.1`** to align all channels. (`version()`
still returns the Rust core version, `0.1.1`, from the bundled dylib — consistent.)

## Publish (tokenless OIDC)

A `publish` job (`needs:` the 4 build legs): download all wheel artifacts into `dist/`,
`twine check dist/*` (metadata + README render sanity), then publish with
`pypa/gh-action-pypi-publish@release/v1` — **no token**, OIDC via the pending publisher Matt
configured (project `primeradianthq-obol`, repo `prime-radiant-inc/obol`, workflow
`pypi-release.yml`, no environment). `permissions: id-token: write`. PyPI's pending-publisher flow
means the very first publish creates the project tokenlessly — no bootstrap.

## Testing & acceptance

- **Probe (done, Appendix A):** a platform wheel bundling the dylib installs `--no-index` into a
  clean venv and `obol.version()` → `0.1.1`, no `OBOL_LIB`.
- **Per-leg smoke (in the workflow, before publish):** after building each wheel, `pip install` it
  into a fresh venv and run `python -c "import obol; assert obol.version() == '<stamped>'"` —
  asserting the **exact** stamped version (truthiness alone would miss a stale/wrong bundled dylib). (On the Linux legs this runs in the manylinux container, proving glibc-2.28
  compatibility.)
- **`twine check`** gates metadata/readme before upload.
- **manylinux validation:** `auditwheel repair` *is* the validation — it refuses to tag a wheel
  whose `.so` needs newer glibc than the policy. A passing repair guarantees PyPI accepts the tag.
- **Local pre-flight (plan):** build the macOS arm64 wheel locally end-to-end (version stamp + tag +
  install + `estimate` against a fixture), and build one Linux wheel in the manylinux container to
  confirm `auditwheel` passes and the wheel installs+loads — the Linux-container-verify standard.
- **Post-publish:** `pip install primeradianthq-obol` in a clean venv on macOS, `import obol`,
  `obol.estimate_path(...)` → `0.000995`; confirm the wheel filename’s tag matches the host arch.

## Out of scope

- **sdist** — a source build needs the Rust toolchain + the cdylib; we ship **wheels only** for the 4
  platforms. (A future sdist could build from source via a PEP 517 backend, but that's a separate
  effort.) PyPI accepts a wheels-only project.
- Windows / musl wheels; per-Python-version wheels (ours is abi-agnostic `py3-none`); cibuildwheel
  (poor fit for an abi-agnostic ctypes wheel — it iterates Python versions); bundling via
  optionalDependencies-style splits.

## Open threads

None — both prior threads resolved by the Brunelleschi review (run against real artifacts):
- **`auditwheel` on our non-extension `.so`: works.** In `manylinux_2_28_x86_64`, `auditwheel repair`
  analyzed the `PyInit_`-less data `.so`, retagged `linux_x86_64` → `manylinux_2_28_x86_64`, and
  **kept the lib at `obol/libobol_ffi.so`** (no ABI-hash rename — that only hits vendored *dependency*
  libs, of which we have none), so the loader still resolves. Installed + imported clean. No fallback
  needed (`wheel tags` would be the backstop, unused).
- **delocate unnecessary.** `otool -L` shows only `libSystem` + `libiconv` (base macOS); the dylib is
  self-contained. The cdylib's max glibc symbol is exactly `GLIBC_2.28` and it links rustls (not
  OpenSSL), so `manylinux_2_28` is correct and tight.

---

## Appendix A — wheel probe (2026-06-06, macOS arm64)

Built a wheel from the real `obol/` package + the `0.1.1` debug dylib via `setuptools` + a
`bdist_wheel.get_tag` override → `primeradianthq_obol-0.1.1-py3-none-macosx_11_0_universal2.whl`,
dylib bundled at `obol/libobol_ffi.dylib`. `pip install --no-index` into a clean venv, then
`import obol; obol.version()` → **`0.1.1`** (loaded the bundled dylib, no `OBOL_LIB`). Pitfalls
found: (1) the naive tag was `universal2` though the dylib is **arm64-only** → must force the exact
arch tag per leg (`OBOL_WHEEL_PLAT`); (2) `root_is_pure=False` routes files under `.data/purelib/` →
use a `has_ext_modules` `Distribution` to keep them at the wheel root instead.
