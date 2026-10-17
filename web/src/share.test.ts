import fc from "fast-check";
import { expect, test } from "vitest";
import { decodeShare, encodeShare, PREFIX } from "./share";

// A preset's shape, with pen names in several scripts and a letter that takes two UTF-8 bytes.
const preset = {
  format: "plotline-preset", version: 3, style: "stipple",
  canvas: { width_mm: 297, height_mm: 210, margin_mm: 10, fit: "fit" }, paper: "#fffdf5",
  pens: [{ name: "Stift grün ü", width_mm: 0.3, color: "#006600" }, { name: "筆 ペン 😀", width_mm: 0.5, color: "#000000" }],
  machine: null, settings: { stipple: { count: 4000 } },
};

test("a preset reads back from its fragment", async () => {
  const fragment = await encodeShare(JSON.stringify(preset));
  expect(fragment.startsWith(PREFIX)).toBe(true);
  const read = await decodeShare(fragment);
  expect("text" in read && JSON.parse(read.text)).toEqual(preset);
});

test("any well-formed text reads back", async () => {
  await fc.assert(
    fc.asyncProperty(fc.string({ unit: "grapheme" }), async (text) => {
      expect(await decodeShare(await encodeShare(text))).toEqual({ text });
    }),
    { numRuns: 100 },
  );
});

test("the fragment is base64url without padding", async () => {
  // Texts of every length mod 3, from bytes that spread over the whole alphabet.
  for (let n = 1; n <= 60; n++) {
    const text = Array.from({ length: n }, (_, i) => String.fromCharCode(33 + ((i * 7919 + n * 31) % 5000))).join("");
    expect(await encodeShare(text), String(n)).toMatch(/^#p=[A-Za-z0-9_-]+$/);
  }
});

test("a fragment that cannot be read gives an error, not a throw", async () => {
  const good = await encodeShare(JSON.stringify(preset));
  const cases = [
    "#p=", "#p=!!!", "#p=a", "#p=AAAA", "#p=not base64 at all", "#q=" + good.slice(PREFIX.length), "",
    good.slice(0, good.length >> 1), // cut short
    good.slice(0, -4) + "AAAA", // changed at the end
  ];
  for (const c of cases) {
    const read = await decodeShare(c);
    expect("error" in read && read.error.length > 0, c).toBe(true);
  }
});

test("a small link cannot inflate into a huge text", async () => {
  const fragment = await encodeShare("0".repeat(4 << 20));
  expect(fragment.length).toBeLessThan(10_000);
  expect(await decodeShare(fragment)).toHaveProperty("error");
});
