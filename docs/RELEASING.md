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

npm can't attach a trusted publisher to a package that doesn't exist yet, and npm has deprecated
classic (2FA-bypassing automation) tokens — so the **first** publish is a one-time **manual**
publish from a maintainer's machine. Every release after is tokenless OIDC via the CI workflow.

1. **Build the complete first-release tarball** (it needs all four platforms' dylibs, so it's
   assembled from a CI run, not one machine). Either run `release.yml` once to get the dylibs as
   artifacts, then assemble + `npm pack` locally with `publishConfig.provenance` removed (a manual
   publish has no CI OIDC to sign provenance) — or ask the assistant to produce it. Result:
   `primeradianthq-obol-<version>.tgz` containing `dist/`, `native/{darwin,linux}-{arm64,x64}/`,
   `package.json` (version set, `publishConfig: {access: public}`), `README.md`.
2. **Publish it manually**, logged into npm (answer the 2FA OTP interactively):
   `npm publish /path/to/primeradianthq-obol-<version>.tgz --access public`. This creates the
   package on the registry.
3. **Configure the trusted publisher** on npmjs.com for the now-existing `@primeradianthq/obol`:
   Package → Settings → Trusted Publisher → GitHub Actions, repo `prime-radiant-inc/obol`,
   workflow `release.yml`.
4. **From now on, releases are tokenless:** `git tag vX.Y.Z && git push origin vX.Y.Z` → the
   workflow builds the 4 dylibs, assembles, and `npm publish`es via OIDC (with provenance). No
   secret needed; delete any leftover `NPM_TOKEN`.

### Notes

- Trusted publishing + provenance require **npm ≥ 11.5.1**; the workflow pins it (`npm@^11.5.1`).
  Provenance is generated on the CI/OIDC releases (public repo + `id-token: write`); the one-time
  manual bootstrap publish has no provenance (no CI), which is why its tarball drops
  `publishConfig.provenance`.
- `version()` (the binding API) returns the **Rust core** version (`obol_version()`, e.g. `0.1.0`),
  which is intentionally decoupled from the npm package version stamped from the tag.

## Other registries

PyPI, crates.io, and Go publishing are not yet wired — separate follow-on tickets. The GitHub
Release dylibs (step 5 above) are the canonical artifacts those will reuse.
