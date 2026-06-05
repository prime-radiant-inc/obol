# Releasing

## npm — `@primeradianthq/obol` (the TypeScript binding)

Publishing is **tag-driven**. Push a tag `vX.Y.Z` on `main` and `.github/workflows/release.yml`:

1. builds the release cdylib on all four native runners (macOS arm64/x64, Linux x64/arm64) and
   strips it (`strip -x` on macOS, `strip` on Linux);
2. assembles `bindings/typescript/native/<platform>-<arch>/libobol_ffi.{dylib,so}`;
3. builds `dist/` with tsup and stamps the package version from the tag (`v1.2.3` → `1.2.3`);
4. publishes `@primeradianthq/obol` to npm (with provenance);
5. cuts a GitHub Release for the tag with the four dylibs attached.

Prerelease tags (`v1.2.3-rc.1`) publish to the `next` dist-tag, not `latest`.

### One-time bootstrap (first publish only)

npm can't attach a trusted publisher to a package that doesn't exist yet, so the **first** publish
uses a token; every release after is tokenless OIDC.

1. **Create a classic _Automation_ token** on npmjs.com (Access Tokens → Generate New Token →
   **Classic Token → Automation**). The **Automation** type is the one that **bypasses 2FA/OTP** —
   CI can't answer an OTP prompt, so a "Publish" classic token or a 2FA-bound granular token fails
   with `npm error code EOTP`. Add it as the repo secret **`NPM_TOKEN`** (Settings → Secrets and
   variables → Actions).
2. **Push the first tag** (`git tag v0.1.0 && git push origin v0.1.0`). The workflow publishes via
   the token and creates the package.
3. **Configure the trusted publisher** on npmjs.com for the now-existing `@primeradianthq/obol`:
   Package → Settings → Trusted Publisher → GitHub Actions, repo `prime-radiant-inc/obol`,
   workflow `release.yml`.
4. **Delete the `NPM_TOKEN` secret.** Push a patch tag and **verify the first tokenless release
   succeeds**. If it fails with `ENEEDAUTH`, a stale `.npmrc` `_authToken=` line is the cause — the
   workflow removes `~/.npmrc` on the no-token (OIDC) path specifically to avoid this, so it should
   not happen.

### Notes

- Trusted publishing + provenance require **npm ≥ 11.5.1**; the workflow pins it (`npm@^11.5.1`).
  Provenance works on either auth path because the repo is public and the job has `id-token: write`.
- `version()` (the binding API) returns the **Rust core** version (`obol_version()`, e.g. `0.1.0`),
  which is intentionally decoupled from the npm package version stamped from the tag.

## Other registries

PyPI, crates.io, and Go publishing are not yet wired — separate follow-on tickets. The GitHub
Release dylibs (step 5 above) are the canonical artifacts those will reuse.
