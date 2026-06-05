import { existsSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

function libFilename(): string {
  switch (process.platform) {
    case "darwin": return "libobol_ffi.dylib";
    case "win32": return "obol_ffi.dll";
    default: return "libobol_ffi.so";
  }
}

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
  // Dev: repo-relative target/{release,debug} (src -> typescript -> bindings -> repo).
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
