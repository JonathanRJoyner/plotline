import { expect, test } from "vitest";
import { imageFile } from "./files";

const png = new File([new Uint8Array([1])], "a.png", { type: "image/png" });
const jpeg = new File([new Uint8Array([1])], "a.jpg", { type: "image/jpeg" });
const txt = new File(["x"], "a.txt", { type: "text/plain" });
const dt = (...files: File[]) => ({ files } as unknown as DataTransfer);

test("imageFile takes the first PNG or JPEG", () => {
  expect(imageFile(dt(txt, png))).toBe(png);
  expect(imageFile(dt(jpeg))).toBe(jpeg);
});

test("imageFile ignores other files and nothing", () => {
  expect(imageFile(dt(txt))).toBeUndefined();
  expect(imageFile(dt())).toBeUndefined();
  expect(imageFile(null)).toBeUndefined();
});
