export interface TokenBuckets {
  input: number;
  output: number;
  cache_read: number;
  cache_write: number;
}
export interface ModelCost {
  model: string;
  provider: string;
  tokens: TokenBuckets;
  subtotal_usd: number;
}
export interface Approximation {
  kind: string;
  detail?: string;
}
export interface CostEstimate {
  total_usd: number;
  per_model: ModelCost[];
  tokens: TokenBuckets;
  unpriced_models: string[];
  approximations: Approximation[];
  pricing_as_of: string;
}
export interface RefreshReport {
  models: number;
  as_of: string;
  written_to: string;
}
export type Dialect = "claude" | "codex" | "pi";

export class ObolError extends Error {
  code: number;
  kind: string;
  constructor(code: number, kind: string, message: string) {
    super(`obol: ${kind} (code ${code}): ${message}`);
    this.name = "ObolError";
    this.code = code;
    this.kind = kind;
  }
}
