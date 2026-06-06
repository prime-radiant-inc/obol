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

# Build the cdylib against the container's glibc 2.28 -> manylinux_2_28-compatible.
cargo build --release -p obol-ffi
strip target/release/libobol_ffi.so
cp target/release/libobol_ffi.so bindings/python/obol/libobol_ffi.so

sed -i "s/^version = \"0.0.0\"/version = \"$VERSION\"/" bindings/python/pyproject.toml

cd bindings/python
"$PY" -m pip install --quiet --upgrade build
"$PY" -m build --wheel                                  # -> dist/*-linux_<arch>.whl
auditwheel repair --plat "manylinux_2_28_$ARCH" -w /io/wheelhouse dist/*.whl

# Smoke: install the repaired wheel in a clean venv and assert the exact version.
"$PY" -m venv /tmp/venv
/tmp/venv/bin/pip install --quiet /io/wheelhouse/*.whl
/tmp/venv/bin/python -c "import obol; v=obol.version(); assert v=='$VERSION', v; print('loaded', v)"
