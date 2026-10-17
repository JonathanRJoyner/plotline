// The browser tests in e2e/: one flow per GUI feature, in Chromium, against the built bundle that
// `npm run build` writes, served by `vite preview`. Run them with `npm run e2e`.
import { createServer, type AddressInfo } from "node:net";
import { defineConfig, devices } from "@playwright/test";

// A free port, asked of the system.
function freePort(): Promise<number> {
  return new Promise((done, fail) => {
    const server = createServer();
    server.once("error", fail);
    server.listen(0, () => {
      const { port } = server.address() as AddressInfo;
      server.close(() => done(port));
    });
  });
}

// PLOTLINE_E2E_PORT picks the port; without it the first load picks a free one and stores it, so the
// worker processes, which load this file again, use the same port. Two worktrees' runs never meet.
process.env.PLOTLINE_E2E_PORT ??= String(await freePort());
const port = Number(process.env.PLOTLINE_E2E_PORT);

export default defineConfig({
  testDir: "e2e",
  forbidOnly: !!process.env.CI,
  reporter: process.env.CI ? [["github"], ["html", { open: "never" }]] : "list",
  use: { baseURL: `http://localhost:${port}`, trace: "retain-on-failure" },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  // Never another worktree's server on the same port: it would test that tree's bundle.
  webServer: { command: `npx vite preview --port ${port} --strictPort`, port, reuseExistingServer: false },
});
