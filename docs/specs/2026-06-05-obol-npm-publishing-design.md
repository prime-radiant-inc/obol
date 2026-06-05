# obol — npm publishing (design spec)

> 2026-06-05 · Shevek@7998e83e · draft for Bob review · Linear PRI-2094
> First publishing slice. npm first (the OIDC trusted-publishing auth is already set up, so the
> credential friction is lowest). PyPI + crates.io are separate follow-ons. Builds on the TS
> binding (PRI-2085) and the CI build matrix (PRI-2089).

## Goal

Publish the TypeScript binding to npm as **`@primeradianthq/obol`**, tag-driven from GitHub
Actions, so `npm install @primeradianthq/obol` / `bun add @primeradianthq/obol` gives a working,
typed binding that loads the right native library on macOS (arm64/x64) and Linux (x64/arm64).

## Package shape: one package, bundled dylibs

**One** package (not the 5-package optionalDependencies scheme), bundling all four prebuilt
dylibs:

```
@primeradianthq/obol/
  dist/                 # tsup output (.js + .d.ts)
    index.js  index.d.ts
    ffi-bun-<hash>.js   # separate chunk (dynamic import — see "runtime split")
    ffi-node-<hash>.js  # separate chunk
  native/
    darwin-arm64/libobol_ffi.dylib
    darwin-x64/libobol_ffi.dylib
    linux-x64/libobol_ffi.so
    linux-arm64/libobol_ffi.so
  package.json
  README.md
```

**Release+stripped is 2.2 MB/platform → ~9 MB bundled** (measured) — fine for a native npm
package. Rationale (decided with Matt): one package = one create + one trusted-publisher config
(vs five), no postinstall, no install-time network, works under `--ignore-scripts`. The
per-platform optionalDependencies split is a later optimization if size ever bites; not now.

## Build: tsup (verified end-to-end)

The package ships built `.js` + `.d.ts` (so it runs on **Node 18+**, not just Node ≥22.18's
type-stripping, and gives editors types). `tsc` can't emit while the source uses `.ts` import
specifiers (`allowImportingTsExtensions` forces `noEmit`), so the build uses **tsup** (esbuild).

`bindings/typescript/tsup.config.ts` (this exact config was probed against the real binding):

```ts
import { defineConfig } from "tsup";
export default defineConfig({
  entry: ["src/index.ts"],
  format: ["esm"],
  dts: true,
  splitting: true,                 // keeps ffi-bun / ffi-node as SEPARATE chunks
  clean: true,
  outDir: "dist",
  external: ["bun:ffi", "koffi"],  // bun:ffi is a Bun builtin; koffi stays a runtime dep
});
```

**Verified during spec authoring (probe):**
- The build emits `index.js` + **separate** `ffi-bun-*.js` and `ffi-node-*.js` chunks, so the
  dynamic adapter import stays runtime-conditional — **the Bun/Node split survives the build**.
- The built `dist` runs under **both** Node and Bun (`total_usd 0.000995`, `pricing_as_of
  2026-06-05`), and **Node never loads `bun:ffi`** (it ran clean, no `bun:` resolution error).
- **`typescript` must be pinned to `^5.9`.** `bun add -d typescript` pulls **6.0.x**, which
  hard-errors the dts build (`TS5101: 'baseUrl' is deprecated`, injected by tsup's dts path). TS
  5.9.3 builds both ESM and dts cleanly. Pin `typescript: "^5.9"` in devDependencies.

devDependencies added: `tsup` (`^8.5`), `typescript` (`^5.9`).

## Native-library resolution (`lib-path.ts`)

Add a branch that resolves the **bundled** dylib, keeping the existing dev fallbacks. Resolution
order:

1. `OBOL_LIB` (explicit override) — unchanged.
2. **Bundled:** `<dir-of-this-file>/../native/${process.platform}-${process.arch}/libobol_ffi.<ext>`.
   In the published package the running file is under `dist/`, so `../native/…` hits the package's
   `native/` dir. In dev (running `src/lib-path.ts`) `../native` is `bindings/typescript/native`,
   which doesn't exist → falls through. Same code, both contexts.
3. **Dev:** `target/{release,debug}/libobol_ffi.<ext>` (the existing repo-relative fallback).

`${process.platform}-${process.arch}` yields `darwin-arm64`/`darwin-x64`/`linux-x64`/
`linux-arm64` directly. `<ext>` is `dylib` on darwin, `so` on linux. An unsupported platform
(Windows/musl) finds nothing and throws the existing clear "library not found" error — consistent
with the mac+linux scope.

## Published manifest

`bindings/typescript/package.json` changes (dev tests import `../src/index.ts` relatively, so
they don't use `exports` — pointing `exports` at `dist` does not break dev):

```jsonc
{
  "name": "@primeradianthq/obol",
  "version": "0.0.0",                  // stamped from the tag at release
  "license": "Apache-2.0",
  "type": "module",
  "exports": { ".": { "types": "./dist/index.d.ts", "import": "./dist/index.js" } },
  "files": ["dist", "native", "README.md"],
  "engines": { "node": ">=18" },
  "dependencies": { "koffi": "2.16.2" },
  "devDependencies": { "tsup": "^8.5", "typescript": "^5.9" },
  "repository": { "type": "git", "url": "git+https://github.com/prime-radiant-inc/obol.git", "directory": "bindings/typescript" },
  "publishConfig": { "access": "public", "provenance": true }
}
```

`private: true` is removed (the release workflow is the only publisher; `publishConfig.access:
public` + `provenance` are explicit). The `files` allowlist is doing real hygiene work — with it,
`npm pack` is a clean 7-file tarball (`dist`, `native`, `README.md`, `package.json`); *without* it
npm would pack `src/`, `test/`, `tsconfig.json`, `bun.lock`, etc. (verified).

**`.gitignore` (required, not optional):** add `bindings/typescript/dist/` and
`bindings/typescript/native/` so neither the built `dist/` nor a 2.2 MB platform-specific
`native/<plat>/*.dylib` can be accidentally committed. `native/` is assembled only at release
time; `dist/` is a build artifact. Without these ignores the "never committed" guarantee is
unenforced.

Note: `version()` (the binding's API) returns `obol_version()` from the dylib = the **Rust core
crate** version (`0.1.0`), which is intentionally *decoupled* from the npm package version (stamped
from the tag). `npm install @primeradianthq/obol@1.2.3` then `await version()` → `0.1.0`, not
`1.2.3`. That's correct (it's the core-lib version), just a documented surprise.

## Release workflow (`.github/workflows/release.yml`)

Triggered on tag push `v*`. Two stages:

1. **Build the 4 release dylibs** — a matrix mirroring CI's `test` matrix (`macos-14`,
   `macos-15-intel`, `ubuntu-24.04`, `ubuntu-24.04-arm`), each: `cargo build --release -p
   obol-ffi` → **strip (per-OS)** → upload `libobol_ffi.<ext>` as an artifact named by
   `${platform}-${arch}`. **`strip` differs by OS and a plain `strip` FAILS on the macOS dylib**
   (exits 1: "symbols referenced by indirect symbol table entries that can't be stripped"). Use
   `strip -x` on the `macos-*` legs (strips local symbols, keeps the `_obol_*` exports — verified)
   and plain `strip` on the `ubuntu-*` legs. A `RUNNER_OS`-conditional step, not one hardcoded
   `strip`.
2. **Pack & publish** (one `ubuntu-24.04` job, needs stage 1): download the 4 dylib artifacts into
   `bindings/typescript/native/<plat-arch>/`; `bun install`; `bun x tsup` (build `dist`); stamp
   the version from the tag (`npm pkg set version=${TAG#v}`); `npm publish` (with `provenance`).
   Also create a **GitHub Release** for the tag with the 4 dylibs attached (canonical artifact
   store, reused by the PyPI/Go slices later).

**Auth / bootstrap.** The workflow publishes via **OIDC trusted publishing** (`permissions:
id-token: write`, `npm publish --provenance`, npm CLI ≥11.5). But npm can't attach a trusted
publisher to a package that doesn't exist yet, so the **first** publish needs a one-time granular
`@primeradianthq` automation token:
- Option A (workflow-driven): add the token as a GH secret `NPM_TOKEN`; the workflow uses it if
  present, else relies on OIDC. First release with the token creates the package; Matt then
  configures the trusted publisher on the now-existing package and removes the secret; subsequent
  releases are tokenless OIDC.
- Option B (manual first publish): Matt runs one `npm publish` of the assembled package from his
  machine; thereafter the workflow is pure OIDC.

The spec assumes Option A (everything in the workflow); the plan makes the publish step
token-or-OIDC so the transition needs no workflow edit.

## Versioning

The git tag is the source of truth. `v1.2.3` → package version `1.2.3`. The workflow stamps it;
the repo keeps `0.0.0`. (Cargo/other-binding versions are out of scope for this slice.)

## Testing & verification

- **Build (already probed):** `bun x tsup` → `dist` with split chunks + `.d.ts`; built `dist`
  runs under Node and Bun with `total_usd 0.000995`, Node not loading `bun:ffi`. The plan
  re-confirms after the `lib-path` change.
- **Bundled-resolution test:** assemble a `native/<this-plat>/` from the local release dylib,
  build `dist`, and run a consumer that imports the built package **without** `OBOL_LIB` set —
  proving the `native/` branch resolves. Under both runtimes.
- **Pack test:** `npm pack` the assembled package, inspect the tarball (`dist` + `native` +
  manifest, no `src`/`node_modules`), install the tarball into a scratch project, and run an
  estimate — the closest local proxy to a real install. This is the acceptance gate that doesn't
  need the registry.
- **The real publish** waits on the token bootstrap (Matt) and is exercised by the first tag.
- **CI unaffected:** the existing `ci.yml` keeps testing the source (relative imports); add a
  `dist/` ignore and (optionally) a `tsup` build smoke to CI later — not required here.

## Out of scope (this cut)

PyPI, crates.io, Go publishing; Windows/musl; the optionalDependencies per-platform split;
publishing `obol-core`/`obol-cli`/`obol-ffi` to crates.io; automated changelog/release notes.

## Open threads (small)

None blocking.

> Resolved during spec review (Kira@ab69313d, reproduced against the real binding): tsup
> `external:[bun:ffi,koffi]+splitting` keeps each adapter in its own chunk — `index.js` references
> neither, Node never loads `bun:ffi` (built `dist` runs 0.000995 under both runtimes); `lib-path`
> `../native` math correct in both `dist/` and `src/`; `typescript` pinned `^5.9` (6.0.3 errors,
> and the project tsconfig has no baseUrl to `ignoreDeprecations`, so the pin is the clean fix);
> koffi 2.16.2 prebuilds cover all 4 targets and load under `--ignore-scripts` (regular dep is
> fine); `npm pack` clean 7 files; `exports`→`dist` doesn't break dev tests (5/5 still pass);
> provenance works on the public repo. **`strip` must be per-OS** (folded into the workflow step).
> npm on the Node-24 runner is already ≥11.x (no `npm i -g` needed). Use `bun x tsup` (bun install
> already ran).
