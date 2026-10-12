// Pan and zoom for the preview: `scale` is pixels per mm, (`x`, `y`) is the
// page origin in element pixels.

export type View = { scale: number; x: number; y: number };

// Centres the page at the largest scale that keeps it inside the element minus `pad` on every side.
export function fitView(pageW: number, pageH: number, elemW: number, elemH: number, pad = 20): View {
  const scale = Math.max(1e-6, Math.min((elemW - 2 * pad) / pageW, (elemH - 2 * pad) / pageH));
  return { scale, x: (elemW - pageW * scale) / 2, y: (elemH - pageH * scale) / 2 };
}

// Multiplies the scale by `factor`; the page point under (px, py) stays under it.
export function zoomAt(view: View, factor: number, px: number, py: number): View {
  return { scale: view.scale * factor, x: px - (px - view.x) * factor, y: py - (py - view.y) * factor };
}

export function toMm(view: View, px: number, py: number): [number, number] {
  return [(px - view.x) / view.scale, (py - view.y) / view.scale];
}
