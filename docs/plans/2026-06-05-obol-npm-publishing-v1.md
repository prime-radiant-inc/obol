# obol npm publishing — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans (inline) — the
> final verify (real publish) needs Matt's one-time npm token; everything else is verified
> locally + by actionlint. Steps use checkbox (`- [ ]`) syntax.

**Goal:** Publish the TypeScript binding to npm as `@primeradianthq/obol`, tag-driven from GitHub
Actions — one package bundling the four platform dylibs, runnable on Node 18+ and Bun.

**Architecture:** Package-local changes (tsup build → `dist`, `lib-path` resolves a bundled
`native/<plat>-<arch>/` dylib, publish manifest) + a tag-triggered `release.yml` that builds the
4 release dylibs (per-OS strip), assembles `native/`, builds `dist`, stamps the version from the
tag, and publishes (token-bootstrap → OIDC). Spec:
`docs/specs/2026-06-05-obol-npm-publishing-design.md` (the tsup approach + path math were probed).

**Tech stack:** tsup (esbuild) `^8.5`, typescript `^5.9` (6.x errors), koffi `2.16.2`, GitHub
Actions, npm OIDC trusted publishing + provenance.

---

## Task 1: Package-local — manifest, tsup, lib-path native branch, gitignore

**Files:**
- Modify: `bindings/typescript/package.json`
- Create: `bindings/typescript/tsup.config.ts`
- Modify: `bindings/typescript/src/lib-path.ts`
- Modify: `.gitignore`

- [ ] **Step 1: Ignore build outputs first** (so nothing gets staged). Append to root `.gitignore`:

```
# TS binding build outputs (dist built by tsup; native/ assembled at release time)
/bindings/typescript/dist
/bindings/typescript/native
```

- [ ] **Step 2: Write `bindings/typescript/tsup.config.ts`** (the probed config):

```ts
import { defineConfig } from "tsup";

export default defineConfig({
  entry: ["src/index.ts"],
  format: ["esm"],
  dts: true,
  splitting: true, // keep ffi-bun / ffi-node as SEPARATE chunks (preserves the Bun/Node split)
  clean: true,
  outDir: "dist",
  external: ["bun:ffi", "koffi"], // bun:ffi is a Bun builtin; koffi stays a runtime dep
});
```

- [ ] **Step 3: Update `bindings/typescript/package.json`** to the published manifest (remove
  `private`, scope the name, point `exports`/`files` at the built outputs, add the build devDeps):

```json
{
  "name": "@primeradianthq/obol",
  "version": "0.0.0",
  "license": "Apache-2.0",
  "type": "module",
  "description": "Agent-transcript cost estimation — TypeScript binding over the obol C ABI (Bun + Node).",
  "exports": { ".": { "types": "./dist/index.d.ts", "import": "./dist/index.js" } },
  "files": ["dist", "native", "README.md"],
  "engines": { "node": ">=18" },
  "dependencies": { "koffi": "2.16.2" },
  "devDependencies": { "tsup": "^8.5", "typescript": "^5.9" },
  "repository": { "type": "git", "url": "git+https://github.com/prime-radiant-inc/obol.git", "directory": "bindings/typescript" },
  "publishConfig": { "access": "public", "provenance": true }
}
```

- [ ] **Step 4: Add the `native/` resolution branch to `bindings/typescript/src/lib-path.ts`.**
  Replace the whole `resolveLibPath` function with:

```ts
export function resolveLibPath(): string {
  const tried: string[] = [];
  const env = process.env.OBOL_LIB;
  if (env) {
    tried.push(env);
    if (existsSync(env)) return env;
  }
  const name = libFilename();
  const here = dirname(fileURLToPath(import.meta.url));
  // Published layout: this file is under dist/, dylibs under ../native/<platform>-<arch>/.
  // In dev (running src/), ../native doesn't exist and we fall through to target/.
  const bundled = join(here, "..", "native", `${process.platform}-${process.arch}`, name);
  tried.push(bundled);
  if (existsSync(bundled)) return bundled;
  // Dev: repo-relative target/{release,debug}.
  const repo = join(here, "..", "..", "..");
  for (const profile of ["release", "debug"]) {
    const p = join(repo, "target", profile, name);
    tried.push(p);
    if (existsSync(p)) return p;
  }
  throw new Error(
    "obol_ffi shared library not found. Set OBOL_LIB or install a platform with a bundled lib. Tried:\n  " +
      tried.join("\n  "),
  );
}
```

  (The existing `libFilename()`, imports of `existsSync`/`dirname`/`join`/`fileURLToPath` stay.)

- [ ] **Step 5: Install deps + build.**

```bash
cd bindings/typescript && bun install
bun x tsup
ls dist/   # expect: index.js, ffi-bun-<hash>.js, ffi-node-<hash>.js, index.d.ts
```

Expected: `ESM ⚡️ Build success` + `DTS ⚡️ Build success`. If dts errors with `TS5101 baseUrl`,
`bun add -d typescript@^5.9` (6.x is the culprit) and rebuild.

- [ ] **Step 6: Verify the built `dist` resolves the bundled lib + runs under BOTH runtimes.**

```bash
cd bindings/typescript
mise exec rust@1.96.0 -- (cd ../.. && cargo build --release -p obol-ffi)
PLAT="$(node -p 'process.platform+"-"+process.arch')"
EXT="$([ "$(uname)" = Darwin ] && echo dylib || echo so)"
mkdir -p "native/$PLAT"
cp "../../target/release/libobol_ffi.$EXT" "native/$PLAT/"
SEED=$(mktemp -d); cp ../testdata/prices.json "$SEED/current.json"
T="$(cd ../.. && pwd)/bindings/testdata/claude-mini.jsonl"
DIST="$(pwd)/dist/index.js"
printf 'import {estimatePath,version} from "%s";console.log(await version(), (await estimatePath("%s","claude")).total_usd);\n' "$DIST" "$T" > /tmp/np-smoke.mjs
echo "node:"; OBOL_PRICING_DIR="$SEED" node /tmp/np-smoke.mjs   # expect: 0.1.0 0.000995, NO bun:ffi error
echo "bun:";  OBOL_PRICING_DIR="$SEED" bun  /tmp/np-smoke.mjs   # expect: 0.1.0 0.000995
rm -rf "$SEED"
```

Expected: both print `0.1.0 0.000995`. (Note: this resolved the lib via `native/` with **no
`OBOL_LIB`**, proving the bundled branch. `native/` is gitignored.)

- [ ] **Step 7: Verify dev tests still pass** (relative `../src` imports, target/ fallback):

```bash
cd bindings/typescript && rm -rf native && bun test && node --test test/obol.test.ts
```

Expected: 5 pass under each (native/ removed → dev resolves via `target/release` or `target/debug`).

- [ ] **Step 8: Pack-test the tarball contents.** Re-assemble `native/` for this platform (Step 6),
  then:

```bash
cd bindings/typescript && npm pack --dry-run 2>&1 | grep -E "Tarball Contents|\.(js|d\.ts|dylib|so)|package.json|README" | head -20
```

Expected: only `dist/*`, `native/<plat>/*`, `package.json`, `README.md` — **no** `src/`, `test/`,
`tsconfig.json`, `node_modules`, `bun.lock`. Then `rm -rf native dist` (build artifacts).

- [ ] **Step 9: Commit** (build outputs are gitignored, so only source/config is staged):

```bash
git add bindings/typescript/package.json bindings/typescript/tsup.config.ts bindings/typescript/src/lib-path.ts bindings/typescript/bun.lock .gitignore
git commit -m "feat(npm): publishable @primeradianthq/obol — tsup build, native/ resolution, manifest"
```

---

## Task 2: Tag-triggered release workflow

**Files:**
- Create: `.github/workflows/release.yml`

- [ ] **Step 1: Write `.github/workflows/release.yml`:**

```yaml
name: Release

on:
  push:
    tags: ["v*"]

permissions:
  contents: write # create the GitHub Release
  id-token: write # OIDC for npm provenance / trusted publishing

concurrency:
  group: release-${{ github.ref }}
  cancel-in-progress: false # never cancel a half-done publish

jobs:
  dylibs:
    name: dylib (${{ matrix.name }})
    runs-on: ${{ matrix.os }}
    strategy:
      fail-fast: false
      matrix:
        include:
          - { os: macos-14, name: darwin-arm64, ext: dylib }
          - { os: macos-15-intel, name: darwin-x64, ext: dylib }
          - { os: ubuntu-24.04, name: linux-x64, ext: so }
          - { os: ubuntu-24.04-arm, name: linux-arm64, ext: so }
    steps:
      - uses: actions/checkout@v4
      - uses: jdx/mise-action@v4
      - name: Build release cdylib
        run: mise exec rust@1.96.0 -- cargo build --release -p obol-ffi
      - name: Strip (per-OS)
        run: |
          LIB="target/release/libobol_ffi.${{ matrix.ext }}"
          if [ "$RUNNER_OS" = "macOS" ]; then strip -x "$LIB"; else strip "$LIB"; fi
      - uses: actions/upload-artifact@v4
        with:
          name: dylib-${{ matrix.name }}
          path: target/release/libobol_ffi.${{ matrix.ext }}
          if-no-files-found: error

  publish:
    needs: dylibs
    runs-on: ubuntu-24.04
    steps:
      - uses: actions/checkout@v4
      - uses: actions/setup-node@v4
        with:
          node-version: "24"
          registry-url: "https://registry.npmjs.org"
      - name: Pin npm (trusted publishing + provenance need >= 11.5.1)
        run: npm install -g npm@^11.5.1
      - uses: oven-sh/setup-bun@v2
        with:
          bun-version: "1.3.11"
      - uses: actions/download-artifact@v4
        with:
          path: /tmp/dylibs
      - name: Assemble native/
        working-directory: bindings/typescript
        run: |
          mkdir -p native/darwin-arm64 native/darwin-x64 native/linux-x64 native/linux-arm64
          cp /tmp/dylibs/dylib-darwin-arm64/libobol_ffi.dylib native/darwin-arm64/
          cp /tmp/dylibs/dylib-darwin-x64/libobol_ffi.dylib   native/darwin-x64/
          cp /tmp/dylibs/dylib-linux-x64/libobol_ffi.so       native/linux-x64/
          cp /tmp/dylibs/dylib-linux-arm64/libobol_ffi.so     native/linux-arm64/
      - name: Build dist + stamp version
        working-directory: bindings/typescript
        run: |
          bun install
          bun x tsup
          npm pkg set version="${GITHUB_REF_NAME#v}"
      - name: Publish to npm
        working-directory: bindings/typescript
        # Prerelease versions (v1.2.3-rc.1) go to the `next` dist-tag, not `latest`.
        # Token path (bootstrap): setup-node's .npmrc carries the token. OIDC path (post-bootstrap,
        # secret deleted): remove .npmrc so an empty `_authToken=` line can't block OIDC activation
        # (npm >= 11.5.1 then authenticates via the trusted publisher). provenance:true works on
        # either path (public repo + id-token: write); no --provenance flag needed.
        run: |
          VERSION="${GITHUB_REF_NAME#v}"
          DIST_TAG="latest"; case "$VERSION" in *-*) DIST_TAG="next";; esac
          if [ -n "$NPM_TOKEN" ]; then
            npm publish --tag "$DIST_TAG"
          else
            rm -f "$HOME/.npmrc"
            npm publish --tag "$DIST_TAG"
          fi
        env:
          NPM_TOKEN: ${{ secrets.NPM_TOKEN }}
          NODE_AUTH_TOKEN: ${{ secrets.NPM_TOKEN }}
      - name: GitHub Release with dylibs
        uses: softprops/action-gh-release@v2
        with:
          files: /tmp/dylibs/dylib-*/libobol_ffi.*
          fail_on_unmatched_files: true
```

- [ ] **Step 2: actionlint.** `"$(go env GOPATH)/bin/actionlint" .github/workflows/release.yml` →
  clean (install via `go install github.com/rhysd/actionlint/cmd/actionlint@latest` if needed).

- [ ] **Step 3: Commit.**

```bash
git add .github/workflows/release.yml
git commit -m "ci: tag-triggered npm release — build 4 dylibs, assemble, publish + GitHub Release"
```

---

## Task 3: Install docs + release runbook

**Files:**
- Modify: `bindings/typescript/README.md`
- Create: `docs/RELEASING.md`

- [ ] **Step 1: Update `bindings/typescript/README.md`** install section to the published package:
  `npm install @primeradianthq/obol` / `bun add @primeradianthq/obol`; note it bundles native libs
  for macOS (arm64/x64) and Linux (x64/arm64) and needs no `cargo build` for consumers; the
  `OBOL_LIB`/`target` paths are dev-only. Note Node 18+.

- [ ] **Step 2: Write `docs/RELEASING.md`** — the runbook: tag `vX.Y.Z` on `main` → `release.yml`
  builds the 4 dylibs, assembles, publishes to npm, and cuts a GitHub Release. The **one-time
  bootstrap**: (a) create a granular `@primeradianthq` automation token, add it as repo secret
  `NPM_TOKEN`; (b) push the first tag — it publishes via the token and creates the package; (c) on
  npmjs.com, configure the package's Trusted Publisher → `prime-radiant-inc/obol`, workflow
  `release.yml`; (d) delete the `NPM_TOKEN` secret, then push a *patch* tag and **verify the first
  tokenless release succeeds** — if it fails with `ENEEDAUTH`, a stale `.npmrc` `_authToken=` line
  is the cause (the workflow removes it on the OIDC path, so this should not happen). Also document:
  trusted publishing + provenance need **npm ≥ 11.5.1** (the workflow pins it); **prerelease tags**
  (`v1.2.3-rc.1`) publish to the `next` dist-tag, not `latest`; and `version()` returns the Rust
  core version (`0.1.0`), not the npm package version.

- [ ] **Step 3: Commit.**

```bash
git add bindings/typescript/README.md docs/RELEASING.md
git commit -m "docs: npm install instructions + release runbook (token bootstrap → OIDC)"
```

---

## Execution: merge, then bootstrap (orchestrator)

- [ ] Push the branch, open a PR; the existing `ci.yml` must stay green (the package.json/lib-path
  changes don't break the source tests — verified in Task 1 Step 7). `gh run watch`.
- [ ] Merge to `main` (fast-forward). The `release.yml` does **not** fire on the merge (only on
  `v*` tags).
- [ ] **Hand to Matt for the one-time bootstrap** (per `docs/RELEASING.md`): the `NPM_TOKEN`
  secret, the first tag, the trusted-publisher config, then secret removal. The real publish is
  exercised by that first tag — I can't do it (it's his namespace + credential).
- [ ] Move PRI-2094 to In Review with the reflection.

## Self-review notes (plan author)

- **Spec coverage:** manifest + tsup + native resolution + gitignore = Task 1; release workflow
  (per-OS strip, assemble, version stamp, token-or-OIDC publish, GitHub Release) = Task 2; docs +
  bootstrap runbook = Task 3. All spec sections map.
- **Placeholder scan:** complete code/commands throughout; the only deferred-to-human step is
  Matt's token bootstrap, which is inherent (his credential/namespace), documented concretely in
  RELEASING.md, not a hand-wave.
- **Consistency:** the 4 matrix names (`darwin-arm64`/`darwin-x64`/`linux-x64`/`linux-arm64`)
  match the `native/<plat>-<arch>` dirs and `process.platform-process.arch`; `ext` (`dylib`/`so`)
  matches `libFilename()`; the strip-per-OS guard uses `RUNNER_OS` (`macOS`/`Linux`), consistent
  with `ci.yml`.
