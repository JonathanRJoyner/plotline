// One flow per user-visible GUI feature, in a real browser against the built bundle. A new
// feature adds its flow here, as it adds its tour step. Every test also fails on a console error.
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { expect, test as base, type Page } from "@playwright/test";

const test = base.extend<{ errors: string[] }>({
  errors: [
    async ({ page }, use) => {
      const errors: string[] = [];
      page.on("console", (m) => { if (m.type() === "error") errors.push(m.text()); });
      page.on("pageerror", (e) => errors.push(e.message));
      await use(errors);
      expect(errors, "console errors").toEqual([]);
    },
    { auto: true },
  ],
});

// The tour opens on a browser's first visit; marked seen, so its card covers nothing.
async function open(page: Page, { tour = false } = {}) {
  if (!tour) await page.addInitScript(() => localStorage.setItem("plotline.tour", "seen"));
  await page.goto("/");
  await expect(page.getByRole("button", { name: "Sample image", exact: true })).toBeVisible();
}

const status = (page: Page) => page.locator(".status > span").first();
// The Style button shows the current style, by name in its value, and opens the style picker.
const styleButton = (page: Page) => page.getByRole("button", { name: /^Style: / });
const currentStyle = async (page: Page) => (await styleButton(page).getAttribute("value"))!;
const picker = (page: Page) => page.locator("dialog").filter({ has: page.getByRole("heading", { name: "Styles", exact: true }) });
const card = (page: Page, style: string) => picker(page).locator(`.picker button[value="${style}"]`);

// Waits for a run newer than `after` to end, expects it done, and returns its id.
async function ran(page: Page, after: number): Promise<number> {
  let id = 0;
  let state = "";
  await expect.poll(async () => {
    const m = /^Run (\d+): (\w+)/.exec((await status(page).textContent()) ?? "");
    [id, state] = [Number(m?.[1] ?? 0), m?.[2] ?? ""];
    return id > after && state !== "running";
  }, { timeout: 60_000 }).toBe(true);
  expect(await status(page).textContent()).toBe(`Run ${id}: done`);
  return id;
}

async function sample(page: Page): Promise<number> {
  await open(page);
  await page.getByRole("button", { name: "Sample image", exact: true }).click();
  return ran(page, 0);
}

async function pick(page: Page, style: string, after: number): Promise<number> {
  await styleButton(page).click();
  await card(page, style).click();
  await expect(picker(page)).toBeHidden();
  return ran(page, after);
}

// The Fit and optimize panel starts closed; a result opens it again by itself.
const fitPanel = (page: Page) => page.locator("details.fit-panel");
const fitSummary = (page: Page) => page.locator("details.fit-panel > summary");

// The styles in the picker's order, read with the picker open, which it then closes.
async function styleNames(page: Page): Promise<string[]> {
  await styleButton(page).click();
  const names = await picker(page).locator(".picker button[value]").evaluateAll((bs) => bs.map((b) => (b as HTMLButtonElement).value));
  await page.keyboard.press("Escape");
  await expect(picker(page)).toBeHidden();
  return names;
}

test("the empty preview offers the sample image and opening one", async ({ page }) => {
  await open(page);
  const card = page.locator(".preview .empty");
  await expect(card).toContainText("Start with an image");
  await expect(card.getByRole("button", { name: "Use the sample image" })).toBeVisible();
  await expect(card.locator("label.button")).toContainText("Open an image");
  await expect(card).toContainText("Or drop or paste a PNG, JPEG or WebP anywhere on the page.");
  // On a phone the card stays inside the preview.
  await page.setViewportSize({ width: 375, height: 800 });
  const [inner, outer] = [(await card.boundingBox())!, (await page.locator(".preview").boundingBox())!];
  expect(inner.x).toBeGreaterThanOrEqual(outer.x);
  expect(inner.x + inner.width).toBeLessThanOrEqual(outer.x + outer.width);
  await card.getByRole("button", { name: "Use the sample image" }).click();
  await ran(page, 0);
  await expect(card).toHaveCount(0);
  // The other way in: the card's own file input takes an image.
  await page.reload();
  await expect(card).toBeVisible();
  await card.locator("input[type=file]").setInputFiles(await coloursPng(page));
  await ran(page, 0);
  await expect(card).toHaveCount(0);
});

test("the sample image draws in every style", async ({ page }) => {
  test.setTimeout(180_000);
  let id = await sample(page);
  const styles = await styleNames(page);
  expect(styles.length).toBeGreaterThan(10);
  const first = await currentStyle(page);
  // `learned` needs a downloaded model, so it draws nothing here.
  for (const style of styles.filter((s) => s !== first && s !== "learned")) {
    id = await pick(page, style, id);
    await expect(page.locator(".stats"), style).toContainText(/paths?/);
  }
});

test("a style switch keeps a changed setting, and Reset to defaults restores it", async ({ page }) => {
  let id = await pick(page, "stipple", await sample(page));
  const count = page.getByRole("spinbutton", { name: /^count$/i });
  const initial = await count.inputValue();
  const changed = String(Math.round(Number(initial) / 2));
  await count.fill(changed);
  await count.blur();
  id = await ran(page, id);
  id = await pick(page, "line", id);
  await expect(count).toHaveValue(changed);
  await page.getByRole("button", { name: "Reset to defaults" }).click();
  await ran(page, id);
  await expect(count).toHaveValue(initial);
});

test("an exported SVG loads back as a preset", async ({ page }) => {
  let id = await pick(page, "stipple", await sample(page));
  const count = page.getByRole("spinbutton", { name: /^count$/i });
  const initial = await count.inputValue();
  const changed = String(Math.round(Number(initial) / 2));
  await count.fill(changed);
  await count.blur();
  id = await ran(page, id);
  const [download] = await Promise.all([page.waitForEvent("download"), page.getByRole("button", { name: "SVG", exact: true }).click()]);
  const svg = readFileSync(await download.path());
  await page.getByRole("button", { name: "Reset to defaults" }).click();
  id = await ran(page, id);
  await expect(count).toHaveValue(initial);
  id = await pick(page, "line", id);
  await page.getByLabel("Load preset").setInputFiles({ name: download.suggestedFilename(), mimeType: "image/svg+xml", buffer: svg });
  await ran(page, id);
  await expect(styleButton(page)).toHaveAttribute("value", "stipple");
  await expect(count).toHaveValue(changed);
});

test("a copied link opens the same setup in a fresh browser, and the fragment is gone from its address", async ({ page, browser }) => {
  let id = await pick(page, "stipple", await sample(page));
  const count = page.getByRole("spinbutton", { name: /^count$/i });
  const changed = String(Math.round(Number(await count.inputValue()) / 2));
  await count.fill(changed);
  await count.blur();
  id = await ran(page, id);
  await page.context().grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.getByRole("button", { name: "Copy link", exact: true }).click();
  await expect(page.getByRole("status")).toHaveText("Link copied.");
  const link = await page.evaluate(() => navigator.clipboard.readText());
  expect(link).toMatch(/^http:\/\/localhost:\d+\/#p=[A-Za-z0-9_-]+$/);
  // A setup change makes the notice stale: it goes.
  await count.fill(String(Number(changed) + 100));
  await count.blur();
  await ran(page, id);
  await expect(page.getByRole("status")).toHaveCount(0);
  // A fresh context remembers no setup, so what loads can only have come from the link.
  const fresh = await browser.newContext();
  try {
    await fresh.addInitScript(() => localStorage.setItem("plotline.tour", "seen"));
    const other = await fresh.newPage();
    const failures: string[] = [];
    other.on("pageerror", (e) => failures.push(e.message));
    other.on("console", (m) => { if (m.type() === "error") failures.push(m.text()); });
    await other.goto(link);
    await expect(styleButton(other)).toHaveAttribute("value", "stipple");
    await expect(other.getByRole("spinbutton", { name: /^count$/i })).toHaveValue(changed);
    expect(new URL(other.url()).hash).toBe("");
    expect(failures).toEqual([]);
  } finally {
    await fresh.close();
  }
});

test("Copy link shows the link to copy by hand when the clipboard refuses", async ({ page }) => {
  await page.addInitScript(() => Object.defineProperty(navigator, "clipboard", { value: { writeText: () => Promise.reject(new Error("denied")) } }));
  await open(page);
  await page.getByRole("button", { name: "Copy link", exact: true }).click();
  const field = page.getByRole("textbox", { name: "Share link" });
  await expect(field).toHaveValue(/^http:\/\/localhost:\d+\/#p=[A-Za-z0-9_-]+$/);
  await expect(page.getByRole("status")).toHaveCount(0);
});

test("a link that cannot be read says so and leaves the remembered setup", async ({ page }) => {
  await pick(page, "stipple", await sample(page));
  // The page remembers a setup shortly after each change.
  await expect.poll(() => page.evaluate(() => localStorage.getItem("plotline.preset"))).toContain('"stipple"');
  await page.goto("/#p=AAAA");
  await page.reload(); // a change of fragment alone does not load the page again
  await expect(page.locator("fieldset[data-tour=presets] .error")).toContainText("could not be read");
  await expect(styleButton(page)).toHaveAttribute("value", "stipple");
  expect(new URL(page.url()).hash).toBe("");
});

test("a look changes the density setting, Undo takes it back, and Reset to defaults shows Balanced", async ({ page }) => {
  let id = await sample(page);
  // The style the page opens on has a Fine look; the menu lists the labels the engine gives.
  const look = page.getByRole("combobox", { name: "Look", exact: true });
  const shown = look.locator("option:checked");
  const spacing = page.getByRole("spinbutton", { name: /^spacing( mm)?$/i });
  await expect(look.getByRole("option", { name: "Fine" })).toHaveCount(1);
  await expect(shown).toHaveText("Balanced");
  const balanced = await spacing.inputValue();
  await look.selectOption({ label: "Fine" });
  id = await ran(page, id);
  await expect(shown).toHaveText("Fine");
  await expect(spacing).not.toHaveValue(balanced);
  await page.getByRole("button", { name: "Undo", exact: true }).click();
  id = await ran(page, id);
  await expect(shown).toHaveText("Balanced");
  await expect(spacing).toHaveValue(balanced);
  // A changed setting is no look, and the menu then shows Custom.
  await spacing.fill("1.7");
  await spacing.blur();
  id = await ran(page, id);
  await expect(shown).toHaveText("Custom");
  await look.selectOption({ label: "Fine" });
  id = await ran(page, id);
  await expect(shown).toHaveText("Fine");
  await page.getByRole("button", { name: "Reset to defaults" }).click();
  await ran(page, id);
  await expect(shown).toHaveText("Balanced");
  await expect(spacing).toHaveValue(balanced);
});

test("a style with no look but Balanced hides the Look menu", async ({ page }) => {
  const id = await sample(page);
  await expect(page.getByRole("combobox", { name: "Look", exact: true })).toBeVisible();
  await pick(page, "ink", id);
  await expect(page.getByRole("combobox", { name: "Look", exact: true })).toHaveCount(0);
});

// Starts Fit to target for three quarters of the hatch drawing's plot time, and returns the target in minutes.
async function fitHatch(page: Page): Promise<number> {
  await pick(page, "hatch", await sample(page));
  const [, min, s] = /about (\d+) min (\d+) s/.exec((await page.locator(".stats").textContent()) ?? "") ?? [];
  const target = Math.round(((Number(min) * 60 + Number(s)) / 60) * 0.75 * 10) / 10;
  await expect(fitPanel(page)).not.toHaveAttribute("open", "");
  await fitSummary(page).click();
  await page.getByRole("spinbutton", { name: "Target", exact: true }).fill(String(target));
  await page.getByRole("combobox", { name: "Target unit" }).selectOption("minutes");
  await page.getByRole("button", { name: "Fit to target" }).click();
  return target;
}

test("Fit to target meets its target", async ({ page }) => {
  test.setTimeout(120_000);
  const target = await fitHatch(page);
  const fitted = page.locator("form.fit span");
  await expect(fitted).toContainText("for a target of", { timeout: 100_000 });
  const [, achieved, aimed] = /: ([\d.e+-]+) min for a target of ([\d.]+) min/.exec((await fitted.textContent()) ?? "") ?? [];
  expect(Number(aimed)).toBe(target);
  // The solver stops within 2 % of the target (TOLERANCE in crates/core/src/fit.rs).
  expect(Math.abs(Number(achieved) / target - 1)).toBeLessThanOrEqual(0.02);
});

test("a Fit message clears after Undo", async ({ page }) => {
  test.setTimeout(120_000);
  await fitHatch(page);
  const spacing = page.getByRole("spinbutton", { name: /^spacing( mm)?$/i });
  const fitted = page.locator("form.fit span");
  // The message shows once the fit's value is applied, so the spacing holds it by then.
  await expect(fitted).toContainText("for a target of", { timeout: 100_000 });
  const chosen = await spacing.inputValue();
  await page.getByRole("button", { name: "Undo", exact: true }).click();
  await expect(spacing).not.toHaveValue(chosen);
  await expect(fitted).toHaveCount(0);
});

test("a plot lint finding's Apply clears it", async ({ page }) => {
  let id = await pick(page, "hatch", await sample(page));
  // Lines closer than the 0.3 mm default pen overlap; Apply sets the spacing to the pen's width.
  const spacing = page.getByRole("spinbutton", { name: /^spacing( mm)?$/i });
  await spacing.fill("0.2");
  await spacing.blur();
  id = await ran(page, id);
  const apply = page.getByRole("button", { name: /^Apply: Marks .* closer than/ });
  await expect(apply).toBeVisible();
  await apply.click();
  await ran(page, id);
  await expect(spacing).toHaveValue("0.3");
  await expect(apply).toHaveCount(0);
});

// Four saturated stripes, drawn in the page and encoded as a PNG, as a file for setInputFiles.
async function coloursPng(page: Page) {
  const png = await page.evaluate(async () => {
    const canvas = document.createElement("canvas");
    canvas.width = 240;
    canvas.height = 320;
    const g = canvas.getContext("2d")!;
    ["#e00000", "#00b000", "#0000e0", "#f0e000"].forEach((c, i) => { g.fillStyle = c; g.fillRect(i * 60, 0, 60, 320); });
    const blob = await new Promise<Blob>((done) => canvas.toBlob((b) => done(b!), "image/png"));
    return Array.from(new Uint8Array(await blob.arrayBuffer()));
  });
  return { name: "colours.png", mimeType: "image/png", buffer: Buffer.from(png) };
}

test("a colour image's palette suggestion sets the pens and Pen use", async ({ page }) => {
  await open(page);
  await page.locator(".toolbar input[type=file]").setInputFiles(await coloursPng(page));
  const id = await ran(page, 0);
  const apply = page.getByRole("button", { name: /^Apply: This is a colour image\./ });
  await expect(apply).toBeVisible();
  await apply.click();
  await ran(page, id);
  const names = page.locator(".pen .name");
  await expect.poll(() => names.count()).toBeGreaterThanOrEqual(2);
  expect(await names.count()).toBeLessThanOrEqual(4);
  for (const name of await names.allTextContents()) expect(name).toMatch(/ 0\.3$/);
  await expect(page.getByRole("combobox", { name: "Pen use" })).toHaveValue("by_colour");
  await expect(apply).toHaveCount(0);
});

test("Reset to defaults keeps Pen use with several pens, and a light first pen alone warns", async ({ page }) => {
  await open(page);
  await page.locator(".toolbar input[type=file]").setInputFiles(await coloursPng(page));
  let id = await ran(page, 0);
  await page.getByRole("button", { name: /^Apply: This is a colour image\./ }).click();
  id = await ran(page, id);
  const penUse = page.getByRole("combobox", { name: "Pen use" });
  await expect(penUse).toHaveValue("by_colour");
  // Pen use goes with the pens, which a reset leaves alone.
  await page.getByRole("button", { name: "Reset to defaults" }).click();
  id = await ran(page, id);
  await expect(penUse).toHaveValue("by_colour");
  // The palette lists the lightest ink first, so one pen leaves it to draw alone.
  await penUse.selectOption("one_pen");
  await ran(page, id);
  const light = page.locator(".lint li[data-severity=warning]").filter({ hasText: /only pen 1 \(.+\) draws, and a light ink alone shows little/ });
  await expect(light).toHaveCount(1);
});

test("a framing suggestion crops the image to its subject, and Remove crop takes it off", async ({ page }) => {
  await open(page);
  // A dark disc up and to the right on a light field, drawn in the page and encoded as a PNG.
  const png = await page.evaluate(async () => {
    const canvas = document.createElement("canvas");
    canvas.width = 256;
    canvas.height = 256;
    const g = canvas.getContext("2d")!;
    g.fillStyle = "#f2f2f2";
    g.fillRect(0, 0, 256, 256);
    g.fillStyle = "#1a1a1a";
    g.beginPath();
    g.arc(0.7 * 256, 0.3 * 256, 0.05 * 256, 0, 2 * Math.PI);
    g.fill();
    const blob = await new Promise<Blob>((done) => canvas.toBlob((b) => done(b!), "image/png"));
    return Array.from(new Uint8Array(await blob.arrayBuffer()));
  });
  await page.locator(".toolbar input[type=file]").setInputFiles({ name: "disc.png", mimeType: "image/png", buffer: Buffer.from(png) });
  const id = await ran(page, 0);
  const pagePanel = page.locator('[data-tour="page"]');
  const apply = page.getByRole("button", { name: /^Apply: The subject fills only part of the image\./ });
  const remove = page.getByRole("button", { name: "Remove crop" });
  await expect(apply).toBeVisible();
  await expect(remove).toHaveCount(0);
  await apply.click();
  const cropped = await ran(page, id);
  await expect(pagePanel).toContainText("Cropped");
  await expect(apply).toHaveCount(0);
  // Undo takes the crop back like any page change, and Remove crop does too.
  await page.getByRole("button", { name: "Undo", exact: true }).click();
  const undone = await ran(page, cropped);
  await expect(remove).toHaveCount(0);
  await expect(apply).toBeVisible();
  await apply.click();
  await ran(page, undone);
  await remove.click();
  await expect(remove).toHaveCount(0);
  await expect(apply).toBeVisible();
});

test("a suggestion can be dismissed, and comes back with the next image", async ({ page }) => {
  await open(page);
  const file = page.locator(".toolbar input[type=file]");
  await file.setInputFiles(await coloursPng(page));
  const id = await ran(page, 0);
  const suggestions = page.getByRole("group", { name: "Suggestions" });
  const apply = page.getByRole("button", { name: /^Apply: This is a colour image\./ });
  await expect(apply).toBeVisible();
  // The cross is named for its reason and hides only that suggestion.
  const dismiss = page.getByRole("button", { name: /^Dismiss: This is a colour image\./ });
  await dismiss.click();
  await expect(apply).toHaveCount(0);
  // The panel goes with its last suggestion, and a redraw does not bring the dismissed one back.
  await expect(suggestions).toHaveCount(0);
  await pick(page, "hatch", id);
  await expect(apply).toHaveCount(0);
  // A new image starts with every suggestion.
  await file.setInputFiles(await coloursPng(page));
  await expect(apply).toBeVisible();
});

test("the stats read in metres and a single path", async ({ page }) => {
  const id = await pick(page, "line", await sample(page));
  const stats = page.locator(".stats");
  // The line style draws one path, whose length is metres.
  await expect(stats).toContainText(/^1 path · pen down \d+\.\d{2} m · pen up (\d+\.\d mm|\d+\.\d{2} m) · about \d+ min \d+ s to plot/);
  // Thousands of paths are grouped.
  await pick(page, "stipple", id);
  await expect(stats).toContainText(/^\d{1,3}(,\d{3})+ paths · /);
});

test("the stats show the tone match", async ({ page }) => {
  await sample(page);
  await expect(page.locator(".stats")).toContainText(/tone error \d\.\d{3} · SSIM -?\d\.\d{2}/);
});

test("Optimize suggests a spacing and changes nothing until Apply", async ({ page }) => {
  test.setTimeout(120_000);
  const id = await pick(page, "hatch", await sample(page));
  const spacing = page.getByRole("spinbutton", { name: /^spacing( mm)?$/i });
  const before = await spacing.inputValue();
  await fitSummary(page).click();
  await page.getByRole("button", { name: "Optimize" }).click();
  // The search is a run of its own, which ends with the suggestion.
  const searched = await ran(page, id);
  await expect(fitPanel(page)).toHaveAttribute("open", "");
  const found = page.locator("form.fit span");
  await expect(found).toContainText("follows the image's tones best");
  await expect(found).toContainText(/\d min to plot/);
  await expect(spacing).toHaveValue(before);
  // The still life's best hatch spacing is wider than the default (brief §14, "Optimize").
  const [, value] = /^Spacing at ([\d.]+) mm follows/.exec((await found.textContent()) ?? "") ?? [];
  expect(Number(value)).toBeGreaterThan(Number(before));
  // A changed setting takes the suggestion back, and the settings as they were bring it again.
  await spacing.fill((Number(before) + 0.1).toFixed(2));
  await spacing.blur();
  const changed = await ran(page, searched);
  await expect(found).toHaveCount(0);
  await spacing.fill(before);
  await spacing.blur();
  const restored = await ran(page, changed);
  await expect(found).toContainText("follows the image's tones best");
  await page.getByRole("button", { name: /^Apply: Spacing at/ }).click();
  await ran(page, restored);
  await expect(spacing).toHaveValue(value);
});

test("Optimize names a closer spacing the time limit left out", async ({ page }) => {
  test.setTimeout(120_000);
  await open(page);
  // A slow pen lift puts the denser spacings over the hour.
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const dialog = page.locator("dialog").filter({ has: page.getByRole("heading", { name: "Settings" }) });
  await dialog.getByRole("button", { name: "Plot time", exact: true }).click();
  const lift = dialog.getByRole("spinbutton", { name: /^Pen lift/ });
  await lift.fill("40");
  await lift.blur();
  await dialog.getByRole("button", { name: "Close" }).click();
  await page.getByRole("button", { name: "Sample image", exact: true }).click();
  const id = await pick(page, "hatch", await ran(page, 0));
  const spacing = page.getByRole("spinbutton", { name: /^spacing( mm)?$/i });
  await spacing.fill("8");
  await spacing.blur();
  const sparse = await ran(page, id);
  await fitSummary(page).click();
  await page.getByRole("button", { name: "Optimize" }).click();
  await ran(page, sparse);
  const found = page.locator("form.fit span");
  await expect(found).toContainText("follows the image's tones best for its plot time");
  await expect(found).toContainText(/Spacing at [\d.]+ mm follows them more closely, with a tone error of \d\.\d{3}, but takes [\d.]+ min to plot, over the limit of [\d.]+ min: type a target in minutes to allow it\./);
});

// A white picture, on which the line styles draw nothing.
async function whitePng(page: Page) {
  const png = await page.evaluate(async () => {
    const canvas = document.createElement("canvas");
    canvas.width = 120;
    canvas.height = 160;
    const g = canvas.getContext("2d")!;
    g.fillStyle = "#ffffff";
    g.fillRect(0, 0, 120, 160);
    const blob = await new Promise<Blob>((done) => canvas.toBlob((b) => done(b!), "image/png"));
    return Array.from(new Uint8Array(await blob.arrayBuffer()));
  });
  return { name: "white.png", mimeType: "image/png", buffer: Buffer.from(png) };
}

test("Optimize says why it has no suggestion, the stages end done, and a new image clears the sentence", async ({ page }) => {
  test.setTimeout(120_000);
  await open(page);
  await page.locator(".toolbar input[type=file]").setInputFiles(await whitePng(page));
  const id = await pick(page, "hatch", await ran(page, 0));
  await fitSummary(page).click();
  await page.getByRole("button", { name: "Optimize" }).click();
  const searched = await ran(page, id);
  const found = page.locator("form.fit span");
  await expect(found).toContainText(/^No value of Spacing from [\d.]+ mm to [\d.]+ mm draws anything here/);
  await expect(page.getByRole("button", { name: /^Apply: / })).toHaveCount(0);
  // The search ends on the current settings' cached drawing, which sends no stage frames.
  const chips = page.locator(".status li");
  expect(await chips.count()).toBeGreaterThan(0);
  await expect(chips.filter({ hasText: "waiting" })).toHaveCount(0);
  // The same settings on another image: the sentence was about the last one.
  await page.getByRole("button", { name: "Sample image", exact: true }).click();
  await ran(page, searched);
  await expect(found).toHaveCount(0);
});

test("Response auto corrects the tone map for the style", async ({ page }) => {
  const id = await pick(page, "hatch", await sample(page));
  const stats = await page.locator(".stats").textContent();
  await page.locator("summary", { hasText: /^Image$/ }).click();
  await page.getByRole("combobox", { name: /^response$/i }).selectOption("auto");
  await ran(page, id);
  await expect(page.locator(".stats")).not.toHaveText(stats ?? "");
});

test("Focus marks points on the preview, a click on one removes it, and Undo brings it back", async ({ page }) => {
  let id = await sample(page);
  await page.locator("summary", { hasText: /^Image$/ }).click();
  const rings = page.locator(".focus-rings .ring");
  const count = page.locator(".focus-line");
  await expect(rings).toHaveCount(0);
  await expect(count).toHaveCount(0);
  // Focus mode: a click on the drawn image, the middle of the page, marks a point there.
  await page.locator("label.chip", { hasText: "Focus" }).click();
  await page.locator(".preview canvas").click();
  await expect(count).toContainText("1 focus point");
  await expect(rings).toHaveCount(1);
  id = await ran(page, id);
  // A second point, off the first and away from its removal target, and the count says so.
  const box = (await page.locator(".preview canvas").boundingBox())!;
  await page.locator(".preview canvas").click({ position: { x: box.width / 2 + 120, y: box.height / 2 - 80 } });
  await expect(count).toContainText("2 focus points");
  await expect(rings).toHaveCount(2);
  id = await ran(page, id);
  // A click on a point removes it; Clear takes the rest.
  await page.getByRole("button", { name: "Remove focus point 1" }).click();
  await expect(count).toContainText("1 focus point");
  await expect(rings).toHaveCount(1);
  id = await ran(page, id);
  await page.getByRole("button", { name: "Remove focus point 1" }).click();
  await expect(rings).toHaveCount(0);
  await expect(count).toHaveCount(0);
  id = await ran(page, id);
  await page.getByRole("button", { name: "Undo", exact: true }).click();
  await expect(rings).toHaveCount(1);
  await expect(count).toContainText("1 focus point");
  id = await ran(page, id);
  await page.getByRole("button", { name: "Clear", exact: true }).click();
  await expect(rings).toHaveCount(0);
  await ran(page, id);
});

test("Two quick clicks in Focus mode both land", async ({ page }) => {
  const id = await sample(page);
  await page.locator("label.chip", { hasText: "Focus" }).click();
  const box = (await page.locator(".preview canvas").boundingBox())!;
  const [x, y] = [box.x + box.width / 2, box.y + box.height / 2];
  // Sent together, so the second press arrives before the engine has answered the first.
  await Promise.all([page.mouse.click(x - 100, y), page.mouse.click(x + 100, y), page.mouse.click(x, y + 150)]);
  await expect(page.locator(".focus-rings .ring")).toHaveCount(3);
  await ran(page, id);
  await page.locator("summary", { hasText: /^Image$/ }).click();
  await expect(page.locator(".focus-line")).toContainText("3 focus points");
});

test("Enter on the focused preview marks a focus point while Focus is on", async ({ page }) => {
  let id = await sample(page);
  const canvas = page.locator(".preview canvas");
  const rings = page.locator(".focus-rings .ring");
  const aim = () => canvasHash(page, 1 / 3, 2 / 3); // the middle third holds the canvas centre
  const off = await aim();
  // With Focus off Enter does nothing.
  await canvas.focus();
  await page.keyboard.press("Enter");
  await page.locator("label.chip", { hasText: "Focus" }).click();
  // With it on the canvas draws a crosshair at its centre, and says what Enter does.
  await expect.poll(aim).not.toBe(off);
  await expect(canvas).toHaveAttribute("aria-label", /, Enter marks a focus point at the centre$/);
  await canvas.focus();
  await page.keyboard.press("Enter");
  await expect(rings).toHaveCount(1);
  id = await ran(page, id);
  const box = (await canvas.boundingBox())!;
  const at = async (i: number) => [Number(await rings.nth(i).getAttribute("cx")), Number(await rings.nth(i).getAttribute("cy"))];
  const [x, y] = await at(0);
  expect(Math.abs(x - box.width / 2)).toBeLessThan(3);
  expect(Math.abs(y - box.height / 2)).toBeLessThan(3);
  // The arrow keys pan, so the next point is marked elsewhere on the page, again at the centre.
  await page.keyboard.press("ArrowRight");
  await page.keyboard.press("Enter");
  await expect(rings).toHaveCount(2);
  await ran(page, id);
  const [x1, y1] = await at(0);
  const [x2, y2] = await at(1);
  expect(Math.abs(x1 - (box.width / 2 - 40))).toBeLessThan(3);
  expect(Math.abs(x2 - box.width / 2)).toBeLessThan(3);
  expect(Math.abs(y2 - y1)).toBeLessThan(3);
  await page.locator("summary", { hasText: /^Image$/ }).click();
  await expect(page.locator(".focus-line")).toContainText("2 focus points");
  // Focus off drops the hint again.
  await page.locator("label.chip", { hasText: "Focus" }).click();
  await expect(canvas).toHaveAttribute("aria-label", /0 fits the page$/);
});

test("Flatten paper changes the drawing", async ({ page }) => {
  const id = await pick(page, "ink", await sample(page));
  const stats = await page.locator(".stats").textContent();
  await page.locator("summary", { hasText: /^Image$/ }).click();
  const flatten = page.getByRole("spinbutton", { name: /^flatten paper$/i });
  await flatten.fill("10");
  await flatten.blur();
  await ran(page, id);
  await expect(page.locator(".stats")).not.toHaveText(stats ?? "");
});

test("Smooth, keep edges changes the drawing", async ({ page }) => {
  const id = await pick(page, "contours", await sample(page));
  const stats = await page.locator(".stats").textContent();
  const image = page.locator("[data-tour=image-settings]");
  await image.locator("summary", { hasText: /^Image$/ }).click();
  await image.locator("summary", { hasText: /^Advanced$/ }).click();
  const smooth = image.getByRole("spinbutton", { name: /^smooth, keep edges$/i });
  await smooth.fill("1");
  await smooth.blur();
  await ran(page, id);
  await expect(page.locator(".stats")).not.toHaveText(stats ?? "");
});

test("a dot style's marks show at the fit zoom, where the pen is under a pixel", async ({ page }) => {
  const id = await sample(page);
  await pick(page, "grid", id);
  await expect(page.getByText("the pen is under a pixel wide")).toBeVisible();
  // Grey pixels darker than the margin's outline are the black pen's dots. Headless Chromium drew
  // the old near-zero strokes too, so preview.test.ts pins the dots as filled discs.
  const dots = await page.locator(".preview canvas").evaluate((cv: HTMLCanvasElement) => {
    const d = cv.getContext("2d")!.getImageData(0, 0, cv.width, cv.height).data;
    let n = 0;
    for (let i = 0; i < d.length; i += 4) if (d[i] === d[i + 1] && d[i] === d[i + 2] && d[i] < 215) n++;
    return n;
  });
  expect(dots).toBeGreaterThan(1000);
});

test("the style picker groups the styles and picks one", async ({ page }) => {
  test.setTimeout(120_000);
  const id = await sample(page);
  const first = await currentStyle(page);
  await styleButton(page).click();
  const dialog = picker(page);
  await expect(dialog).toBeVisible();
  await expect(dialog.locator(".picker h3")).toHaveText(["Lines", "The Voronoi point family", "Sketch lines", "Other placers", "Tone only"]);
  await expect(card(page, first)).toHaveAttribute("aria-current", "true");
  const choice = dialog.locator(`.picker button[value]:not([value="${first}"])`).first();
  const style = (await choice.getAttribute("value"))!;
  await expect(choice.locator(".description")).toHaveText((await choice.getAttribute("title"))!);
  await expect(choice.locator("img")).toBeVisible({ timeout: 100_000 });
  // Pointing at the card shows its picture over twice as wide, and the click still reaches the card.
  await choice.hover();
  const zoom = dialog.locator(".zoom img");
  await expect(zoom).toBeVisible();
  const [small, large] = [await choice.locator("img").boundingBox(), await zoom.boundingBox()];
  expect(large!.width).toBeGreaterThan(2 * small!.width);
  await choice.click();
  await expect(dialog).toBeHidden();
  await expect(styleButton(page)).toHaveAttribute("value", style);
  await ran(page, id);
});

test("the style picker works from the keyboard", async ({ page }) => {
  const id = await sample(page);
  const first = await currentStyle(page);
  await styleButton(page).focus();
  await page.keyboard.press("Enter");
  await expect(card(page, first)).toBeFocused();
  await page.keyboard.press("Tab");
  const next = await page.evaluate(() => (document.activeElement as HTMLButtonElement).value);
  expect(next).not.toBe(first);
  await page.keyboard.press("Enter");
  await expect(picker(page)).toBeHidden();
  await expect(styleButton(page)).toHaveAttribute("value", next);
  await ran(page, id);
  // Opened again, the picker starts on the new style.
  await styleButton(page).focus();
  await page.keyboard.press("Enter");
  await expect(card(page, next)).toBeFocused();
  await expect(card(page, next)).toHaveAttribute("aria-current", "true");
  await page.keyboard.press("Escape");
  await expect(picker(page)).toBeHidden();
});

test("the style picker works before an image", async ({ page }) => {
  await open(page);
  await styleButton(page).click();
  await expect(picker(page)).toContainText("Open an image to see each style drawn on it");
  await expect(picker(page).locator(".picker img")).toHaveCount(0);
  await card(page, "hatch").click();
  await expect(picker(page)).toBeHidden();
  await expect(styleButton(page)).toHaveAttribute("value", "hatch");
});

test("an image pasted with the style picker open draws its pictures", async ({ page }) => {
  test.setTimeout(120_000);
  await open(page);
  await styleButton(page).click();
  await expect(picker(page).locator(".picker img")).toHaveCount(0);
  await page.evaluate(async () => {
    const canvas = document.createElement("canvas");
    [canvas.width, canvas.height] = [60, 80];
    const g = canvas.getContext("2d")!;
    g.fillRect(10, 10, 40, 60);
    const blob = await new Promise<Blob>((done) => canvas.toBlob((b) => done(b!), "image/png"));
    const data = new DataTransfer();
    data.items.add(new File([blob], "pasted.png", { type: "image/png" }));
    document.dispatchEvent(new ClipboardEvent("paste", { clipboardData: data }));
  });
  await expect(picker(page)).not.toContainText("Open an image to see each style drawn on it");
  await expect(picker(page).locator(".picker img").first()).toBeVisible({ timeout: 100_000 });
});

test("picking the current style again leaves the drawing alone", async ({ page }) => {
  const id = await sample(page);
  await styleButton(page).focus();
  await page.keyboard.press("Enter");
  await expect(card(page, await currentStyle(page))).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(picker(page)).toBeHidden();
  await page.waitForTimeout(500); // time for a run to start by mistake
  await expect(status(page)).toHaveText(`Run ${id}: done`);
});

test("the style picker survives reopening and a run in progress", async ({ page }) => {
  test.setTimeout(180_000);
  const id = await sample(page);
  const styles = await styleNames(page);
  const dialog = picker(page);
  // Opened while a run draws, then closed and reopened before its pictures finish.
  await page.getByRole("button", { name: "Run" }).click();
  for (let i = 0; i < 3; i++) {
    await styleButton(page).click();
    await expect(dialog).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(dialog).toBeHidden();
  }
  await styleButton(page).click();
  // Every style but `learned`, which needs a downloaded model and so shows plain paper.
  await expect(dialog.locator(".picker img")).toHaveCount(styles.length - 1, { timeout: 150_000 });
  await expect(dialog.locator(".picker .paper > span")).toHaveCount(0); // no card left drawing or failed
  await page.keyboard.press("Escape");
  await ran(page, id);
});

// The picker's tabs, and the e2e test model, a single convolution built in code.
const tab = (page: Page, name: string) => picker(page).locator("nav").getByRole("button", { name, exact: true });
const TINY = fileURLToPath(new URL("fixtures/tiny-conv.onnx", import.meta.url));

// Loads the test model through Load your own, which then shows it in the Styles tab.
async function loadOwn(page: Page): Promise<void> {
  await styleButton(page).click();
  await tab(page, "Load your own").click();
  await expect(picker(page)).toContainText("You're responsible for the licence of any model you load.");
  await picker(page).locator('input[type=file][accept=".onnx"]').setInputFiles(TINY);
  await expect(tab(page, "Styles")).toHaveAttribute("aria-current", "true");
}

test("a model of your own draws in the learned style", async ({ page }) => {
  test.setTimeout(120_000);
  let id = await sample(page);
  await loadOwn(page);
  const own = card(page, "learned").filter({ hasText: "your own" });
  await expect(own).toContainText(/file-[0-9a-f]{12}/);
  id = await ran(page, id); // adding a model redraws
  await own.click();
  await expect(picker(page)).toBeHidden();
  await expect(styleButton(page)).toHaveAttribute("value", "learned");
  await ran(page, id);
  await expect(page.locator(".stats")).toContainText(/[1-9][\d,]* paths?/);
  // The settings name the model as a choice of the loaded ones, and Focus has nothing to keep.
  await expect(page.getByRole("combobox", { name: "Model" })).toHaveValue(/^file-[0-9a-f]{12}$/);
  await expect(page.locator("label.chip", { hasText: "Focus" }).locator("input")).toBeDisabled();
});

test("a model of your own stays after a reload, and Remove forgets it", async ({ page }) => {
  await open(page);
  await loadOwn(page);
  const own = card(page, "learned").filter({ hasText: "your own" });
  const name = /file-[0-9a-f]{12}/.exec((await own.textContent())!)![0];
  await page.keyboard.press("Escape");
  await page.reload();
  await styleButton(page).click();
  await expect(own).toContainText(name);
  await tab(page, "Get models").click();
  const row = picker(page).locator("li", { hasText: name });
  await row.getByRole("button", { name: `Remove ${name}`, exact: true }).click();
  await expect(row).toHaveCount(0);
  await tab(page, "Styles").click();
  await expect(card(page, "learned")).toContainText("needs a model");
  await page.keyboard.press("Escape");
  await page.reload();
  await styleButton(page).click();
  await expect(card(page, "learned")).toContainText("needs a model");
});

test("Get models shows the licence before any download", async ({ page }) => {
  // Every request off the page's own server is recorded and answered with a file of the wrong size.
  const requests: string[] = [];
  await page.route((url) => url.hostname !== "localhost", (route) => {
    requests.push(route.request().url());
    return route.fulfill({ status: 200, body: "not a model", headers: { "access-control-allow-origin": "*" } });
  });
  await open(page);
  await styleButton(page).click();
  // Without a model, learned's card says so and opens Get models.
  await card(page, "learned").click();
  await expect(tab(page, "Get models")).toHaveAttribute("aria-current", "true");
  const dialog = picker(page);
  // The listed model's card, and its button, named for the model.
  const teed = dialog.locator(".catalogue li", { hasText: "Edges (TEED)" });
  const download = teed.getByRole("button", { name: "Download Edges (TEED)", exact: true });
  await expect(teed).toContainText("MIT");
  // Cancel fetches nothing and puts the focus back on the tab.
  await download.click();
  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(tab(page, "Get models")).toBeFocused();
  await download.click();
  for (const text of ["Copyright (c) 2022 Xavier Soria Poma", "BIPED", "0.3 MB", "Provided as is, without warranty of any kind."]) await expect(dialog).toContainText(text);
  await expect(dialog.getByRole("link", { name: "https://github.com/xavysp/TEED" })).toBeVisible();
  await expect(dialog.getByRole("link", { name: "https://arxiv.org/abs/2308.06468" })).toBeVisible();
  const accept = dialog.getByRole("button", { name: "Accept" });
  await expect(accept).toBeDisabled();
  await dialog.getByLabel("I've read the licence").check();
  await expect(accept).toBeEnabled();
  expect(requests).toEqual([]);
  await accept.click();
  await expect(tab(page, "Get models")).toBeFocused();
  await expect(teed.locator(".error")).toContainText("wrong size");
  expect(requests).toHaveLength(1);
  // Accepted once, the next download asks no more.
  await download.click();
  await expect.poll(() => requests.length).toBe(2);
  await expect(dialog).not.toContainText("Provided as is");
});

test("Undo and Redo step through style changes", async ({ page }) => {
  let id = await sample(page);
  const first = await currentStyle(page);
  const next = (await styleNames(page)).find((s) => s !== first)!;
  id = await pick(page, next, id);
  await page.getByRole("button", { name: "Undo", exact: true }).click();
  await expect(styleButton(page)).toHaveAttribute("value", first);
  id = await ran(page, id);
  await page.getByRole("button", { name: "Redo", exact: true }).click();
  await expect(styleButton(page)).toHaveAttribute("value", next);
  id = await ran(page, id);
  await page.locator("main").click();
  await page.keyboard.press("Control+z");
  await expect(styleButton(page)).toHaveAttribute("value", first);
  await ran(page, id);
});

// The preview canvas's pixel a little inside the page's corner: in the margin, where the engine
// draws nothing, so it shows the paper. The page sits where fitView puts it, 20 px from the box.
async function paperPixel(page: Page): Promise<number[]> {
  const mm = async (name: RegExp) => Number(await page.getByRole("spinbutton", { name }).inputValue());
  const size = [await mm(/^Width( mm)?$/), await mm(/^Height( mm)?$/), await mm(/^Margin( mm)?$/)];
  return page.locator(".preview canvas").evaluate((c: HTMLCanvasElement, [pw, ph, m]) => {
    const [w, h] = [c.clientWidth, c.clientHeight];
    const scale = Math.min((w - 40) / pw, (h - 40) / ph);
    const k = c.width / w; // the device pixel ratio
    const x = ((w - pw * scale) / 2 + (m / 2) * scale) * k;
    const y = ((h - ph * scale) / 2 + (m / 2) * scale) * k;
    return Array.from(c.getContext("2d")!.getImageData(Math.round(x), Math.round(y), 1, 1).data);
  }, size);
}

test("the paper colour shows in the preview and Undo takes it back", async ({ page }) => {
  let id = await sample(page);
  await expect.poll(() => paperPixel(page)).toEqual([255, 255, 255, 255]);
  await page.getByLabel("Paper", { exact: true }).fill("#000000");
  await expect.poll(() => paperPixel(page)).toEqual([0, 0, 0, 255]);
  // The paper is no input of a run, so the change repaints and starts none.
  await expect(status(page)).toHaveText(new RegExp(`^Run ${id}: done`));
  await page.getByRole("button", { name: "Undo", exact: true }).click();
  id = await ran(page, id);
  await expect(page.getByLabel("Paper", { exact: true })).toHaveValue("#ffffff");
  await expect.poll(() => paperPixel(page)).toEqual([255, 255, 255, 255]);
});

// A hash of the canvas pixels from `from` to `to` of its width (fractions), once the canvas has
// painted. Left out: the outer 8 px at the sides, where the divider sits at 0 and 100, and the
// top 100 px, where its labels sit.
async function canvasHash(page: Page, from: number, to: number): Promise<number> {
  await page.evaluate(() => new Promise((done) => requestAnimationFrame(() => requestAnimationFrame(done))));
  return page.evaluate(([a, b]) => {
    const canvas = document.querySelector<HTMLCanvasElement>(".preview canvas")!;
    const x0 = Math.max(8, Math.floor(canvas.width * a));
    const x1 = Math.min(canvas.width - 8, Math.floor(canvas.width * b));
    const px = canvas.getContext("2d")!.getImageData(x0, 100, x1 - x0, canvas.height - 100).data;
    let hash = 0;
    for (let i = 0; i < px.length; i++) hash = (hash * 31 + px[i]) | 0;
    return hash;
  }, [from, to]);
}

test("Compare swipes the previous drawing against the current one", async ({ page }) => {
  test.setTimeout(120_000);
  // Narrow enough that the page fills the preview's width, so each third shows the drawing.
  await page.setViewportSize({ width: 900, height: 1000 });
  await pick(page, "hatch", await sample(page));
  const [left, right] = [await canvasHash(page, 0, 1 / 3), await canvasHash(page, 2 / 3, 1)];
  await page.locator("label.chip", { hasText: "Compare" }).click();
  await expect(page.getByRole("checkbox", { name: "Compare", exact: true })).toBeChecked();
  // Left of the divider is the first run's drawing now, and right of it the hatch drawing as before.
  await expect.poll(() => canvasHash(page, 0, 1 / 3)).not.toBe(left);
  const previous = await canvasHash(page, 0, 1 / 3);
  expect(await canvasHash(page, 2 / 3, 1)).toBe(right);
  // The divider at 0 leaves nothing to the comparison.
  const swipe = page.getByRole("slider", { name: "Swipe position" });
  await swipe.fill("0");
  await expect.poll(() => canvasHash(page, 0, 1 / 3)).toBe(left);
  await swipe.fill("50");
  await expect.poll(() => canvasHash(page, 0, 1 / 3)).toBe(previous);
  // The divider drags by its handle, and the slider follows.
  const preview = (await page.locator(".preview").boundingBox())!;
  const handle = (await page.locator(".compare-handle").boundingBox())!;
  await page.mouse.move(handle.x + handle.width / 2, preview.y + preview.height / 2);
  await page.mouse.down();
  await page.mouse.move(preview.x + preview.width / 4, preview.y + preview.height / 2, { steps: 5 });
  await page.mouse.up();
  expect(Math.abs(Number(await swipe.inputValue()) - 25)).toBeLessThanOrEqual(1.5);
  await swipe.fill("50");
  // The source image replaces the first run's drawing.
  await page.getByRole("combobox", { name: "Compare with" }).selectOption({ label: "source image" });
  await expect.poll(() => canvasHash(page, 0, 1 / 3)).not.toBe(previous);
  expect(await canvasHash(page, 0, 1 / 3)).not.toBe(left);
  expect(await canvasHash(page, 2 / 3, 1)).toBe(right);
});

// The divider's pixel at the middle of the preview, where the swipe starts, as r, g, b.
const dividerColour = (page: Page) => page.locator(".preview canvas").evaluate((c: HTMLCanvasElement) =>
  Array.from(c.getContext("2d")!.getImageData(Math.round(c.width / 2), Math.round(c.height / 2), 1, 1).data.slice(0, 3)));

test("the compare divider takes the accent of the theme it is in", async ({ page }) => {
  await sample(page);
  await page.locator("label.chip", { hasText: "Compare" }).click();
  const [light, dark] = [[79, 70, 229], [139, 143, 249]]; // --accent in style.css
  await expect.poll(() => dividerColour(page)).toEqual(light);
  // The system's colour scheme, while the theme follows it.
  await page.emulateMedia({ colorScheme: "dark" });
  await expect.poll(() => dividerColour(page)).toEqual(dark);
  await page.emulateMedia({ colorScheme: "light" });
  await expect.poll(() => dividerColour(page)).toEqual(light);
  // The theme button: system, then light, then dark.
  await page.getByRole("button", { name: "System theme" }).click();
  await page.getByRole("button", { name: "Light theme" }).click();
  await expect(page.getByRole("button", { name: "Dark theme" })).toBeVisible();
  await expect.poll(() => dividerColour(page)).toEqual(dark);
});

test("Compare waits for a drawing, and says when there is nothing to compare with", async ({ page }) => {
  await open(page);
  // In the page but hidden, so it does not offer a comparison of nothing.
  const card = page.locator(".compare-pick");
  const chip = page.getByRole("checkbox", { name: "Compare", exact: true });
  await expect(card).toBeAttached();
  await expect(card).toBeHidden();
  // The test chart has no tone map, and nothing came before it.
  await page.getByRole("button", { name: "Test chart" }).click();
  const id = await ran(page, 0);
  await expect(card).toBeVisible();
  await expect(chip).toBeEnabled();
  await page.locator("label.chip", { hasText: "Compare" }).click();
  await expect(page.getByText("Run again to compare with the previous drawing.")).toBeVisible();
  await expect(page.getByRole("slider", { name: "Swipe position" })).toHaveCount(0);
  // The next drawing has a tone map and the chart before it.
  await page.getByRole("button", { name: "Sample image", exact: true }).click();
  await ran(page, id);
  const menu = page.getByRole("combobox", { name: "Compare with" });
  await expect(menu).toHaveValue("previous");
  await expect(menu.getByRole("option", { name: "source image" })).toBeEnabled();
  await expect(page.getByRole("slider", { name: "Swipe position" })).toBeVisible();
});

test("Compare offers only the source image on a first run", async ({ page }) => {
  await sample(page);
  await page.locator("label.chip", { hasText: "Compare" }).click();
  const menu = page.getByRole("combobox", { name: "Compare with" });
  await expect(menu).toHaveValue("source");
  await expect(menu.getByRole("option", { name: "previous run" })).toBeDisabled();
  await expect(menu.getByRole("option", { name: "source image" })).toBeEnabled();
});

test("the toolbar keeps to one row on a wide window and two on a phone, its buttons still named", async ({ page }) => {
  await open(page);
  const toolbar = page.locator(".toolbar");
  for (const [width, rows] of [[1730, 1], [1280, 1], [960, 2], [375, 2]] as const) {
    await page.setViewportSize({ width, height: 800 });
    const { height, overflow } = await toolbar.evaluate((e) => ({ height: e.getBoundingClientRect().height, overflow: e.scrollWidth > e.clientWidth || document.documentElement.scrollWidth > innerWidth }));
    expect(Math.round((height - 8) / 36), `rows at ${width} px`).toBe(rows); // a row is a 30 px button and a 6 px gap
    expect(overflow, `overflow at ${width} px`).toBe(false);
    await expect(page.getByRole("button", { name: "Settings", exact: true })).toBeVisible();
  }
});

test("Tab reaches the toolbar first, and the skip link jumps to the style settings", async ({ page }) => {
  await open(page);
  const skip = page.getByRole("link", { name: "Skip to the style settings" });
  await page.keyboard.press("Tab");
  await expect(skip).toBeFocused();
  await page.keyboard.press("Tab");
  expect(await page.evaluate(() => !!document.activeElement?.closest(".toolbar"))).toBe(true);
  // The columns keep their places on screen: the settings left of the preview and the page panels right of it.
  const [left, middle, right] = await Promise.all(["aside.side-left", "main", "aside.side-right"].map((c) => page.locator(c).boundingBox()));
  expect(left!.x + left!.width).toBeLessThanOrEqual(middle!.x + 1);
  expect(middle!.x + middle!.width).toBeLessThanOrEqual(right!.x + 1);
  // The link lands on the Style card, and Tab goes on to the menu in it.
  await skip.focus();
  await page.keyboard.press("Enter");
  await expect(page.locator("#style-settings")).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(styleButton(page)).toBeFocused();
});

test("on a phone the style menu shows without scrolling", async ({ page }) => {
  await page.setViewportSize({ width: 375, height: 812 });
  await sample(page);
  await expect(fitPanel(page)).not.toHaveAttribute("open", "");
  const menu = (await styleButton(page).boundingBox())!;
  expect(menu.y + menu.height, "bottom of the Style menu").toBeLessThanOrEqual(812);
  expect(await page.evaluate(() => scrollY)).toBe(0);
  // The picker fits the phone: two cards a row and no sideways scroll.
  await styleButton(page).click();
  const cards = picker(page).locator(".picker button[value]");
  const [a, b, c] = await Promise.all([0, 1, 2].map((i) => cards.nth(i).boundingBox()));
  expect(b!.y).toBe(a!.y);
  expect(c!.y).toBeGreaterThan(a!.y);
  expect(await picker(page).evaluate((d) => d.scrollWidth <= d.clientWidth)).toBe(true);
});

test("two pen edits made before the first reply both land", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: "Edit pens" }).click();
  const dialog = page.locator("dialog").filter({ has: page.getByRole("heading", { name: "Pens", exact: true }) });
  // Both changes fire in one task, so the engine cannot have answered the first when the second is made.
  await dialog.evaluate((d) => {
    for (const [label, value] of [["Name", "Quick"], ["Line width", "0.25"]]) {
      const input = [...d.querySelectorAll("label")].find((l) => l.textContent?.trim().startsWith(label))!.querySelector("input")!;
      input.value = value;
      input.dispatchEvent(new Event("change", { bubbles: true }));
    }
  });
  await expect(dialog.locator("ol.pens")).toContainText("Quick");
  await expect(dialog.locator("ol.pens")).toContainText("0.25");
});

test("the test chart draws without an image", async ({ page }) => {
  await open(page);
  // It calibrates the first pen, so it sits with the pens and not with the exports.
  await expect(page.getByRole("group", { name: "Export" }).getByRole("button", { name: "Test chart" })).toHaveCount(0);
  await page.getByRole("group", { name: "Pens" }).getByRole("button", { name: "Test chart" }).click();
  const id = await ran(page, 0);
  const [, paths] = /([\d,]+) paths?/.exec((await page.locator(".stats").textContent()) ?? "") ?? [];
  expect(Number(paths.replaceAll(",", ""))).toBeGreaterThan(0);
  await expect(page.getByRole("button", { name: "Export SVG" })).toBeEnabled();
  // The passes patches are overdraw by design, and the next run replaces the chart.
  await expect(page.locator("ul.lint")).toContainText("drawn over");
  await page.getByRole("button", { name: "Sample image", exact: true }).click();
  await ran(page, id);
});

test("a machine lifted by G-code lines writes them, with long moves cut", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const dialog = page.locator("dialog").filter({ has: page.getByRole("heading", { name: "Settings" }) });
  await dialog.getByRole("combobox", { name: /^Machine/ }).selectOption("new");
  await dialog.getByRole("button", { name: "Pen lift", exact: true }).click();
  await dialog.getByRole("combobox", { name: /^Lift by/ }).selectOption("gcode");
  const down = dialog.getByRole("textbox", { name: "Pen down" });
  await expect(down).toHaveValue("M280 P0 S30");
  await down.fill("M3 S40");
  await down.blur();
  await dialog.getByRole("button", { name: "G-code", exact: true }).click();
  const longest = dialog.getByRole("spinbutton", { name: /^Longest move/ });
  await longest.fill("2");
  await longest.blur();
  await expect(longest).toHaveValue("2"); // a refused value would snap back to empty
  await dialog.getByRole("button", { name: "Close" }).click();
  await page.getByRole("button", { name: "Sample image", exact: true }).click();
  await ran(page, 0);
  const [file] = await Promise.all([page.waitForEvent("download"), page.getByRole("button", { name: "G-code", exact: true }).click()]);
  const lines = readFileSync(await file.path(), "utf8").split("\n").filter((l) => l && !l.startsWith(";"));
  expect(lines).toContain("M3 S40");
  expect(lines).toContain("M280 P0 S90");
  expect(lines.filter((l) => l.includes("Z"))).toEqual([]);
  // Every move after the first travel, whose start the file does not know, is at most 2 mm.
  const points = lines.flatMap((l) => {
    const m = /^G[01] X(\S+) Y(\S+)/.exec(l);
    return m ? [[Number(m[1]), Number(m[2])]] : [];
  });
  expect(points.length).toBeGreaterThan(10);
  const longer = points.slice(1).filter(([x, y], i) => Math.hypot(x - points[i][0], y - points[i][1]) > 2.002);
  expect(longer).toEqual([]);
});

test("HPGL exports one file with a pen for the layer, at the resolution in Settings", async ({ page }) => {
  await sample(page);
  const [file] = await Promise.all([page.waitForEvent("download"), page.getByRole("button", { name: "HPGL", exact: true }).click()]);
  expect(file.suggestedFilename()).toMatch(/^plotline-[\w-]+\.hpgl$/);
  const text = readFileSync(await file.path(), "utf8");
  expect(text.startsWith("IN;\n")).toBe(true);
  expect(text).toContain("SP1;");
  expect(text.endsWith("PU;\nSP0;\n")).toBe(true);
  // A quarter of the units to the mm gives a smaller file, and a value that is no use snaps back.
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const dialog = page.locator("dialog").filter({ has: page.getByRole("heading", { name: "Settings" }) });
  await dialog.getByRole("button", { name: "HPGL", exact: true }).click();
  const units = dialog.getByRole("spinbutton", { name: /^Plotter units per mm/ });
  await expect(units).toHaveValue("40");
  await units.fill("0");
  await units.blur();
  await expect(units).toHaveValue("40");
  await units.fill("10");
  await units.blur();
  await expect(units).toHaveValue("10");
  await dialog.getByRole("button", { name: "Close" }).click();
  const [coarser] = await Promise.all([page.waitForEvent("download"), page.getByRole("button", { name: "HPGL", exact: true }).click()]);
  const small = readFileSync(await coarser.path(), "utf8");
  expect(small.length).toBeLessThan(text.length);
  // Kept in the browser, so a reload keeps it.
  await page.reload();
  await expect(page.getByRole("button", { name: "Sample image", exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await dialog.getByRole("button", { name: "HPGL", exact: true }).click();
  await expect(units).toHaveValue("10");
  // The engine refuses more than 1000, and the page shows its message instead of a download. The
  // reload left no image, so the sample is drawn again first.
  await units.fill("2000");
  await units.blur();
  await dialog.getByRole("button", { name: "Close" }).click();
  await page.getByRole("button", { name: "Sample image", exact: true }).click();
  await ran(page, 0);
  await page.getByRole("button", { name: "HPGL", exact: true }).click();
  await expect(page.getByText("plotter units per mm must be above 0 and at most 1000, not 2000")).toBeVisible();
});

test("a saved pen comes back from My pens after a reload, and exports as a file", async ({ page }) => {
  await open(page);
  const edit = () => page.getByRole("button", { name: "Edit pens" }).click();
  const dialog = page.locator("dialog").filter({ has: page.getByRole("heading", { name: "Pens", exact: true }) });
  const mine = dialog.locator('[data-tour="library"]');
  const name = dialog.getByLabel("Name");
  const width = dialog.getByLabel("Line width");
  await edit();
  // Each change waits for the engine's reply, which the list shows, as one made on the stale list would undo the last.
  const list = dialog.locator("ol.pens");
  await name.fill("Fine red");
  await name.blur();
  await expect(list).toContainText("Fine red");
  await width.fill("0.25");
  await width.blur();
  await expect(list).toContainText("0.25");
  const red = dialog.getByRole("button", { name: "red", exact: true });
  await red.click();
  await expect(red).toHaveAttribute("aria-pressed", "true");
  await mine.getByRole("button", { name: "Save this pen" }).click();
  await expect(mine.getByRole("button", { name: /^Fine red/ })).toHaveCount(1);
  await expect(mine.getByRole("button", { name: "Save this pen" })).toBeDisabled();
  // A second pen takes the saved one's name and width from the list.
  await dialog.getByRole("button", { name: "Add pen" }).click();
  await expect(dialog.getByRole("heading", { name: "Pen 2" })).toBeVisible();
  await expect(name).toHaveValue("Pen 2");
  await mine.getByRole("button", { name: /^Fine red/ }).click();
  await expect(name).toHaveValue("Fine red");
  await expect(width).toHaveValue("0.25");
  // Kept in the browser, so a reload keeps it.
  await page.reload();
  await expect(page.getByRole("button", { name: "Sample image", exact: true })).toBeVisible();
  await edit();
  await expect(mine.getByRole("button", { name: /^Fine red/ })).toHaveCount(1);
  const [file] = await Promise.all([page.waitForEvent("download"), mine.getByRole("button", { name: "Export" }).click()]);
  expect(file.suggestedFilename()).toBe("plotline-pens.json");
  const saved = JSON.parse(readFileSync(await file.path(), "utf8"));
  expect(saved.format).toBe("plotline-pens");
  expect(saved.pens).toHaveLength(1);
  expect(saved.pens[0]).toMatchObject({ name: "Fine red", width_mm: 0.25 });
});

test("pens saved under My pens choose a colour image's palette, and a note names an ink to add", async ({ page }) => {
  const mine = [{ name: "Lemon fine", width_mm: 0.3, color: "#f5c400" }, { name: "Sky fine", width_mm: 0.3, color: "#1d4fb8" }];
  await page.addInitScript((pens) => localStorage.setItem("plotline.pens", JSON.stringify({ format: "plotline-pens", version: 1, pens })), mine);
  await open(page);
  await page.locator(".toolbar input[type=file]").setInputFiles(await coloursPng(page));
  const id = await ran(page, 0);
  await expect(page.getByText(/^Pens you don't have would draw its colours closer: with an? \w+ pen/)).toBeVisible();
  const apply = page.getByRole("button", { name: /^Apply: This is a colour image\. Your pens / });
  await expect(apply).toBeVisible();
  await apply.click();
  await ran(page, id);
  // Only the saved pens, each of them, as the image's yellow and blue need both.
  const names = page.locator(".pen .name");
  await expect.poll(async () => (await names.allTextContents()).sort()).toEqual(mine.map((p) => p.name).sort());
  await expect(page.getByRole("combobox", { name: "Pen use" })).toHaveValue("by_colour");
  await expect(apply).toHaveCount(0);
});

// The steps as tour.tsx lists them, so a step added there is checked here with no change.
const STEPS = [...readFileSync(new URL("../src/tour.tsx", import.meta.url), "utf8").matchAll(/\{ (?:target: "([^"]+)", )?title: "([^"]+)"/g)]
  .map(([, target, title]) => ({ target, title }));

test("every tour step rings its element", async ({ page }) => {
  expect(STEPS.length).toBeGreaterThan(10);
  await open(page, { tour: true });
  const card = page.locator("dialog.tour");
  for (const [i, { target, title }] of STEPS.entries()) {
    await expect(card.locator("h2")).toHaveText(title);
    if (target) {
      // Attached, not visible: a target may be empty until there is something to show, such as
      // the lint list, and the card then sits in the middle of the window.
      const el = page.locator(`[data-tour="${target}"]`);
      await expect(el, title).toBeAttached();
      await expect(el, title).toHaveAttribute("data-tour-active", "");
    } else {
      await expect(page.locator("[data-tour-active]"), title).toHaveCount(0);
    }
    await card.getByRole("button", { name: i === STEPS.length - 1 ? "Done" : "Next" }).click();
  }
  await expect(card).toBeHidden();
});

test("the tour gives focus back when it closes", async ({ page }) => {
  await open(page, { tour: true }); // a first visit opens it with nothing focused
  const card = page.locator("dialog.tour");
  const tour = page.getByRole("button", { name: "Tour", exact: true });
  await expect(card).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(card).toBeHidden();
  await expect(tour).toBeFocused();
  // Opened with another element focused, the X and Done go back to it. dispatchEvent leaves the focus where it was.
  const style = styleButton(page);
  for (const by of ["X", "Done"]) {
    await style.focus();
    await tour.dispatchEvent("click");
    await expect(card).toBeVisible();
    if (by === "X") await card.getByRole("button", { name: "Close the tour" }).click();
    else {
      for (let i = 1; i < STEPS.length; i++) await card.getByRole("button", { name: "Next" }).click();
      await card.getByRole("button", { name: "Done" }).click();
    }
    await expect(card).toBeHidden();
    await expect(style, by).toBeFocused();
  }
  // The focused element gone by the close: the start card's button leaves with the image it opens.
  const start = page.getByRole("button", { name: "Use the sample image" });
  await start.focus();
  await tour.dispatchEvent("click");
  await expect(card).toBeVisible();
  await start.dispatchEvent("click");
  await ran(page, 0);
  await expect(start).toHaveCount(0);
  await page.keyboard.press("Escape");
  await expect(card).toBeHidden();
  await expect(tour).toBeFocused();
});
