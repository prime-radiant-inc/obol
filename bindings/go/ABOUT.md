# obol-go

> Generated Go binding for obol — agent-transcript cost estimation, no cgo.

**Family:** obol · **Type:** library · **Lifecycle:** production · **Owner:** mhat

## What it does
A Go module that wraps [obol](https://github.com/prime-radiant-inc/obol). It loads obol's
prebuilt native C-ABI library at runtime via purego (no cgo) and re-types the core's JSON.
The repository is generated: obol's release workflow assembles the Go source from
`obol/bindings/go/` together with the prebuilt native libraries and tags a matching release
here, so `go get` resolves a self-contained module.

## How it fits
- Depends on: [obol](https://github.com/prime-radiant-inc/obol) — generated from
  `obol/bindings/go/`; loads obol's native lib at runtime (`loader.go` binds C symbols
  `obol_version`, `obol_estimate_path`, …); native libs embedded via `embed_*.go`.
  **Not** a `go.mod` dependency (go.mod requires only purego).
- Used by: Go consumers via `go get github.com/prime-radiant-inc/obol-go`
- External: purego (runtime dlopen of the native library)

## Runtime & data
- Runs: library (loaded into a Go host process)
- Data in: obol usage-sidecar JSONL or ATIF `trajectory.json`
- Data out: per-session USD cost estimate

## Links
- Upstream (source of truth): https://github.com/prime-radiant-inc/obol

<!-- Maintained by the maintaining-project-map skill. Do not hand-edit; regenerated. -->
