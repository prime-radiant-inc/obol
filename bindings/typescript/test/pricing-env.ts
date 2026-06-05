// Cross-runtime control of OBOL_PRICING_DIR for tests.
// Bun does NOT sync runtime `process.env` writes to the native env that bun:ffi's C calls (and
// Rust's getenv) observe, so under Bun we must call libc setenv/unsetenv. Node propagates
// process.env to getenv natively, so there process.env alone is enough.
const KEY = "OBOL_PRICING_DIR";
const isBun = typeof (globalThis as { Bun?: unknown }).Bun !== "undefined";

interface LibcEnv {
  setenv(k: Uint8Array, v: Uint8Array, overwrite: number): number;
  unsetenv(k: Uint8Array): number;
}
let libc: LibcEnv | undefined;
async function nativeEnv(): Promise<LibcEnv> {
  if (libc) return libc;
  const { dlopen, FFIType } = await import("bun:ffi");
  const name = process.platform === "darwin" ? "libSystem.dylib" : "libc.so.6";
  const { symbols } = dlopen(name, {
    setenv: { args: [FFIType.cstring, FFIType.cstring, FFIType.i32], returns: FFIType.i32 },
    unsetenv: { args: [FFIType.cstring], returns: FFIType.i32 },
  });
  libc = symbols as unknown as LibcEnv;
  return libc;
}

export async function setPricingDir(dir: string): Promise<void> {
  process.env[KEY] = dir;
  if (isBun) (await nativeEnv()).setenv(Buffer.from(KEY + "\0"), Buffer.from(dir + "\0"), 1);
}

export async function clearPricingDir(): Promise<void> {
  delete process.env[KEY];
  if (isBun) (await nativeEnv()).unsetenv(Buffer.from(KEY + "\0"));
}
