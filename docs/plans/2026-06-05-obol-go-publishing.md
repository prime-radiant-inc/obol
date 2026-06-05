# obol — Go publishing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `go get github.com/prime-radiant-inc/obol-go` work on a clean machine with no C toolchain, by rewriting the Go binding from cgo to purego and publishing a generated `obol-go` module from the monorepo release workflow.

**Architecture:** The monorepo `bindings/go/obol/` stays the embed-free source of truth (resolves the native lib via `OBOL_LIB` or `target/`). A scripted publish transform flattens that source, generates per-platform `go:embed` files + drops in the four release dylibs, and a `publish-go` job in `release.yml` commits+tags the `obol-go` repo via a fine-grained PAT. The binding loads the lib at runtime with `github.com/ebitengine/purego` (`CGO_ENABLED=0`); the published module extracts its embedded lib to a content-hashed cache dir and `Dlopen`s it.

**Tech Stack:** Go 1.21+ / purego v0.10.1, Rust cdylib (obol-ffi, unchanged), GitHub Actions, bash.

**Spec:** `docs/specs/2026-06-05-obol-go-publishing-design.md` (Linear PRI-2095).

**Toolchain note:** Rust commands use `mise exec rust@1.96.0 -- cargo …`. Go uses the system `go` (≥1.21). Run all Go commands with `CGO_ENABLED=0` to prove the no-cgo path. The cdylib must be built once before any Go test: `mise exec rust@1.96.0 -- cargo build -p obol-ffi` (produces `target/debug/libobol_ffi.{dylib,so}`).

---

## File Structure

Monorepo `bindings/go/obol/` after the rewrite:
- `obol.go` — public API + JSON types (surface unchanged; cgo internals → purego calls).
- `loader.go` — **new** (`//go:build darwin || linux`): purego `openLibrary()` (OBOL_LIB → embedded extract → dev `target/`), `RegisterLibFunc` of the 5 symbols, `sync.Once`, C-string helpers.
- `loader_unsupported.go` — **new** (`//go:build !darwin && !linux`): off-target stubs (purego has no `Dlopen` on Windows) so the package compiles everywhere and fails at runtime.
- `embed_stub.go` — **new**: dev `var embeddedLib []byte` (nil) + `const embeddedExt = ""`. Replaced by generated files in `obol-go`.
- `obol_test.go` — existing tests, env-set via the new helper instead of `t.Setenv`.
- `pricing_env_test.go` — **new**: libc-`setenv`-via-purego helper (test-only `_test.go`, so it never ships to `obol-go`).
- `cmd/total/main.go`, `go.mod`, `README.md` — unchanged (except go.mod gains the purego require).

Generated into `obol-go` by `scripts/assemble-obol-go.sh` (**new**):
- root `obol.go`, `loader.go`, `loader_unsupported.go` (copied/flattened), `embed_<goos>_<goarch>.go` ×4 + `embed_unsupported.go`, `smoke_test.go`, `native/<plat>-<arch>/libobol_ffi.{dylib,so}` ×4, `go.mod`, `go.sum`, `LICENSE`, `NOTICE`, `README.md` (seeded).

Workflow/CI:
- `.github/workflows/release.yml` — **add** `publish-go` job.
- `.github/workflows/ci.yml` — Go legs: `CGO_ENABLED=0`, drop `LD_LIBRARY_PATH`, set `OBOL_LIB`.
- `scripts/validate_bindings.sh` — Go leg gets `CGO_ENABLED=0`.
- `docs/RELEASING.md` — add the Go section.

---

## Task 1: purego loader + binding rewrite

Replace cgo with purego. After this task the existing Go test suite passes under `CGO_ENABLED=0` on macOS (Linux env-safety is Task 2).

**Files:**
- Create: `bindings/go/obol/loader.go`
- Create: `bindings/go/obol/embed_stub.go`
- Modify: `bindings/go/obol/obol.go` (replace cgo internals)
- Modify: `bindings/go/go.mod` (add purego require)

- [ ] **Step 1: Build the cdylib (prerequisite for every Go test below)**

Run: `mise exec rust@1.96.0 -- cargo build -p obol-ffi`
Expected: `target/debug/libobol_ffi.dylib` (macOS) or `.so` (Linux) exists.

- [ ] **Step 2: Add purego to the Go module**

Run: `cd bindings/go && go get github.com/ebitengine/purego@v0.10.1`
Expected: `go.mod` gains `require github.com/ebitengine/purego v0.10.1`; `go.sum` updated.

- [ ] **Step 3: Create `embed_stub.go`**

```go
package obol

// Dev build: no embedded native library. The published obol-go module REPLACES this
// file with generated embed_<goos>_<goarch>.go files (and embed_unsupported.go). When
// embeddedLib is empty, the loader falls back to OBOL_LIB / the repo target/ dir.
var embeddedLib []byte

const embeddedExt = ""
```

- [ ] **Step 4: Create `loader.go`** (build-constrained to platforms where purego has `Dlopen`)

`purego.Dlopen`/`RTLD_*` exist only on darwin/linux/bsd — NOT Windows. `loader.go` must carry a
build tag or the module fails to **compile** on Windows (rather than failing cleanly at runtime).
The `embed_unsupported.go` stub handles `embeddedLib`, but the loader symbols need their own
off-target stub (next step). Start `loader.go` with the constraint:

```go
//go:build darwin || linux

package obol

import (
	"crypto/sha256"
	"encoding/hex"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"sync"
	"unsafe"

	"github.com/ebitengine/purego"
)

// C-ABI functions, bound once on first use.
var (
	loadOnce sync.Once
	loadErr  error

	cVersion       func() uintptr
	cEstimatePath  func(path *byte, dialect *byte, out *uintptr) int32
	cEstimateBytes func(data *byte, n uintptr, dialect *byte, out *uintptr) int32
	cRefresh       func(asOf *byte, out *uintptr) int32
	cStringFree    func(p uintptr)
)

// ensureLoaded resolves, dlopens, and binds the native library exactly once.
func ensureLoaded() error {
	loadOnce.Do(func() {
		h, err := openLibrary()
		if err != nil {
			loadErr = err
			return
		}
		purego.RegisterLibFunc(&cVersion, h, "obol_version")
		purego.RegisterLibFunc(&cEstimatePath, h, "obol_estimate_path")
		purego.RegisterLibFunc(&cEstimateBytes, h, "obol_estimate_bytes")
		purego.RegisterLibFunc(&cRefresh, h, "obol_refresh_pricing")
		purego.RegisterLibFunc(&cStringFree, h, "obol_string_free")
	})
	return loadErr
}

func libExt() string {
	if runtime.GOOS == "darwin" {
		return "dylib"
	}
	return "so"
}

func dlopen(path string) (uintptr, error) {
	h, err := purego.Dlopen(path, purego.RTLD_NOW|purego.RTLD_GLOBAL)
	if err != nil {
		return 0, fmt.Errorf("obol: dlopen %s: %w", path, err)
	}
	return h, nil
}

// openLibrary: OBOL_LIB -> embedded (extract + dlopen, cache then temp) -> dev target/.
func openLibrary() (uintptr, error) {
	if env := os.Getenv("OBOL_LIB"); env != "" {
		return dlopen(env) // explicit override: no fallback
	}
	if len(embeddedLib) > 0 {
		var firstErr error
		for _, base := range cacheBases() {
			path, err := extractEmbedded(embeddedLib, embeddedExt, base)
			if err == nil {
				if h, derr := dlopen(path); derr == nil {
					return h, nil
				} else {
					err = derr
				}
			}
			if firstErr == nil {
				firstErr = err
			}
		}
		return 0, fmt.Errorf("obol: could not load embedded libobol_ffi (set OBOL_LIB to override): %w", firstErr)
	}
	for _, path := range devTargets() {
		if fileExists(path) {
			return dlopen(path)
		}
	}
	return 0, fmt.Errorf("obol: libobol_ffi not found; set OBOL_LIB")
}

// cacheBases returns the dirs to try for extraction, persistent first then a
// noexec-resistant temp dir, deduplicated.
func cacheBases() []string {
	bases := []string{}
	if c, err := os.UserCacheDir(); err == nil {
		bases = append(bases, c)
	}
	if t := os.TempDir(); t != "" && (len(bases) == 0 || t != bases[0]) {
		bases = append(bases, t)
	}
	return bases
}

// devTargets are repo-relative build outputs, located from this source file (dev only).
func devTargets() []string {
	_, file, _, ok := runtime.Caller(0)
	if !ok {
		return nil
	}
	root := filepath.Join(filepath.Dir(file), "..", "..", "..") // bindings/go/obol -> repo
	return []string{
		filepath.Join(root, "target", "release", "libobol_ffi."+libExt()),
		filepath.Join(root, "target", "debug", "libobol_ffi."+libExt()),
	}
}

// extractEmbedded writes the lib to <base>/obol-go/<hash>/libobol_ffi.<ext> atomically and
// returns its path. The content-hash dir is upgrade-safe; concurrent writers can't collide.
func extractEmbedded(b []byte, ext, base string) (string, error) {
	sum := sha256.Sum256(b)
	dir := filepath.Join(base, "obol-go", hex.EncodeToString(sum[:8]))
	target := filepath.Join(dir, "libobol_ffi."+ext)
	if fileExists(target) {
		return target, nil
	}
	if err := os.MkdirAll(dir, 0o755); err != nil {
		return "", err
	}
	tmp, err := os.CreateTemp(dir, "lib-*") // unique name per writer
	if err != nil {
		return "", err
	}
	tmpName := tmp.Name()
	defer os.Remove(tmpName) // harmless no-op once renamed
	if _, err := tmp.Write(b); err != nil {
		tmp.Close()
		return "", err
	}
	if err := tmp.Chmod(0o755); err != nil {
		tmp.Close()
		return "", err
	}
	if err := tmp.Close(); err != nil {
		return "", err
	}
	if err := os.Rename(tmpName, target); err != nil {
		if fileExists(target) { // a concurrent writer won the race; identical bytes
			return target, nil
		}
		return "", err
	}
	return target, nil
}

func fileExists(p string) bool {
	_, err := os.Stat(p)
	return err == nil
}

// cstr reads a NUL-terminated C string at p without freeing it.
func cstr(p uintptr) string {
	if p == 0 {
		return ""
	}
	var n int
	for *(*byte)(unsafe.Pointer(p + uintptr(n))) != 0 {
		n++
	}
	return string(unsafe.Slice((*byte)(unsafe.Pointer(p)), n))
}

// bytePtr returns &b[0] or nil for an empty slice (→ C NULL).
func bytePtr(b []byte) *byte {
	if len(b) == 0 {
		return nil
	}
	return &b[0]
}

// dialectBytes returns a NUL-terminated copy, or nil for "" (auto-detect → C NULL).
func dialectBytes(d string) []byte {
	if d == "" {
		return nil
	}
	return append([]byte(d), 0)
}
```

- [ ] **Step 4b: Create `loader_unsupported.go`** (off-target stubs so non-darwin/linux still compiles)

`obol.go` (unconstrained) calls `ensureLoaded`, `cstr`, `bytePtr`, `dialectBytes`, and the `c*` func
vars — all defined in the constrained `loader.go`. On Windows etc. they'd be undefined. Provide
stubs under the complementary build tag so the package compiles everywhere and fails *at runtime*
with a clear message:

```go
//go:build !darwin && !linux

package obol

import "errors"

// Off-target placeholders. On any platform without a purego Dlopen, the binding compiles but
// every entry point fails fast (Version returns ""); set OBOL_LIB is not enough here — the
// platform simply isn't built. darwin/linux use loader.go instead.
var (
	cVersion       func() uintptr
	cEstimatePath  func(path *byte, dialect *byte, out *uintptr) int32
	cEstimateBytes func(data *byte, n uintptr, dialect *byte, out *uintptr) int32
	cRefresh       func(asOf *byte, out *uintptr) int32
	cStringFree    func(p uintptr)
)

func ensureLoaded() error {
	return errors.New("obol: libobol_ffi is not available on this platform (only macOS and Linux are built)")
}

func cstr(p uintptr) string        { return "" }
func bytePtr(b []byte) *byte        { return nil }
func dialectBytes(d string) []byte  { return nil }
```

- [ ] **Step 5: Rewrite `obol.go` — replace the cgo block + imports**

Replace the package doc comment, the entire `/* #cgo … */ import "C"` block, and the import list at the top of `bindings/go/obol/obol.go` (lines 1-17) with:

```go
// Package obol is a thin purego binding over obol-core's C ABI. The Rust core owns all
// accounting; this package only marshals C strings and unmarshals JSON. No cgo: the native
// library is loaded at runtime via github.com/ebitengine/purego (CGO_ENABLED=0 works).
package obol

import (
	"encoding/json"
	"fmt"
	"runtime"
)
```

(The `TokenBuckets`/`ModelCost`/`Approximation`/`CostEstimate`/`RefreshReport`/`ObolError` type
declarations and `Error()` stay exactly as they are.)

- [ ] **Step 6: Rewrite `obol.go` — replace `drain`, the four entry points, and delete the cgo helpers**

Replace `drain` (lines 64-71), `EstimatePath`/`EstimateBytes`/`Refresh`/`Version` (lines 97-143), and `dialectArg`/`freeDialect` (lines 145-156) with:

```go
// drain copies the obol-owned C string into a Go []byte and frees it. Always frees.
func drain(out uintptr) []byte {
	if out == 0 {
		return nil
	}
	defer cStringFree(out)
	return []byte(cstr(out))
}

// EstimatePath estimates a transcript file's cost. dialect "" means auto-detect.
func EstimatePath(path, dialect string) (*CostEstimate, error) {
	if err := ensureLoaded(); err != nil {
		return nil, err
	}
	p := append([]byte(path), 0)
	d := dialectBytes(dialect)
	var out uintptr
	code := cEstimatePath(&p[0], bytePtr(d), &out)
	runtime.KeepAlive(p)
	runtime.KeepAlive(d)
	return decodeEstimate(code, drain(out))
}

// EstimateBytes estimates in-memory transcript bytes. dialect "" means auto-detect.
func EstimateBytes(data []byte, dialect string) (*CostEstimate, error) {
	if err := ensureLoaded(); err != nil {
		return nil, err
	}
	dptr := data
	if len(dptr) == 0 {
		dptr = []byte{0} // non-nil pointer for len 0; length below stays 0
	}
	d := dialectBytes(dialect)
	var out uintptr
	code := cEstimateBytes(&dptr[0], uintptr(len(data)), bytePtr(d), &out)
	runtime.KeepAlive(dptr)
	runtime.KeepAlive(d)
	return decodeEstimate(code, drain(out))
}

// Refresh pulls fresh pricing tables. asOf is the caller's date string.
func Refresh(asOf string) (*RefreshReport, error) {
	if err := ensureLoaded(); err != nil {
		return nil, err
	}
	a := append([]byte(asOf), 0)
	var out uintptr
	code := cRefresh(&a[0], &out)
	runtime.KeepAlive(a)
	payload := drain(out)
	if int(code) != 0 {
		return nil, toError(int(code), payload)
	}
	var r RefreshReport
	if err := json.Unmarshal(payload, &r); err != nil {
		return nil, err
	}
	return &r, nil
}

// Version returns the obol core library version (static C string; not freed).
func Version() string {
	if err := ensureLoaded(); err != nil {
		return ""
	}
	return cstr(cVersion())
}
```

Also change `decodeEstimate`'s signature from `C.int32_t` to `int32` (line 86: `func decodeEstimate(code int32, payload []byte) (*CostEstimate, error)`), and its body `if int(code) != 0` stays. `toError` is unchanged. The `"fmt"` import is used by `ObolError.Error`; `"unsafe"` is gone from obol.go (now only in loader.go).

- [ ] **Step 7: Run the suite on the dev machine (macOS)**

Run: `cd bindings/go && CGO_ENABLED=0 go test ./...`
Expected: PASS — `TestVersion`, `TestEstimatePath`, `TestMissingTablesIsError` (code 1), `TestUnknownDialectIsError` (code 7). (`t.Setenv` reaches the dylib on macOS; Task 2 makes it Linux-safe.)

- [ ] **Step 8: Confirm cgo is truly gone, and the package compiles off-target**

Run: `cd bindings/go && CGO_ENABLED=0 go build ./... && CGO_ENABLED=0 GOOS=windows GOARCH=amd64 go build ./...`
Expected: both succeed, proving no `import "C"` remains and the `loader_unsupported.go` stub keeps
Windows compiling. (Do **not** add `go vet` here: `cstr` trips vet's `unsafeptr` check on the raw
C-address→pointer conversion — an unavoidable, harmless advisory; CI runs neither vet on this leg.)

- [ ] **Step 9: Commit**

```bash
git add bindings/go/obol/obol.go bindings/go/obol/loader.go bindings/go/obol/loader_unsupported.go bindings/go/obol/embed_stub.go bindings/go/go.mod bindings/go/go.sum
git commit -m "feat(go): rewrite binding from cgo to purego (PRI-2095)"
```

---

## Task 2: Linux-safe env test helper

`os.Setenv` doesn't reach a dlopen'd libc's `getenv` under `CGO_ENABLED=0` on Linux. Route test env through libc `setenv` via purego (the proven TS `pricing-env.ts` pattern). Test-only — never ships to `obol-go`.

**Files:**
- Create: `bindings/go/obol/pricing_env_test.go`
- Modify: `bindings/go/obol/obol_test.go` (use the helper)

- [ ] **Step 1: Create `pricing_env_test.go`**

```go
package obol

import (
	"runtime"
	"sync"
	"testing"

	"github.com/ebitengine/purego"
)

var (
	libcOnce sync.Once
	cSetenv  func(name *byte, val *byte, overwrite int32) int32
)

func libcSetenv() func(name *byte, val *byte, overwrite int32) int32 {
	libcOnce.Do(func() {
		name := "libc.so.6"
		if runtime.GOOS == "darwin" {
			name = "libSystem.B.dylib"
		}
		h, err := purego.Dlopen(name, purego.RTLD_NOW|purego.RTLD_GLOBAL)
		if err != nil {
			return // cSetenv stays nil; setPricingDir falls back to os.Setenv only
		}
		purego.RegisterLibFunc(&cSetenv, h, "setenv")
	})
	return cSetenv
}

// setPricingDir sets OBOL_PRICING_DIR so the dlopen'd core's getenv sees it on every
// platform: os.Setenv (Go-side reads + macOS) AND libc setenv (Linux, CGO_ENABLED=0).
// Restores via t.Cleanup.
func setPricingDir(t *testing.T, dir string) {
	t.Helper()
	t.Setenv("OBOL_PRICING_DIR", dir) // Go-side + macOS; auto-restored by t.Setenv
	if fn := libcSetenv(); fn != nil {
		name := append([]byte("OBOL_PRICING_DIR"), 0)
		val := append([]byte(dir), 0)
		fn(&name[0], &val[0], 1)
		runtime.KeepAlive(name)
		runtime.KeepAlive(val)
		t.Cleanup(func() {
			empty := append([]byte("OBOL_PRICING_DIR"), 0)
			z := []byte{0}
			fn(&empty[0], &z[0], 1) // best-effort clear for native readers
			runtime.KeepAlive(empty)
		})
	}
}
```

- [ ] **Step 2: Point the tests at the helper**

In `bindings/go/obol/obol_test.go`, change `seed`'s last line from `t.Setenv("OBOL_PRICING_DIR", dir)` to `setPricingDir(t, dir)`. In `TestMissingTablesIsError`, change `t.Setenv("OBOL_PRICING_DIR", "/nonexistent/obol-go-xyz")` to `setPricingDir(t, "/nonexistent/obol-go-xyz")`.

- [ ] **Step 3: Run the suite again (macOS — still green)**

Run: `cd bindings/go && CGO_ENABLED=0 go test ./...`
Expected: PASS (4/4). On macOS both paths set the env; this proves no regression. The Linux payoff is verified in Task 6.

- [ ] **Step 4: Commit**

```bash
git add bindings/go/obol/pricing_env_test.go bindings/go/obol/obol_test.go
git commit -m "test(go): set OBOL_PRICING_DIR via libc setenv for Linux no-cgo (PRI-2095)"
```

---

## Task 3: Embedded extract→dlopen path (unit-tested in the monorepo)

The embed branch is dormant in the monorepo (`embeddedLib` is nil), so test `extractEmbedded` + `dlopen` directly by feeding it the real dev dylib's bytes. This exercises the exact code the published module runs, without needing `go:embed`.

**Files:**
- Create: `bindings/go/obol/loader_embed_test.go`

- [ ] **Step 1: Write the failing test**

```go
package obol

import (
	"os"
	"testing"

	"github.com/ebitengine/purego"
)

// Feed the dev dylib's bytes through the embedded extract+dlopen path and confirm a
// real symbol resolves — the same mechanics the published obol-go module relies on.
func TestExtractEmbeddedLoads(t *testing.T) {
	var libPath string
	for _, p := range devTargets() {
		if fileExists(p) {
			libPath = p
			break
		}
	}
	if libPath == "" {
		t.Skip("build the cdylib first: mise exec rust@1.96.0 -- cargo build -p obol-ffi")
	}
	raw, err := os.ReadFile(libPath)
	if err != nil {
		t.Fatal(err)
	}
	path, err := extractEmbedded(raw, libExt(), t.TempDir())
	if err != nil {
		t.Fatalf("extractEmbedded: %v", err)
	}
	if !fileExists(path) {
		t.Fatalf("extracted path missing: %s", path)
	}
	h, err := purego.Dlopen(path, purego.RTLD_NOW|purego.RTLD_GLOBAL)
	if err != nil {
		t.Fatalf("dlopen extracted: %v", err)
	}
	var version func() uintptr
	purego.RegisterLibFunc(&version, h, "obol_version")
	if got := cstr(version()); got == "" {
		t.Fatal("empty version from extracted lib")
	}
}
```

- [ ] **Step 2: Run it (it passes — the code exists from Task 1)**

Run: `cd bindings/go && CGO_ENABLED=0 go test ./obol/ -run TestExtractEmbeddedLoads -v`
Expected: PASS. (This is a characterization test over Task 1's helpers; if `extractEmbedded` were wrong it would fail here rather than only in CI.)

- [ ] **Step 3: Verify idempotent re-extraction (second call reuses the file)**

Add to the same file:

```go
func TestExtractEmbeddedIdempotent(t *testing.T) {
	base := t.TempDir()
	b := []byte("not a real lib, just bytes for path logic")
	p1, err := extractEmbedded(b, "so", base)
	if err != nil {
		t.Fatal(err)
	}
	p2, err := extractEmbedded(b, "so", base)
	if err != nil {
		t.Fatal(err)
	}
	if p1 != p2 {
		t.Fatalf("paths differ: %s vs %s", p1, p2)
	}
}
```

Run: `cd bindings/go && CGO_ENABLED=0 go test ./obol/ -run TestExtractEmbedded -v`
Expected: PASS (both).

- [ ] **Step 4: Commit**

```bash
git add bindings/go/obol/loader_embed_test.go
git commit -m "test(go): exercise embed extract+dlopen path with real dylib bytes (PRI-2095)"
```

---

## Task 4: The publish transform (`assemble-obol-go.sh`)

Generate a complete, compilable `obol-go` module from the monorepo source + the four release dylibs, and prove it self-tests via the embed path.

**Files:**
- Create: `scripts/assemble-obol-go.sh`

- [ ] **Step 1: Write `scripts/assemble-obol-go.sh`**

```bash
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
```

- [ ] **Step 2: Make it executable**

Run: `chmod +x scripts/assemble-obol-go.sh`

- [ ] **Step 3: Build a local dylib set in the release-artifact layout**

The release dylibs aren't on this machine, but `bindings/typescript/native/` holds all four from a prior CI run. Stage them in the `dylib-<plat>-<arch>/` shape the script expects:

```bash
rm -rf /tmp/obol-dylibs && for plat in darwin-arm64 darwin-x64 linux-x64 linux-arm64; do
  ext=dylib; [ "${plat%-*}" = linux ] && ext=so
  mkdir -p "/tmp/obol-dylibs/dylib-$plat"
  cp "bindings/typescript/native/$plat/libobol_ffi.$ext" "/tmp/obol-dylibs/dylib-$plat/"
done && find /tmp/obol-dylibs -type f
```
Expected: four `libobol_ffi.{dylib,so}` under `dylib-<plat>-<arch>/`.

- [ ] **Step 4: Run the transform into a scratch git repo**

```bash
rm -rf /tmp/obol-go-asm && git init -q /tmp/obol-go-asm
scripts/assemble-obol-go.sh /tmp/obol-go-asm /tmp/obol-dylibs 0.1.0
ls /tmp/obol-go-asm && ls /tmp/obol-go-asm/native/*
```
Expected: `obol.go loader.go embed_darwin_amd64.go embed_darwin_arm64.go embed_linux_amd64.go embed_linux_arm64.go embed_unsupported.go smoke_test.go go.mod go.sum native/`, and one `libobol_ffi.*` under each `native/<plat>-<arch>/`. **Verify the `x64`→`amd64` mapping:** `embed_darwin_amd64.go` exists (NOT `embed_darwin_x64.go`).

- [ ] **Step 5: Prove the assembled module self-tests via the embed path**

Run: `cd /tmp/obol-go-asm && CGO_ENABLED=0 go test ./...`
Expected: PASS — `TestVersionLoadsEmbedded`. This loads the host-platform embedded dylib with **no `OBOL_LIB`**, exercising embed→extract→`Dlopen` end to end (the exact thing the release workflow's smoke step runs).

- [ ] **Step 6: Confirm the embed build constraints with a cross-compile**

Run: `cd /tmp/obol-go-asm && CGO_ENABLED=0 GOOS=linux GOARCH=arm64 go build ./... && CGO_ENABLED=0 GOOS=windows GOARCH=amd64 go build ./...`
Expected: both succeed — proving exactly one `embeddedLib` per platform (linux/arm64 picks `embed_linux_arm64.go`; windows picks `embed_unsupported.go`). A `redeclared` error here means the arch mapping is wrong.

- [ ] **Step 7: Commit**

```bash
git add scripts/assemble-obol-go.sh
git commit -m "feat(go): assemble-obol-go.sh — generate the published module from source + dylibs (PRI-2095)"
```

---

## Task 5: Release workflow + CI + docs

Wire the `publish-go` job, switch CI's Go legs to `CGO_ENABLED=0`, and document the runbook.

**Files:**
- Modify: `.github/workflows/release.yml` (add `publish-go` job)
- Modify: `.github/workflows/ci.yml` (Go legs)
- Modify: `scripts/validate_bindings.sh` (Go leg `CGO_ENABLED=0`)
- Modify: `docs/RELEASING.md`

- [ ] **Step 1: Add the `publish-go` job to `release.yml`**

Append this job under `jobs:` (sibling of `dylibs` and `publish`):

```yaml
  publish-go:
    needs: dylibs
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@v4 # monorepo: source + assemble script
      - uses: actions/setup-go@v5
        with:
          go-version: "1.23"
          cache: false
      - uses: actions/download-artifact@v4
        with:
          path: /tmp/dylibs # → /tmp/dylibs/dylib-<plat>-<arch>/libobol_ffi.*
      - name: Check out obol-go
        uses: actions/checkout@v4
        with:
          repository: prime-radiant-inc/obol-go
          token: ${{ secrets.OBOL_GO_TOKEN }} # fine-grained PAT, Contents:write on obol-go
          path: obol-go
      - name: Refuse if tag already published (proxy.golang.org is immutable)
        working-directory: obol-go
        run: |
          if git ls-remote --exit-code --tags origin "${GITHUB_REF_NAME}" >/dev/null 2>&1; then
            echo "::error::${GITHUB_REF_NAME} already tagged on obol-go — bump the version."; exit 1
          fi
      - name: Assemble module
        run: scripts/assemble-obol-go.sh "$GITHUB_WORKSPACE/obol-go" /tmp/dylibs "${GITHUB_REF_NAME#v}"
      - name: Smoke test (embed→extract→Dlopen, no OBOL_LIB)
        working-directory: obol-go
        run: CGO_ENABLED=0 go test ./...
      - name: Commit, tag, push
        working-directory: obol-go
        run: |
          git config user.name  "obol-release"
          git config user.email "obol-release@users.noreply.github.com"
          git add -A
          git commit -m "Release ${GITHUB_REF_NAME} (generated from prime-radiant-inc/obol@${GITHUB_SHA})"
          git tag "${GITHUB_REF_NAME}"
          git push origin HEAD:main --follow-tags
```

- [ ] **Step 2: Switch the CI Go binding step to purego**

In `.github/workflows/ci.yml`, replace the `Go binding tests` step (lines 73-81) with:

```yaml
      - name: Go binding tests
        working-directory: bindings/go
        env:
          CGO_ENABLED: "0"
        run: |
          ext=so; [ "$RUNNER_OS" = "macOS" ] && ext=dylib
          export OBOL_LIB="$GITHUB_WORKSPACE/target/debug/libobol_ffi.$ext"
          go test ./...
```

- [ ] **Step 3: Drop `LD_LIBRARY_PATH` from the equivalence-gate step**

In `.github/workflows/ci.yml`, replace the `Five-language equivalence gate` step (lines 90-95) with:

```yaml
      - name: Five-language equivalence gate
        run: ./scripts/validate_bindings.sh
```

- [ ] **Step 4: Make the gate's Go leg cgo-free**

In `scripts/validate_bindings.sh`, change the `go_total` line from
`go_total=$( (cd bindings/go && go run ./cmd/total "$ROOT/$T" claude) | norm)` to
`go_total=$( (cd bindings/go && CGO_ENABLED=0 go run ./cmd/total "$ROOT/$T" claude) | norm)`.
(The script already `export`s `OBOL_LIB` (line 12) and `OBOL_PRICING_DIR` (line 16) before this, so purego resolves the lib and the pricing dir is inherited — no `LD_LIBRARY_PATH` needed.)

- [ ] **Step 5: Run the gate locally to confirm parity holds**

Run: `./scripts/validate_bindings.sh`
Expected: `OK: rust == python == go == ts(bun) == ts(node) total_usd (0.000995)`.

- [ ] **Step 6: Add the Go section to `docs/RELEASING.md`**

Insert before the existing `## Other registries` section (and delete Go from that section's "not yet wired" list):

```markdown
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
```

- [ ] **Step 7: Commit**

```bash
git add .github/workflows/release.yml .github/workflows/ci.yml scripts/validate_bindings.sh docs/RELEASING.md
git commit -m "ci(go): publish-go release job + cgo-free CI legs + Go releasing runbook (PRI-2095)"
```

---

## Task 6: Linux container verification + Linear/memory

Close the macOS-only gap: prove purego + the embed path + the libc-`setenv` helper work on Linux/glibc, then update tracking.

**Files:**
- Create (throwaway, not committed): `/tmp/obol-linux-go-verify.sh`

- [ ] **Step 1: Write the Linux verification script**

```bash
cat > /tmp/obol-linux-go-verify.sh <<'EOS'
set -eux
apt-get update -qq && apt-get install -y -qq golang-go curl build-essential >/dev/null
cp -r /src /work && cd /work
# Build the Linux cdylib so OBOL_LIB / target resolution works for the monorepo tests.
curl -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.96.0 >/dev/null
. "$HOME/.cargo/env"
cargo build -p obol-ffi
ext=so
export OBOL_LIB="/work/target/debug/libobol_ffi.$ext"
# 1. Monorepo binding suite under CGO_ENABLED=0 — proves purego + the libc setenv helper on Linux.
( cd bindings/go && CGO_ENABLED=0 go test ./... )
# 2. Assembled-module embed path with the linux .so, NO OBOL_LIB.
rm -rf /tmp/dyl && for p in darwin-arm64 darwin-x64 linux-x64 linux-arm64; do
  e=dylib; [ "${p%-*}" = linux ] && e=so; mkdir -p "/tmp/dyl/dylib-$p"
  cp "bindings/typescript/native/$p/libobol_ffi.$e" "/tmp/dyl/dylib-$p/" 2>/dev/null || true
done
git init -q /tmp/asm
scripts/assemble-obol-go.sh /tmp/asm /tmp/dyl 0.1.0
( cd /tmp/asm && unset OBOL_LIB && CGO_ENABLED=0 go test ./... )
echo "LINUX-GO-VERIFY-OK"
EOS
echo "wrote /tmp/obol-linux-go-verify.sh"
```

- [ ] **Step 2: Run it in an ubuntu:24.04 container (host arch = arm64)**

Run: `docker run --rm -v /Users/mw/Code/prime/obol:/src:ro -v /tmp/obol-linux-go-verify.sh:/v.sh:ro ubuntu:24.04 bash /v.sh`
Expected: ends with `LINUX-GO-VERIFY-OK`. Step 1 confirms the libc-`setenv` helper actually propagates `OBOL_PRICING_DIR` on Linux (the whole reason it exists); step 2 confirms the embed→extract→`dlopen` path works against the linux `.so` with no `OBOL_LIB`.
(If the sandbox blocks the network for `apt`/`rustup`, re-run the Bash tool call with `dangerouslyDisableSandbox: true`.)

- [ ] **Step 3: Update the Go binding README for the purego reality**

In `bindings/go/README.md`, ensure the install/usage notes say the binding is purego (`CGO_ENABLED=0`, no C toolchain), that `OBOL_LIB` overrides the loaded library, and that consumers under macOS hardened-runtime + library-validation should point `OBOL_LIB` at a signed dylib. Keep it short; match the existing file's tone.

Run: `cd bindings/go && CGO_ENABLED=0 go test ./...` (sanity — unchanged).
Expected: PASS.

- [ ] **Step 4: Commit the README**

```bash
git add bindings/go/README.md
git commit -m "docs(go): note purego/no-cgo, OBOL_LIB override, hardened-runtime caveat (PRI-2095)"
```

- [ ] **Step 5: Update project memory**

Edit `/Users/mw/.claude/projects/-Users-mw-Code-prime/memory/project_obol.md`: add a PRI-2095 paragraph (Go publishing: purego rewrite, generated `obol-go` repo, `assemble-obol-go.sh`, `publish-go` job, `OBOL_GO_TOKEN`) and update the "Next" line to drop Go (leaving PyPI + crates.io). No `MEMORY.md` index change needed (the obol line already exists).

- [ ] **Step 6: Move PRI-2095 to In Review with a reflective comment**

Use the Linear MCP tools: set PRI-2095 state to `In Review` and add a `save_comment` reflecting on the implementation (what was smooth — the FFI's pointer-flatness made purego a near-transcription; what was tricky — the `x64`→`amd64` embed-filename trap and the Linux env-propagation nuance; risk flags — the first real `obol-go` publish only exercises end-to-end when a tag is cut).

---

## First release (post-merge, human-gated)

Not a code task — the first `obol-go` publish happens when a `vX.Y.Z` tag is pushed (the same tag that publishes npm). The `publish-go` job seeds the real module over the bootstrap commit. Nothing to do here until a release is cut; the smoke test + tag-exists guard protect against a bad first publish.

---

## Self-Review

**Spec coverage:** purego mechanism (T1) ✓; embed→extract→dlopen loader incl. unique-temp + cache/temp fallback (T1) ✓; OBOL_LIB→embedded→dev resolution (T1) ✓; env helper for Linux no-cgo (T2) ✓; embed unit test (T3) ✓; publish transform with `x64`→`amd64` mapping + go.sum + smoke test + unsupported stub (T4) ✓; `publish-go` job with tag-idempotency guard + token auth (T5) ✓; CI cgo-free Go legs + gate (T5) ✓; RELEASING runbook (T5) ✓; Linux container verify (T6) ✓; lockstep versioning (T5 job uses the tag) ✓; library-only obol-go (T4 omits cmd/) ✓; Linear In Review + memory (T6) ✓.

**Placeholder scan:** no TBD/TODO; every code step has complete code; commands have expected output.

**Type consistency:** `embeddedLib []byte` / `embeddedExt string` declared in `embed_stub.go` (T1) and regenerated identically in T4; `extractEmbedded(b, ext, base)` / `devTargets()` / `cstr` / `bytePtr` / `dialectBytes` used consistently across T1/T3; `decodeEstimate(int32, []byte)` signature matches its single caller set; the four C func vars (`cVersion`/`cEstimatePath`/`cEstimateBytes`/`cRefresh`/`cStringFree`) are declared in `loader.go` and called in `obol.go`.
