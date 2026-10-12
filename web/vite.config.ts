import { copyFileSync } from "node:fs";
import { resolve } from "node:path";
import preact from "@preact/preset-vite";
import { defineConfig } from "vitest/config";

const outDir = "../src/plotline/web";

export default defineConfig({
  plugins: [
    preact(),
    {
      // The built bundle contains preact and the lucide icons it uses, so ship their licences beside it.
      name: "licences",
      closeBundle() {
        for (const name of ["preact", "lucide-preact"]) copyFileSync(`node_modules/${name}/LICENSE`, resolve(outDir, `${name}-LICENSE.txt`));
      },
    },
  ],
  base: "./",
  // No modulepreload polyfill: every browser with module workers has modulepreload, and preact
  // and the icons stay the only third-party code in the bundle.
  build: { outDir, emptyOutDir: true, modulePreload: { polyfill: false } },
  worker: { format: "es" },
  test: { environment: "node", include: ["src/**/*.test.ts"] },
});
