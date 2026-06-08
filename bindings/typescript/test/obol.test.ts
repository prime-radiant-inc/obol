import { test } from "node:test";
import assert from "node:assert/strict";
import { mkdtempSync, copyFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, dirname } from "node:path";
import { fileURLToPath } from "node:url";
import * as obol from "../src/index.ts";
import { setPricingDir, clearPricingDir } from "./pricing-env.ts";

const HERE = dirname(fileURLToPath(import.meta.url));
const TESTDATA = join(HERE, "..", "..", "testdata"); // test -> typescript -> bindings, then /testdata
const TRANSCRIPT = join(TESTDATA, "claude-mini.jsonl");

async function seed(): Promise<string> {
  const dir = mkdtempSync(join(tmpdir(), "obol-ts-"));
  copyFileSync(join(TESTDATA, "prices.json"), join(dir, "current.json"));
  await setPricingDir(dir);
  return dir;
}

test("version", async () => {
  assert.equal(await obol.version(), "0.2.1");
});

test("estimatePath success", async () => {
  const dir = await seed();
  try {
    const est = await obol.estimatePath(TRANSCRIPT, "claude");
    assert.ok(est.total_usd > 0, `total_usd=${est.total_usd}`);
    assert.equal(est.pricing_as_of, "2026-06-05");
  } finally {
    rmSync(dir, { recursive: true, force: true });
    await clearPricingDir();
  }
});

test("missing tables -> ObolError code 1", async () => {
  await setPricingDir("/nonexistent/obol-ts-xyz");
  try {
    await assert.rejects(
      () => obol.estimatePath(TRANSCRIPT, "claude"),
      (e: unknown) => e instanceof obol.ObolError && e.code === 1 && e.kind === "PricingTablesMissing",
    );
  } finally {
    await clearPricingDir();
  }
});

test("unknown dialect -> ObolError code 7", async () => {
  const dir = await seed();
  try {
    await assert.rejects(
      () => obol.estimatePath(TRANSCRIPT, "banana" as obol.Dialect),
      (e: unknown) => e instanceof obol.ObolError && e.code === 7,
    );
  } finally {
    rmSync(dir, { recursive: true, force: true });
    await clearPricingDir();
  }
});
