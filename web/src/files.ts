// The image file among dropped or pasted data, if any.
const TYPES = new Set(["image/png", "image/jpeg"]);

export function imageFile(dt: DataTransfer | null): File | undefined {
  return Array.from(dt?.files ?? []).find((f) => TYPES.has(f.type));
}
