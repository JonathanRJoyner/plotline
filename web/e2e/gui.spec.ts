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
  await choice.click();
  await expect(dialog).toBeHidden();
  await expect(styleSelect(page)).toHaveValue(styles[i]);
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
