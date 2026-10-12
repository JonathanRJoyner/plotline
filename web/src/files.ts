// The image types the engine decodes, for the file picker and for dropped or pasted data.
export const ACCEPT = "image/png,image/jpeg,image/webp";
const TYPES = new Set(ACCEPT.split(","));

// The image file among dropped or pasted data, if any.
export function imageFile(dt: DataTransfer | null): File | undefined {
  return Array.from(dt?.files ?? []).find((f) => TYPES.has(f.type));
}
