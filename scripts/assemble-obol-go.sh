#!/usr/bin/env bash
# Assemble the obol-go module from the monorepo source + the four release dylibs.
# Usage: assemble-obol-go.sh <obol-go-dir> <dylibs-dir> [version]
#   <obol-go-dir>  checked-out obol-go working tree (tracked module files are wiped + rewritten)
#   <dylibs-dir>   contains dylib-<plat>-<arch>/libobol_ffi.{dylib,so} (release.yml artifact layout)
#   [version]      informational only; the module version comes from the git tag
set -euo pipefail
DEST="$1"; DYLIBS="$2"; VERSION="${3:-0.0.0}"
REPO="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$REPO/bindings/go/obol"
PUREGO_VERSION="v0.10.1"

# 1. Wipe previously generated module files (keep .git, .gitignore, README seed).
( cd "$DEST"
  git rm -rq --ignore-unmatch '*.go' go.mod go.sum >/dev/null 2>&1 || true
  rm -rf native )

# 2. Copy the embed-free source, flattened to the module root (package stays `obol`).
#    loader_unsupported.go ships too, so the module compiles (and fails cleanly) off darwin/linux.
cp "$SRC/obol.go"               "$DEST/obol.go"
cp "$SRC/loader.go"             "$DEST/loader.go"
cp "$SRC/loader_unsupported.go" "$DEST/loader_unsupported.go"

# 3. Native libs + generated per-platform embed files.
#    The embed FILENAME uses canonical GOARCH (amd64), NOT the x64 dir naming — else the
#    file gets no build constraint and embeddedLib is redeclared on every platform.
mkdir -p "$DEST/native"
for plat in darwin-arm64 darwin-x64 linux-x64 linux-arm64; do
  os="${plat%-*}"; arch="${plat#*-}"
  ext=dylib; [ "$os" = linux ] && ext=so
  goarch="$arch"; [ "$arch" = x64 ] && goarch=amd64
  mkdir -p "$DEST/native/$plat"
  cp "$DYLIBS/dylib-$plat/libobol_ffi.$ext" "$DEST/native/$plat/libobol_ffi.$ext"
  cat > "$DEST/embed_${os}_${goarch}.go" <<EOF
package obol

import _ "embed"

//go:embed native/$plat/libobol_ffi.$ext
var embeddedLib []byte

const embeddedExt = "$ext"
EOF
done

# 4. Unsupported-platform stub so the module compiles (and fails clearly at runtime) off-target.
cat > "$DEST/embed_unsupported.go" <<'EOF'
//go:build !darwin && !linux

package obol

var embeddedLib []byte

const embeddedExt = ""
EOF

# 5. Version-only smoke test: forces embed→extract→Dlopen with no OBOL_LIB, no pricing fixture.
cat > "$DEST/smoke_test.go" <<'EOF'
package obol

import "testing"

func TestVersionLoadsEmbedded(t *testing.T) {
	if v := Version(); v == "" {
		t.Fatal("Version() empty — the embedded library failed to load")
	}
}
EOF

# 6. go.mod + go.sum (tidy writes go.sum, required for a working `go get`).
cat > "$DEST/go.mod" <<EOF
module github.com/prime-radiant-inc/obol-go

go 1.21

require github.com/ebitengine/purego $PUREGO_VERSION
EOF
( cd "$DEST" && CGO_ENABLED=0 go mod tidy )

# 7. Refresh license/notice from the monorepo (README is the committed seed).
cp "$REPO/LICENSE" "$DEST/LICENSE"
cp "$REPO/NOTICE"  "$DEST/NOTICE"

echo "assembled obol-go $VERSION into $DEST"
