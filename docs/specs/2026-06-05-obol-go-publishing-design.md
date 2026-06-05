# obol — Go publishing (design spec)

> 2026-06-05 · Shevek@7998e83e · draft for Bob review · Linear PRI-2095
> Fourth publishing slice (after npm). Goal: a plain `go get` of the obol Go binding works on a
> clean machine, with **no C toolchain required**. Builds on the C-ABI spine (PRI-2084) and reuses
> the GitHub-Release dylibs produced by the npm release workflow (PRI-2094).

## Goal

`go get github.com/prime-radiant-inc/obol-go` (then `import "github.com/prime-radiant-inc/obol-go"`)
gives a working, typed binding that loads obol's native library at runtime and estimates transcript
cost on macOS (arm64/x64) and Linux (x64/arm64) — `CGO_ENABLED=0`, no compiler, no `cargo`, no
manual library install.

## Why this is shaped differently from npm

Go has **no package registry**. "Publishing" a module is pushing a git tag; `proxy.golang.org`
caches it automatically. So there is no account/token *to the registry*. The entire difficulty is
the **native library**: the current binding is **cgo**, which needs the C lib at build time — a
fresh `go get` on a clean machine has nothing to link. Solving native-lib distribution is the whole
ticket. Two decisions (locked with Matt in brainstorming) shape everything:

1. **Mechanism: purego, not cgo.** Drop cgo; load the lib at runtime via
   [`github.com/ebitengine/purego`](https://github.com/ebitengine/purego). `CGO_ENABLED=0`,
   consumers need no C compiler. **Proven** (see Appendix A): purego v0.10.1 drives the real cdylib
   end-to-end and reproduces the canonical `total_usd 0.000995`.
2. **Distribution: a dedicated `obol-go` repo**, not the monorepo subdir. Go's subdir-module tag
   rule (`bindings/go/vX.Y.Z`) collides with our `vX.Y.Z` release tags and forces a long import
   path; worse, embedding the dylibs (required — see below) would accrete ~10 MB of binary into the
   **main** repo's history *per release*, forever — the very thing we paid `git filter-repo` to
   undo. `obol-go` gives a clean root module path and plain `vX.Y.Z` tags, and isolates the binary
   bloat in a repo whose sole purpose is to carry it. **Source of truth stays in the monorepo**
   (`bindings/go/`); `obol-go` is a **generated publish target** — never hand-edited.

## Native-library strategy: embed + extract + dlopen

purego `dlopen`s a library *by path* at runtime — so the lib must exist on disk somewhere. To make
`go get` self-contained, the per-platform lib is **embedded** in the module via `go:embed`
(build-constrained so a given build carries only its own platform, ~2.5 MB), **extracted** to a
cache dir on first use, and **dlopen'd** from there.

Resolution order in the loader (first hit wins):

1. **`OBOL_LIB`** — explicit path override (escape hatch for locked-down / W^X environments, and the
   dev/CI path). Unchanged contract from the other bindings.
2. **Embedded** — if an embedded lib is present (it is, in published `obol-go`), extract to
   `os.UserCacheDir()/obol-go/<sha256-prefix-of-bytes>/libobol_ffi.<ext>` and dlopen it. The
   content-hash subdir makes the cache **upgrade-safe** (new lib bytes → new dir) and lets concurrent
   processes share one extraction. Write is atomic and **collision-safe**: `os.CreateTemp(dir,
   "lib-*")` (a *uniquely* named temp, so concurrent first-callers don't corrupt each other), write,
   then `os.Rename` onto the shared target — rename is atomic and the bytes are identical, so
   last-writer-wins is harmless. If the target already exists, reuse it (no rewrite).
3. **Dev fallback** — repo-relative `target/{release,debug}/libobol_ffi.<ext>` (so the monorepo
   source builds and tests without any embed, against a freshly-`cargo build`-ed lib).

An unsupported platform (Windows/musl) hits none of these and returns the existing clear
"library not found — set OBOL_LIB" error. `<ext>` is `dylib` on darwin, `so` on linux.

**Extraction-failure hardening** (the loader names `OBOL_LIB` as the remedy in *all* these, not
just "not found"):
- *noexec cache mount* — `dlopen` doesn't need the file's exec bit (it `mmap`s `PROT_EXEC`), but a
  cache dir on a `noexec` mount fails the `mmap` with `EACCES`. On a dlopen failure from the
  extracted path, the loader retries once under `os.TempDir()` (if different), then errors pointing
  at `OBOL_LIB`.
- *macOS* — a dylib written by our own process gets **no** `com.apple.quarantine` xattr, so
  Gatekeeper does not block it (the common worry is a non-issue). The real edge is a consumer
  running under **hardened runtime + library validation**, which rejects an unsigned extracted
  dylib; those consumers point `OBOL_LIB` at a signed copy. One sentence in the README.

Loading is guarded by a `sync.Once`; the first call resolves → extracts → `purego.Dlopen` →
`RegisterLibFunc` for the five symbols, and caches the function values. All subsequent calls reuse
them.

## Source layout (monorepo `bindings/go/`, after the cgo→purego rewrite)

The monorepo holds the **embed-free** source of truth — it resolves the lib via `OBOL_LIB`/`target`
(resolution steps 1 & 3 above; step 2's embed files do not exist here, which is what keeps the
dylibs out of the main repo).

```
bindings/go/
  go.mod                     # module github.com/prime-radiant-inc/obol/bindings/go (unchanged)
  obol/
    obol.go                  # public API + JSON types (unchanged surface; cgo internals replaced)
    loader.go                # purego: resolve → extract → Dlopen → RegisterLibFunc (sync.Once)
    embed_stub.go            # var embeddedLib []byte = nil; const embeddedExt = "" (dev: no embed)
    setenv_native.go         # libc setenv via purego (test/Linux env helper) — see "env" below
    obol_test.go             # same tests; t.Setenv calls replaced by the native env helper
  cmd/total/main.go          # unchanged (still imports the monorepo module path)
  README.md
```

**`obol.go` keeps the exact public API** — `EstimatePath`, `EstimateBytes`, `Refresh`, `Version`,
and the `CostEstimate`/`ModelCost`/`TokenBuckets`/`Approximation`/`RefreshReport`/`ObolError`
types — so `cmd/total`, the equivalence gate, and any consumer are **source-compatible**. Only the
internals change: cgo `C.obol_*` calls become purego-bound function-value calls. Mapping from the
probe (Appendix A):

| FFI symbol | Go-bound signature |
|---|---|
| `obol_version` | `func() uintptr` (static `const char*`, **not** freed) |
| `obol_estimate_path` | `func(path *byte, dialect *byte, out *uintptr) int32` |
| `obol_estimate_bytes` | `func(data *byte, n uintptr, dialect *byte, out *uintptr) int32` |
| `obol_refresh_pricing` | `func(asOf *byte, out *uintptr) int32` |
| `obol_string_free` | `func(p uintptr)` |

Strings cross as NUL-terminated `[]byte` whose `&b[0]` is passed as `*byte` (kept alive across the
synchronous call with `runtime.KeepAlive`); `dialect == ""` passes a **nil** `*byte` → C `NULL` →
auto-detect (preserving today's `dialectArg` semantics). Out-params are read NUL-terminated from the
returned `uintptr` and freed via `obol_string_free` (the `drain` chokepoint, reimplemented over
`unsafe` instead of `C.GoString`). The empty-`data` case keeps the 1-byte-sentinel guard so
`&data[0]` is never taken on a zero-length slice.

## The env-propagation detail (and why a helper exists)

The core reads `OBOL_PRICING_DIR` via libc `getenv`. Probe finding (Appendix A): a **runtime**
`os.Setenv` reaches the dylib on macOS (Go routes through libSystem, shared `environ`) but **on
Linux with `CGO_ENABLED=0` it will not** (Go makes raw syscalls, never links libc, so a dlopen'd
libc keeps its own `environ`). Consequences:

- **Consumers are unaffected** — `OBOL_PRICING_DIR` is a test/override knob; normal use lets the core
  resolve its default pricing path. Nothing consumer-facing depends on runtime env propagation.
- **Env exported *before* the process is safe everywhere** (inherited `environ`). The five-language
  equivalence gate already `export`s `OBOL_PRICING_DIR` before invoking Go — it keeps working
  untouched.
- **Go *unit* tests** (which set the dir at runtime) route through `setenv_native.go`: a tiny helper
  that `dlopen`s libc (`libSystem.B.dylib` on darwin, `libc.so.6` on linux) and calls `setenv`,
  mirroring the TS `test/pricing-env.ts` pattern. The probe confirmed this path works. Tests call
  the helper (which also does `os.Setenv` for Go-side reads + `t.Cleanup` restoration) instead of
  bare `t.Setenv`.

## The published module: `obol-go` (the publish transform)

The release workflow transforms the monorepo source into the `obol-go` module. The transform is a
**selective copy + three edits**, scripted in `scripts/assemble-obol-go.sh` (committed, so it's
reviewable and runnable locally for the pack test):

1. **Copy** `bindings/go/obol/{obol.go,loader.go,setenv_native.go}` to the **root** of an assembly
   dir (flatten — so the import is the clean `github.com/prime-radiant-inc/obol-go`, package `obol`,
   matching the seeded README). **Omit** `obol_test.go`, `cmd/`, and `embed_stub.go`. **Add** a
   minimal `smoke_test.go` (committed verbatim by the script, not generated logic): it asserts
   `Version() != ""`, which forces the full resolve→extract→`Dlopen` path through the **embedded**
   lib (no `OBOL_LIB`, no pricing fixture) — so the workflow's `go test ./...` actually exercises
   the embed path and a broken assembly fails before the tag. The richer accounting-equivalence is
   already covered by the monorepo gate (identical `loader.go`), so it need not be re-shipped.
2. **Generate embed** files — for each platform, `embed_<goos>_<goarch>.go` with the filename build
   constraint Go applies automatically (e.g. `embed_darwin_arm64.go` builds only on darwin/arm64),
   each containing `//go:embed native/<plat>-<arch>/libobol_ffi.<ext>` → `var embeddedLib []byte`
   plus `const embeddedExt`. Add an `embed_unsupported.go`
   (`//go:build !darwin && !linux`) defining `var embeddedLib []byte = nil` so the module still
   *compiles* on other platforms and fails at runtime with the clear loader error (not a build
   error). These four+one files replace the dev `embed_stub.go`.

   > **The filename uses canonical `GOARCH`, NOT the directory naming.** Our dirs are
   > `darwin-x64`/`linux-x64`/`*-arm64` (release.yml convention), but `x64` is **not** a Go arch
   > token. A file named `embed_darwin_x64.go` therefore gets **no implicit build constraint** and
   > compiles everywhere → `embeddedLib redeclared` on all four platforms. The assemble script
   > **must map the arch for the filename**: `x64 → amd64`, `arm64 → arm64`, while keeping the
   > `x64`/`arm64` spelling for the `native/<plat>-<arch>/` embed **path**. So
   > `native/darwin-x64/…` is embedded by a file named `embed_darwin_amd64.go`. (Empirically
   > confirmed: the unmapped name is a hard compile error, caught by the smoke test but only as a
   > confusing build failure.)
3. **Drop in** `native/<plat>-<arch>/libobol_ffi.{dylib,so}` (the four GitHub-Release dylibs) and a
   **rewritten `go.mod`**: `module github.com/prime-radiant-inc/obol-go`, `go 1.21`, and
   `require github.com/ebitengine/purego v0.10.1` (the probed version; purego v0.10.1 needs Go
   ≥1.18, `go:embed` needs ≥1.16, so 1.21 is safe). Then run **`go mod tidy`** in the assembly dir
   and **commit the generated `go.sum`** — a module with a `require` but no `go.sum` is a broken
   `go get` for any consumer on the default `-mod=readonly`. Also copy `LICENSE`, `NOTICE`, and the
   existing seeded `README.md`.

`embed_stub.go` and the generated `embed_*.go` are **mutually exclusive** — the stub lives only in
the monorepo, the generated set only in `obol-go` — so `embeddedLib` is defined exactly once in each
context. The loader code (`loader.go`) is byte-identical across both; only the source of
`embeddedLib`/`embeddedExt` differs.

## Release workflow: the `publish-go` job

Add one job to `.github/workflows/release.yml`, gated `needs: dylibs` (it reuses the same four
stripped release dylibs the npm path already builds — no new build matrix):

```yaml
  publish-go:
    needs: dylibs
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@v4                       # monorepo (source + assemble script)
      - uses: actions/download-artifact@v4              # the 4 dylib-* artifacts
        with: { path: /tmp/dylibs }
      - name: Check out obol-go
        uses: actions/checkout@v4
        with:
          repository: prime-radiant-inc/obol-go
          token: ${{ secrets.OBOL_GO_TOKEN }}           # fine-grained PAT, Contents:write
          path: obol-go
      - name: Refuse if tag already published (idempotency / no half-push)
        working-directory: obol-go
        run: |
          if git ls-remote --exit-code --tags origin "${GITHUB_REF_NAME}" >/dev/null 2>&1; then
            echo "::error::${GITHUB_REF_NAME} already tagged on obol-go — proxy.golang.org is immutable, bump the version."; exit 1
          fi
      - name: Assemble module into obol-go     # selective copy + embed gen + go.mod rewrite + go mod tidy (writes go.sum)
        run: scripts/assemble-obol-go.sh "$GITHUB_WORKSPACE/obol-go" /tmp/dylibs "${GITHUB_REF_NAME#v}"
      - name: Smoke test the assembled module
        working-directory: obol-go
        run: CGO_ENABLED=0 go test ./...               # Version()-only smoke: forces embed→extract→Dlopen, no OBOL_LIB
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

Auth is the fine-grained PAT `OBOL_GO_TOKEN` (Contents:read+write on `obol-go` only) over HTTPS —
the default `GITHUB_TOKEN` can't reach a second repo, and deploy keys are disabled org-wide.
`obol-go` stays **workflow-free**, so Contents-only suffices (a `.github/workflows/` push would
need Workflows:write — deliberately avoided). The assemble script wipes `obol-go`'s tracked module
files before copying so a shrinking file set can't leave stragglers (and `git add -A` then stages
the rewritten `go.mod` + generated `go.sum` + `native/` libs). The smoke test means a broken
assembly fails the release **before** the tag is pushed — load-bearing, because once
`proxy.golang.org` serves a `vX.Y.Z` it is cached **immutably** and can only be superseded, never
fixed in place. The PAT must be minted by an account that has write access to `obol-go` (an org-setup
precondition, not a workflow concern).

## Versioning

Lockstep with the monorepo: tag `vX.Y.Z` on the monorepo → `obol-go` gets the same `vX.Y.Z`. As
with npm, `Version()` returns the **Rust core** version from `obol_version()` (`0.1.0`), decoupled
from the module tag — documented, not a bug. `obol-go`'s first release is whatever tag triggers it;
the bootstrap seed commit is already in place.

## Testing & acceptance

1. **Monorepo unit tests** — the existing `obol_test.go` suite, rewritten to purego + the env helper,
   green under `CGO_ENABLED=0 go test ./...` on macOS and Linux. Same assertions (positive total,
   `pricing_as_of`, error codes 1 and 7).
2. **Five-language equivalence gate** — `scripts/validate_bindings.sh` keeps passing with the
   purego Go leg: `rust == python == go == ts(bun) == ts(node) == 0.000995`. The Go leg no longer
   needs `LD_LIBRARY_PATH`/rpath (purego resolves the path directly via `OBOL_LIB`) — simplify the
   CI Go legs accordingly.
3. **Assembled-module test (the real acceptance)** — run `scripts/assemble-obol-go.sh` locally into a
   scratch dir with the four `native/` libs present, then in that dir
   `CGO_ENABLED=0 go test ./...` and a consumer `go run` that imports the module **without
   `OBOL_LIB`**, proving the embed→extract→dlopen path resolves. This is the closest local proxy to
   what a `go get` consumer sees; it's also exactly the workflow's smoke step.
4. **Linux container verify** (standard for this project) — in `ubuntu:24.04`, build the assembled
   module against the linux `.so` and run the estimate with `CGO_ENABLED=0`, confirming both purego
   on Linux **and** the env helper (libc `setenv`) behave. This closes the macOS-only probe gap.

## CI (monorepo `ci.yml`)

The existing Go legs switch to `CGO_ENABLED=0` and drop the cgo rpath/`LD_LIBRARY_PATH` setup (set
`OBOL_LIB` to the freshly built `target/debug` lib instead). `setup-go` stays. No new runners.

## Out of scope (this cut)

PyPI, crates.io; Windows/musl targets; a per-platform module split; vendoring obol-go's source into
the monorepo; a verify-CI inside `obol-go`; changelog automation. The GitHub App auth upgrade
(no-rotation alternative to the PAT) is noted but deferred. **Cache GC:** each released version
leaves a ~2.5 MB content-hash dir under `UserCacheDir/obol-go/` and we do **not** prune old ones
this cut (bounded by release count; accepted). **musl:** the env-helper `dlopen("libc.so.6")` is
glibc-specific — musl's soname differs; out of scope and consistent with the Linux-glibc-only
target.

## Open threads

None blocking. Two to confirm in review: (a) cache dir choice — `os.UserCacheDir()` vs
`os.TempDir()` for the extracted lib (UserCacheDir persists → no re-extract per run; chosen); (b)
whether to also ship `cmd/total` in `obol-go` as a runnable example (leaning **no** — keep the
published module library-only).

---

## Appendix A — purego probe (run 2026-06-05, macOS arm64, `CGO_ENABLED=0`)

Against the real `target/release/libobol_ffi.dylib`, purego **v0.10.1**:

- `purego.Dlopen` + `RegisterLibFunc` bound `obol_version` / `obol_estimate_bytes` /
  `obol_string_free` with plain Go signatures (`*byte`, `*uintptr`, `uintptr` return). No
  struct-by-value anywhere — the FFI's pointer-flatness is a clean purego fit.
- `obol_version()` → `0.1.0` (static `const char*` read via `uintptr`, not freed).
- `obol_estimate_bytes(claude-mini.jsonl, "claude")` → `code=0`,
  `total_usd 0.000995`, `pricing_as_of 2026-06-05` — **identical** to the other five consumers.
- Env: `os.Setenv("OBOL_PRICING_DIR")` reached the dylib's `getenv` on macOS; the libc-`setenv`-via-
  purego path also worked. (Linux-no-cgo runtime `os.Setenv` is the at-risk case → the env helper +
  inherited-env strategy above; verified in step 4 of acceptance.)

Probe source: `/tmp/obol-purego-probe/` (scratch, not committed).
