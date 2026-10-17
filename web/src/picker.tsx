// The style picker of spec 2026-10-16-style-picker: every style grouped by family, each drawn
// small on the current image with its label and description, in a dialog with room for more tabs.
import { Check, LayoutGrid } from "lucide-preact";
import type { JSX } from "preact";
import { useState } from "preact/hooks";
import type { Canvas } from "./events";
import { Modal } from "./panels";
import { place } from "./tour";
import { onPaper } from "./view";

/** One style the picker lists; a learned model's entry may bring a note and a picture of its own. */
export type Entry = { name: string; label: string; description: string; note?: string; picture?: string };

// The families and their order, word for word as docs/styles.md heads them; a test keeps the two equal.
export const FAMILIES: ReadonlyArray<readonly [label: string, names: readonly string[]]> = [
  ["Lines", ["grid", "hatch", "waves", "spiral", "truchet", "ink", "learned"]],
  ["The Voronoi point family", ["stipple", "line", "dashes", "tree", "mesh", "cells", "scribble"]],
  ["Sketch lines", ["sketch"]],
  ["Other placers", ["halftone", "dither", "circles"]],
  ["Tone only", ["contours", "flow", "hilbert", "mosaic"]],
];
// Where a style in no family goes, so a style the table does not know yet still shows.
export const MORE = "More styles";

/** The entries by family in `FAMILIES` order, then the rest under `MORE` in their own order; no group is empty. */
export function groupStyles<E extends { name: string }>(entries: readonly E[]): Array<[string, E[]]> {
  const byName = new Map(entries.map((e) => [e.name, e]));
  const groups: Array<[string, E[]]> = FAMILIES.map(([label, names]) => [label, names.flatMap((n) => byName.get(n) ?? [])]);
  const known = new Set(FAMILIES.flatMap(([, names]) => names));
  groups.push([MORE, entries.filter((e) => !known.has(e.name))]);
  return groups.filter(([, es]) => es.length > 0);
}

export type Thumb = { url: string | null; error: string | null };
type PickerProps = {
  opens: number; styles: Entry[]; style: string; canvas: Canvas; paper: string; hasImage: boolean;
  thumbs: Record<string, Thumb>; onPick: (style: string) => void; onClose: () => void;
};

// The enlarged picture is this many times the card's; its frame (padding, border and caption, as
// style.css draws them) adds this many pixels around it.
const ZOOM = 2.5;
const FRAME = { w: 14, h: 36 };
// The picker's tabs; a tab is its name here and its panel below. The column of tabs shows once
// there are two.
const TABS = ["Styles"] as const;

// A click on a card switches to its style and closes the picker; closing stops the pictures, which
// arrive one by one. Pointing at a card, or tabbing to it, shows its picture enlarged beside it,
// inside the dialog. The enlargement takes no clicks, and a touch shows none, so a tap still picks.
export function StylePicker({ opens, styles, style, canvas, paper, hasImage, thumbs, onPick, onClose }: PickerProps): JSX.Element {
  const [tab, setTab] = useState<(typeof TABS)[number]>("Styles");
  const [zoom, setZoom] = useState<{ entry: Entry; left: number; top: number; width: number } | null>(null);
  const ratio = canvas.width_mm / canvas.height_mm;
  const show = (entry: Entry, card: HTMLElement) => {
    const paper = card.querySelector(".paper");
    if (!paper) return; // no image, no picture to enlarge
    const p = paper.getBoundingClientRect();
    const box = card.closest("dialog")!.getBoundingClientRect();
    const width = Math.min(ZOOM * p.width, box.width - 16 - FRAME.w, (box.height - 16 - FRAME.h) * ratio);
    const r = { left: p.left - box.left, top: p.top - box.top, right: p.right - box.left, bottom: p.bottom - box.top };
    const { left, top } = place(r, width + FRAME.w, width / ratio + FRAME.h, box.width, box.height);
    setZoom({ entry, left: box.left + left, top: box.top + top, width });
  };
  const hide = () => setZoom(null);
  const pictureOf = (s: Entry) => s.picture ?? thumbs[s.name]?.url;
  const zoomed = zoom && pictureOf(zoom.entry);
  const aspectRatio = `${canvas.width_mm} / ${canvas.height_mm}`;
  const panels = {
    "Styles": (
      <div class="picker" onScroll={hide}>
        {!hasImage && <p class="help">Open an image to see each style drawn on it.</p>}
        {groupStyles(styles).map(([family, entries]) => (
          <section key={family}>
            <h3>{family}</h3>
            <ul class="thumbs">
              {entries.map((s) => {
                const t = thumbs[s.name];
                const url = pictureOf(s);
                const current = s.name === style;
                return (
                  <li key={s.name}>
                    <button
                      type="button" value={s.name} title={s.description} aria-current={current ? "true" : undefined} autofocus={current}
                      onClick={(e) => { onPick(s.name); e.currentTarget.closest("dialog")!.close(); }}
                      onPointerEnter={(e) => e.pointerType !== "touch" && show(s, e.currentTarget)}
                      onPointerLeave={hide}
                      onFocus={(e) => e.currentTarget.matches(":focus-visible") && show(s, e.currentTarget)}
                      onBlur={hide}
                    >
                      {(hasImage || s.picture) && (
                        <span class="paper" style={{ aspectRatio, background: paper, color: onPaper(paper) }}>
                          {url ? <img src={url} alt="" /> : <span>{t?.error ?? "Drawing…"}</span>}
                        </span>
                      )}
                      <span class="name">{current && <Check aria-hidden="true" />}{s.label}{s.note && <span class="note">{s.note}</span>}</span>
                      <span class="description">{s.description}</span>
                    </button>
                  </li>
                );
              })}
            </ul>
          </section>
        ))}
      </div>
    ),
  };
  return (
    <Modal opens={opens} title="Styles" icon={<LayoutGrid />} wide onClose={() => { hide(); onClose(); }}>
      {TABS.length > 1 && (
        <nav>{TABS.map((t) => <button type="button" aria-current={t === tab ? "true" : undefined} onClick={() => setTab(t)}>{t}</button>)}</nav>
      )}
      {panels[tab]}
      {zoom && zoomed && (
        <figure class="zoom" style={{ left: zoom.left, top: zoom.top, width: zoom.width }} aria-hidden="true">
          <img src={zoomed} alt="" style={{ aspectRatio }} />
          <figcaption>{zoom.entry.label}</figcaption>
        </figure>
      )}
    </Modal>
  );
}
