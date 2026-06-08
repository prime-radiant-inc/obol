import { estimatePath, type Dialect } from "./src/index.ts";

const path = process.argv[2];
const dialect = process.argv[3] as Dialect | undefined;
if (!path || !dialect) {
  console.error("usage: total <transcript> <dialect>");
  process.exit(2);
}
const est = await estimatePath(path as string, dialect as Dialect);
console.log(est.total_usd);
