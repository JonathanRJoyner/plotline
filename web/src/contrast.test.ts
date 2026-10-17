import { readFileSync } from "node:fs";
import { expect, test } from "vitest";

// WCAG 2 contrast ratio of two #rrggbb colours.
const lum = (hex: string) => {
  const [r, g, b] = [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16) / 255).map((c) => (c <= 0.03928 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};
const ratio = (a: string, b: string) => {
  const [hi, lo] = [lum(a), lum(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
};

const css = readFileSync(new URL("style.css", import.meta.url), "utf8");
// A token's light and dark values, from `--name: light-dark(#light, #dark)`.
const token = (name: string): [string, string] => {
  const m = new RegExp(`--${name}: light-dark\\((#[0-9a-f]{6}), (#[0-9a-f]{6})\\)`).exec(css);
  if (!m) throw new Error(`no light-dark token --${name}`);
  return [m[1], m[2]];
};

test("a form control's border is 3:1 against the card and the field in both themes (WCAG 1.4.11)", () => {
  for (const [theme, i] of [["light", 0], ["dark", 1]] as const) {
    const border = token("control-border")[i];
    for (const behind of ["surface", "field"]) expect(ratio(border, token(behind)[i]), `${theme} border on ${behind}`).toBeGreaterThanOrEqual(3);
  }
});
