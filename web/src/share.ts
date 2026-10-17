// Share links: a preset in the URL fragment (docs/decisions/2026-10-17-share-link.md). The preset's
// JSON is compressed with the browser's raw deflate (RFC 1951) and written as base64url (RFC 4648
// section 5) without padding. A fragment never goes to a server, and a preset holds no image.

export const PREFIX = "#p=";

// Far above any preset, so that a small link cannot inflate into memory-sized text.
const MAX_BYTES = 1 << 20;

const base64url = (bytes: Uint8Array): string => {
  let binary = "";
  for (const b of bytes) binary += String.fromCharCode(b);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
};

/** The fragment (with its `#p=`) that carries `text`, usually a preset's JSON. */
export async function encodeShare(text: string): Promise<string> {
  const packed = new Blob([text]).stream().pipeThrough(new CompressionStream("deflate-raw"));
  return PREFIX + base64url(new Uint8Array(await new Response(packed).arrayBuffer()));
}

/** The text a fragment from `encodeShare` carries, or the reason it cannot be read; it never throws. */
export async function decodeShare(fragment: string): Promise<{ text: string } | { error: string }> {
  const bad = { error: "The link's setup could not be read: the link looks cut short or changed." };
  const body = fragment.slice(PREFIX.length);
  if (!fragment.startsWith(PREFIX) || !/^[A-Za-z0-9_-]+$/.test(body)) return bad;
  try {
    const binary = atob(body.replace(/-/g, "+").replace(/_/g, "/"));
    const bytes = Uint8Array.from(binary, (c) => c.charCodeAt(0));
    const reader = new Blob([bytes]).stream().pipeThrough(new DecompressionStream("deflate-raw")).getReader();
    const utf8 = new TextDecoder();
    let text = "";
    for (let size = 0, r = await reader.read(); !r.done; r = await reader.read()) {
      size += r.value.length;
      if (size > MAX_BYTES) {
        await reader.cancel();
        return bad;
      }
      text += utf8.decode(r.value, { stream: true });
    }
    return { text: text + utf8.decode() };
  } catch {
    return bad;
  }
}
