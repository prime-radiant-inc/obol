import { backend, type RawResult } from "./ffi.ts";
import { ObolError } from "./types.ts";
import type { CostEstimate, RefreshReport, Dialect } from "./types.ts";

function unwrap<T>(r: RawResult): T {
  if (r.code !== 0) {
    let kind = "Unknown";
    let message = "no detail";
    let code = r.code;
    if (r.json) {
      try {
        const e = (JSON.parse(r.json) as { error?: { code?: number; kind?: string; message?: string } }).error;
        if (e) {
          kind = e.kind ?? kind;
          message = e.message ?? message;
          code = e.code ?? code;
        }
      } catch {
        /* keep defaults */
      }
    }
    throw new ObolError(code, kind, message);
  }
  return JSON.parse(r.json as string) as T;
}

export async function version(): Promise<string> {
  return (await backend()).version();
}
export async function estimatePath(path: string, dialect: Dialect | null = null): Promise<CostEstimate> {
  return unwrap<CostEstimate>((await backend()).estimatePath(path, dialect));
}
export async function estimateBytes(data: Uint8Array, dialect: Dialect | null = null): Promise<CostEstimate> {
  return unwrap<CostEstimate>((await backend()).estimateBytes(data, dialect));
}
export async function refresh(asOf: string): Promise<RefreshReport> {
  return unwrap<RefreshReport>((await backend()).refresh(asOf));
}

export { ObolError } from "./types.ts";
export type { CostEstimate, ModelCost, TokenBuckets, Approximation, RefreshReport, Dialect } from "./types.ts";
