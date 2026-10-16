// The image types the engine decodes, for the file picker and for dropped or pasted data.
export const ACCEPT = "image/png,image/jpeg,image/webp";
const TYPES = new Set(ACCEPT.split(","));

// The image file among dropped or pasted data, if any.
export function imageFile(dt: DataTransfer | null): File | undefined {
  return Array.from(dt?.files ?? []).find((f) => TYPES.has(f.type));
}

// Saves text as a file through a link to an object URL.
export function download(name: string, text: string, type: string) {
  const url = URL.createObjectURL(new Blob([text], { type }));
  const a = document.createElement("a");
  a.href = url;
  a.download = name;
  a.click();
  setTimeout(() => URL.revokeObjectURL(url));
}
