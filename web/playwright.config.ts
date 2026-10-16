// The browser tests in e2e/: one flow per GUI feature, in Chromium, against the built bundle that
// `npm run build` writes, served by `vite preview`. Run them with `npm run e2e`.
import { defineConfig, devices } from "@playwright/test";

const port = 4317;

export default defineConfig({
  testDir: "e2e",
  forbidOnly: !!process.env.CI,
  reporter: process.env.CI ? [["github"], ["html", { open: "never" }]] : "list",
  use: { baseURL: `http://localhost:${port}`, trace: "retain-on-failure" },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  // Never another worktree's server on the same port: it would test that tree's bundle.
  webServer: { command: `npx vite preview --port ${port} --strictPort`, port, reuseExistingServer: false },
});
