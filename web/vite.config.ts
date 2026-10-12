import { copyFileSync } from "node:fs";
import { resolve } from "node:path";
import preact from "@preact/preset-vite";
import { defineConfig } from "vitest/config";

const outDir = "../src/plotline/web";

export default defineConfig({
  plugins: [
    preact(),
    {
      // The built bundle contains preact, so ship its licence beside it.
      name: "preact-licence",
      closeBundle() {
        copyFileSync("node_modules/preact/LICENSE", resolve(outDir, "preact-LICENSE.txt"));
      },
    },
  ],
  base: "./",
  build: { outDir, emptyOutDir: true },
  worker: { format: "es" },
  test: { environment: "node", include: ["src/**/*.test.ts"] },
});
