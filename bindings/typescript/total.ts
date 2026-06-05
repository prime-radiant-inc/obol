import { estimatePath } from "./src/index.ts";

const path = process.argv[2];
const dialect = (process.argv[3] ?? null) as "claude" | "codex" | "pi" | null;
if (!path) {
  console.error("usage: total <transcript> [dialect]");
  process.exit(2);
}
const est = await estimatePath(path, dialect);
console.log(est.total_usd);
