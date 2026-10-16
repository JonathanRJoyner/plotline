// One flow per user-visible GUI feature, in a real browser against the built bundle. A new
// feature adds its flow here, as it adds its tour step. Every test also fails on a console error.
import { readFileSync } from "node:fs";
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
  await expect(page.getByRole("button", { name: "Sample image" })).toBeVisible();
}

const status = (page: Page) => page.locator(".status > span").first();
const styleSelect = (page: Page) => page.getByRole("combobox", { name: "Style", exact: true });

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
  await page.getByRole("button", { name: "Sample image" }).click();
  return ran(page, 0);
}

async function pick(page: Page, style: string, after: number): Promise<number> {
  await styleSelect(page).selectOption(style);
  return ran(page, after);
}

const styleNames = (page: Page) => styleSelect(page).locator("option").evaluateAll((os) => os.map((o) => (o as HTMLOptionElement).value));

test("the sample image draws in every style", async ({ page }) => {
  test.setTimeout(180_000);
  let id = await sample(page);
  const styles = await styleNames(page);
  expect(styles.length).toBeGreaterThan(10);
  const first = await styleSelect(page).inputValue();
  for (const style of styles.filter((s) => s !== first)) {
    id = await pick(page, style, id);
    await expect(page.locator(".stats"), style).toContainText("paths");
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
  await expect(styleSelect(page)).toHaveValue("stipple");
  await expect(count).toHaveValue(changed);
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

test("Fit to target meets its target", async ({ page }) => {
  test.setTimeout(120_000);
  await pick(page, "hatch", await sample(page));
  const [, min, s] = /about (\d+) min (\d+) s/.exec((await page.locator(".stats").textContent()) ?? "") ?? [];
  const target = Math.round(((Number(min) * 60 + Number(s)) / 60) * 0.75 * 10) / 10;
  await page.getByRole("spinbutton", { name: "Target", exact: true }).fill(String(target));
  await page.getByRole("combobox", { name: "Target unit" }).selectOption("minutes");
  await page.getByRole("button", { name: "Fit to target" }).click();
  const fitted = page.locator("form.fit span");
  await expect(fitted).toContainText("for a target of", { timeout: 100_000 });
  const [, achieved, aimed] = /: ([\d.e+-]+) min for a target of ([\d.]+) min/.exec((await fitted.textContent()) ?? "") ?? [];
  expect(Number(aimed)).toBe(target);
  // The solver stops within 2 % of the target (TOLERANCE in crates/core/src/fit.rs).
  expect(Math.abs(Number(achieved) / target - 1)).toBeLessThanOrEqual(0.02);
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

test("a colour image's palette suggestion sets the pens and Pen use", async ({ page }) => {
  await open(page);
  // Four saturated stripes, drawn in the page and encoded as a PNG.
  const png = await page.evaluate(async () => {
    const canvas = document.createElement("canvas");
    canvas.width = 240;
    canvas.height = 320;
    const g = canvas.getContext("2d")!;
    ["#e00000", "#00b000", "#0000e0", "#f0e000"].forEach((c, i) => { g.fillStyle = c; g.fillRect(i * 60, 0, 60, 320); });
    const blob = await new Promise<Blob>((done) => canvas.toBlob((b) => done(b!), "image/png"));
    return Array.from(new Uint8Array(await blob.arrayBuffer()));
  });
  await page.locator('input[type="file"][accept^="image/"]').setInputFiles({ name: "colours.png", mimeType: "image/png", buffer: Buffer.from(png) });
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

test("the stats show the tone match", async ({ page }) => {
  await sample(page);
  await expect(page.locator(".stats")).toContainText(/tone error \d\.\d{3} · SSIM -?\d\.\d{2}/);
});

test("Optimize suggests a spacing and changes nothing until Apply", async ({ page }) => {
  test.setTimeout(120_000);
  const id = await pick(page, "hatch", await sample(page));
  const spacing = page.getByRole("spinbutton", { name: /^spacing( mm)?$/i });
  const before = await spacing.inputValue();
  await page.getByRole("button", { name: "Optimize" }).click();
  // The search is a run of its own, which ends with the suggestion.
  const searched = await ran(page, id);
  const found = page.locator("form.fit span");
  await expect(found).toContainText("follows the image's tones best");
  await expect(spacing).toHaveValue(before);
  // The still life's best hatch spacing is wider than the default (brief §14, "Optimize").
  const [, value] = /^Spacing at ([\d.]+) mm follows/.exec((await found.textContent()) ?? "") ?? [];
  expect(Number(value)).toBeGreaterThan(Number(before));
  await page.getByRole("button", { name: /^Apply: Spacing at/ }).click();
  await ran(page, searched);
  await expect(spacing).toHaveValue(value);
});

test("Response auto corrects the tone map for the style", async ({ page }) => {
  const id = await pick(page, "hatch", await sample(page));
  const stats = await page.locator(".stats").textContent();
  await page.locator("summary", { hasText: /^Image$/ }).click();
  await page.getByRole("combobox", { name: /^response$/i }).selectOption("auto");
  await ran(page, id);
  await expect(page.locator(".stats")).not.toHaveText(stats ?? "");
});

test("the thumbnail grid picks a style", async ({ page }) => {
  test.setTimeout(120_000);
  const id = await sample(page);
  const styles = await styleNames(page);
  const first = await styleSelect(page).inputValue();
  const i = styles.findIndex((s) => s !== first);
  await page.getByRole("button", { name: "Try all styles" }).click();
  const dialog = page.locator("dialog").filter({ has: page.getByRole("heading", { name: "All styles" }) });
  await expect(dialog).toBeVisible();
  const choice = dialog.locator("ul.thumbs li").nth(i).getByRole("button");
  await expect(choice.locator("img")).toBeVisible({ timeout: 100_000 });
  // Pointing at the cell shows its picture over twice as wide, and the click still reaches the cell.
  await choice.hover();
  const zoom = dialog.locator(".zoom img");
  await expect(zoom).toBeVisible();
  const [small, large] = [await choice.locator("img").boundingBox(), await zoom.boundingBox()];
  expect(large!.width).toBeGreaterThan(2 * small!.width);
  await choice.click();
  await expect(dialog).toBeHidden();
  await expect(styleSelect(page)).toHaveValue(styles[i]);
  await ran(page, id);
});

test("the thumbnail grid survives reopening and a run in progress", async ({ page }) => {
  test.setTimeout(180_000);
  const id = await sample(page);
  const styles = await styleNames(page);
  const tryAll = page.getByRole("button", { name: "Try all styles" });
  const dialog = page.locator("dialog").filter({ has: page.getByRole("heading", { name: "All styles" }) });
  // Opened while a run draws, then closed and reopened before its pictures finish.
  await page.getByRole("button", { name: "Run" }).click();
  for (let i = 0; i < 3; i++) {
    await tryAll.click();
    await expect(dialog).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(dialog).toBeHidden();
  }
  await tryAll.click();
  await expect(dialog.locator("ul.thumbs img")).toHaveCount(styles.length, { timeout: 150_000 });
  await expect(dialog.locator("ul.thumbs .paper > span")).toHaveCount(0); // no cell left drawing or failed
  await page.keyboard.press("Escape");
  await ran(page, id);
});

test("Undo and Redo step through style changes", async ({ page }) => {
  let id = await sample(page);
  const first = await styleSelect(page).inputValue();
  const next = (await styleNames(page)).find((s) => s !== first)!;
  id = await pick(page, next, id);
  await page.getByRole("button", { name: "Undo", exact: true }).click();
  await expect(styleSelect(page)).toHaveValue(first);
  id = await ran(page, id);
  await page.getByRole("button", { name: "Redo", exact: true }).click();
  await expect(styleSelect(page)).toHaveValue(next);
  id = await ran(page, id);
  await page.locator("main").click();
  await page.keyboard.press("Control+z");
  await expect(styleSelect(page)).toHaveValue(first);
  await ran(page, id);
});

test("the toolbar keeps to one row on a wide window and two on a phone, its buttons still named", async ({ page }) => {
  await open(page);
  const toolbar = page.locator(".toolbar");
  for (const [width, rows] of [[1730, 1], [1280, 1], [960, 2], [375, 2]] as const) {
    await page.setViewportSize({ width, height: 800 });
    const { height, overflow } = await toolbar.evaluate((e) => ({ height: e.getBoundingClientRect().height, overflow: e.scrollWidth > e.clientWidth }));
    expect(Math.round((height - 8) / 36), `rows at ${width} px`).toBe(rows); // a row is a 30 px button and a 6 px gap
    expect(overflow, `overflow at ${width} px`).toBe(false);
    await expect(page.getByRole("button", { name: "Settings", exact: true })).toBeVisible();
  }
});

test("the test chart draws without an image", async ({ page }) => {
  await open(page);
  await page.getByRole("button", { name: "Test chart" }).click();
  const id = await ran(page, 0);
  const [, paths] = /(\d+) paths/.exec((await page.locator(".stats").textContent()) ?? "") ?? [];
  expect(Number(paths)).toBeGreaterThan(0);
  await expect(page.getByRole("button", { name: "Export SVG" })).toBeEnabled();
  // The passes patches are overdraw by design, and the next run replaces the chart.
  await expect(page.locator("ul.lint")).toContainText("drawn over");
  await page.getByRole("button", { name: "Sample image" }).click();
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
  await page.getByRole("button", { name: "Sample image" }).click();
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
  await expect(page.getByRole("button", { name: "Sample image" })).toBeVisible();
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await dialog.getByRole("button", { name: "HPGL", exact: true }).click();
  await expect(units).toHaveValue("10");
  // The engine refuses more than 1000, and the page shows its message instead of a download. The
  // reload left no image, so the sample is drawn again first.
  await units.fill("2000");
  await units.blur();
  await dialog.getByRole("button", { name: "Close" }).click();
  await page.getByRole("button", { name: "Sample image" }).click();
  await ran(page, 0);
  await page.getByRole("button", { name: "HPGL", exact: true }).click();
  await expect(page.getByText("plotter units per mm must be above 0 and at most 1000, not 2000")).toBeVisible();
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
