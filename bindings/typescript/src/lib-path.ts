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
  // this file: bindings/typescript/src/lib-path.ts — repo root is three up from src/
  const here = dirname(fileURLToPath(import.meta.url));
  const repo = join(here, "..", "..", ".."); // src -> typescript -> bindings -> repo
  for (const profile of ["release", "debug"]) {
    const p = join(repo, "target", profile, name);
    tried.push(p);
    if (existsSync(p)) return p;
  }
  throw new Error(
    "obol_ffi shared library not found. Set OBOL_LIB or run `cargo build -p obol-ffi`. Tried:\n  " +
      tried.join("\n  "),
  );
}
