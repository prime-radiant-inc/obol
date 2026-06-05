import { defineConfig } from "tsup";

export default defineConfig({
  entry: ["src/index.ts"],
  format: ["esm"],
  dts: true,
  splitting: true, // keep ffi-bun / ffi-node as SEPARATE chunks (preserves the Bun/Node split)
  clean: true,
  outDir: "dist",
  external: ["bun:ffi", "koffi"], // bun:ffi is a Bun builtin; koffi stays a runtime dep
});
