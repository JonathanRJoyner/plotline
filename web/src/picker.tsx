// The style picker of spec 2026-10-16-style-picker: every style grouped by family, each drawn
// small on the current image with its label and description, in a dialog whose other tabs get a
// learned line model or load one of your own (spec 2026-10-16-learned-line-models, "In the GUI").
import { Check, Download, FileUp, LayoutGrid, Trash2 } from "lucide-preact";
import type { JSX } from "preact";
import { useEffect, useRef, useState } from "preact/hooks";
import type { Canvas } from "./events";
import { accept, accepted, type Call, fetchModel, forget, keptIds, loadModel } from "./models";
import { Modal } from "./panels";
import type { CatalogueEntry, ModelInfo } from "./protocol";
import { place } from "./tour";
import { onPaper } from "./view";

/**
 * One style the picker lists. A learned model's entry names its `model` and brings a note, and a
 * picture of its own, or null for plain paper; an entry with a `picture` field is never drawn.
 */
export type Entry = { name: string; label: string; description: string; note?: string; picture?: string | null; model?: string };

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

/**
 * The entries by family in `FAMILIES` order, then the rest under `MORE` in their own order; no group
 * is empty. Entries of one style, such as one per learned model, stay in their own order.
 */
export function groupStyles<E extends { name: string }>(entries: readonly E[]): Array<[string, E[]]> {
  const groups: Array<[string, E[]]> = FAMILIES.map(([label, names]) => [label, names.flatMap((n) => entries.filter((e) => e.name === n))]);
  const known = new Set(FAMILIES.flatMap(([, names]) => names));
  groups.push([MORE, entries.filter((e) => !known.has(e.name))]);
  return groups.filter(([, es]) => es.length > 0);
}

const LEARNED = "learned";

/**
 * The styles with `learned` as one entry per loaded model, in the engine's order, each with the
 * model's card picture when the catalogue lists it; with no model loaded, one entry that needs one.
 */
export function withModels(styles: readonly Entry[], models: readonly ModelInfo[]): Entry[] {
  return styles.flatMap((s) => {
    if (s.name !== LEARNED) return [s];
    if (models.length === 0) return [{ ...s, note: "needs a model", picture: null }];
    return models.map((m) => ({
      name: s.name, model: m.id, label: m.label,
      description: m.entry?.description ?? "Draws the lines a model file of your own finds in the image.",
      note: m.entry?.licence ?? "your own",
      picture: m.entry ? `models/${m.id}.png` : null,
    }));
  });
}

const message = (e: unknown): string => (e instanceof Error ? e.message : String(e));
const mb = (bytes: number) => `${(bytes / 1e6).toFixed(1)} MB`;
// A catalogue text with its web addresses as links: `source` is one, and `paper` a citation then one.
const linked = (text: string) => text.split(/(https:\/\/\S+)/).map((part, i) => (i % 2 ? <a href={part} target="_blank" rel="noreferrer">{part}</a> : part));
const TERMS = [["Licence", "licence"], ["Attribution", "attribution"], ["Training", "training"], ["Source", "source"], ["Paper", "paper"]] as const;

// The licence screen a first download goes through; nothing is fetched until Accept, which waits
// for the box to be ticked.
function Licence({ entry, onAccept, onCancel }: { entry: CatalogueEntry; onAccept: () => void; onCancel: () => void }): JSX.Element {
  const [read, setRead] = useState(false);
  const box = useRef<HTMLInputElement>(null);
  useEffect(() => box.current?.focus(), []); // the Download button that opened it has gone
  return (
    <section class="licence">
      <h3>{entry.label}</h3>
      <dl>
        {TERMS.map(([label, key]) => <><dt>{label}</dt><dd>{linked(entry[key])}</dd></>)}
        <dt>Size</dt><dd>{mb(entry.bytes)}</dd>
      </dl>
      <p>Provided as is, without warranty of any kind.</p>
      <label><input ref={box} type="checkbox" checked={read} onChange={(e) => setRead(e.currentTarget.checked)} /> I've read the licence</label>
      <div class="actions">
        <button type="button" class="primary" disabled={!read} onClick={onAccept}>Accept</button>
        <button type="button" onClick={onCancel}>Cancel</button>
      </div>
    </section>
  );
}

export type Thumb = { url: string | null; error: string | null };
type PickerProps = {
  opens: number; styles: Entry[]; style: string; canvas: Canvas; paper: string; hasImage: boolean;
  thumbs: Record<string, Thumb>; onPick: (entry: Entry) => void; onClose: () => void;
  /** The current `model.model`, the loaded models, the catalogue, and the engine call that adds and removes models. */
  model: string; models: ModelInfo[]; catalogue: CatalogueEntry[]; send: Call;
};

// The enlarged picture is this many times the card's; its frame (padding, border and caption, as
// style.css draws them) adds this many pixels around it.
const ZOOM = 2.5;
const FRAME = { w: 14, h: 36 };
// The picker's tabs, by id and label; a tab is its entry here and its panel below.
const TABS = [["styles", "Styles"], ["get-models", "Get models"], ["load-own", "Load your own"]] as const;
type Tab = (typeof TABS)[number][0];

// A click on a card switches to its style and closes the picker; closing stops the pictures, which
// arrive one by one. Pointing at a card, or tabbing to it, shows its picture enlarged beside it,
// inside the dialog. The enlargement takes no clicks, and a touch shows none, so a tap still picks.
// Closing goes back to the Styles tab, so the next opening starts on the current style's card.
export function StylePicker({ opens, styles, style, canvas, paper, hasImage, thumbs, onPick, onClose, model, models, catalogue, send }: PickerProps): JSX.Element {
  const [tab, setTab] = useState<Tab>("styles");
  const [zoom, setZoom] = useState<{ entry: Entry; left: number; top: number; width: number } | null>(null);
  const [licence, setLicence] = useState<CatalogueEntry | null>(null);
  const [progress, setProgress] = useState<Record<string, number>>({}); // downloads under way, by id
  const [failed, setFailed] = useState<Record<string, string>>({}); // by model id; "" for Load your own
  const [kept, setKept] = useState<string[]>([]);
  const nav = useRef<HTMLElement>(null);
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
  // Another tab, with the focus on its button, since the control that led there may be gone.
  const openTab = (t: Tab) => {
    hide();
    setTab(t);
    nav.current?.querySelector<HTMLElement>(`[value="${t}"]`)?.focus();
  };
  const refresh = () => void keptIds().then(setKept);
  useEffect(refresh, [opens]);
  const report = (id: string, e?: unknown) => setFailed((f) => ({ ...f, [id]: e === undefined ? "" : message(e) }));

  // A download keeps the file only once the engine has taken it; see `fetchModel`.
  const get = async (entry: CatalogueEntry) => {
    report(entry.id);
    setProgress((p) => ({ ...p, [entry.id]: 0 }));
    try {
      await fetchModel(entry, send, (got, total) => setProgress((p) => ({ ...p, [entry.id]: got / total })));
    } catch (e) {
      report(entry.id, e);
    }
    setProgress(({ [entry.id]: _, ...p }) => p);
    refresh();
  };
  const remove = async (id: string) => {
    report(id);
    try {
      if (models.some((m) => m.id === id)) await send("removeModel", [id]);
      await forget(id);
    } catch (e) {
      report(id, e);
    }
    refresh();
  };
  const loadOwn = async (file: File) => {
    report("");
    try {
      await loadModel(await file.arrayBuffer(), send);
      refresh();
      openTab("styles"); // where the model now shows
    } catch (e) {
      report("", e);
    }
  };

  const pictureOf = (s: Entry) => (s.picture === undefined ? thumbs[s.name]?.url : s.picture);
  const zoomed = zoom && pictureOf(zoom.entry);
  const aspectRatio = `${canvas.width_mm} / ${canvas.height_mm}`;
  const loaded = new Set(models.map((m) => m.id));
  const listed = new Set(catalogue.map((e) => e.id));
  // Models of your own, and kept files the engine did not take, which Remove still forgets.
  const others = [...new Set([...models.filter((m) => !m.entry).map((m) => m.id), ...kept.filter((id) => !listed.has(id))])];
  const removeButton = (id: string) => <button type="button" onClick={() => remove(id)}><Trash2 /> Remove</button>;
  const panels: Record<Tab, JSX.Element> = {
    "styles": (
      <div class="picker" onScroll={hide}>
        {!hasImage && <p class="help">Open an image to see each style drawn on it.</p>}
        {groupStyles(withModels(styles, models)).map(([family, entries]) => (
          <section key={family}>
            <h3>{family}</h3>
            <ul class="thumbs">
              {entries.map((s) => {
                const t = thumbs[s.name];
                const url = pictureOf(s);
                const current = s.name === style && (s.model === undefined || s.model === model);
                return (
                  <li key={s.model ?? s.name}>
                    <button
                      type="button" value={s.name} title={s.description} aria-current={current ? "true" : undefined} autofocus={current}
                      onClick={(e) => {
                        if (s.name === LEARNED && s.model === undefined) return openTab("get-models");
                        onPick(s);
                        e.currentTarget.closest("dialog")!.close();
                      }}
                      onPointerEnter={(e) => e.pointerType !== "touch" && show(s, e.currentTarget)}
                      onPointerLeave={hide}
                      onFocus={(e) => e.currentTarget.matches(":focus-visible") && show(s, e.currentTarget)}
                      onBlur={hide}
                    >
                      {(hasImage || s.picture) && (
                        <span class="paper" style={{ aspectRatio, background: paper, color: onPaper(paper) }}>
                          {url ? <img src={url} alt="" /> : s.picture === undefined && <span>{t?.error ?? "Drawing…"}</span>}
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
    "get-models": (
      <div class="fields">
        {licence ? (
          <Licence
            key={licence.id} entry={licence} onCancel={() => setLicence(null)}
            onAccept={() => { setLicence(null); void accept(licence); void get(licence); }}
          />
        ) : (
          <>
            <ul class="catalogue">
              {catalogue.map((e) => (
                <li key={e.id}>
                  <img src={`models/${e.id}.png`} alt="" />
                  <span class="name">{e.label}<span class="note">{e.licence}</span></span>
                  <span class="description">{e.description}</span>
                  <span class="help">{mb(e.bytes)}</span>
                  {loaded.has(e.id) || kept.includes(e.id) ? removeButton(e.id) : (
                    <button type="button" disabled={e.id in progress} onClick={async () => ((await accepted(e)) ? get(e) : setLicence(e))}>
                      <Download /> {e.id in progress ? `Downloading ${Math.round(progress[e.id] * 100)}%` : "Download"}
                    </button>
                  )}
                  {kept.includes(e.id) && !loaded.has(e.id) && <p class="help">Kept in this browser, but it did not load.</p>}
                  {failed[e.id] && <p class="error">{failed[e.id]}</p>}
                </li>
              ))}
            </ul>
            {others.length > 0 && (
              <>
                <h3>Your models</h3>
                <ul class="kept">
                  {others.map((id) => (
                    <li key={id}>
                      {models.find((m) => m.id === id)?.label ?? id}
                      {!loaded.has(id) && <span class="note">not loaded</span>}
                      {removeButton(id)}
                      {failed[id] && <p class="error">{failed[id]}</p>}
                    </li>
                  ))}
                </ul>
              </>
            )}
          </>
        )}
      </div>
    ),
    "load-own": (
      <div class="fields">
        <label class="button">
          <FileUp /> Choose a model file
          <input
            class="visually-hidden" type="file" accept=".onnx"
            onChange={(e) => {
              const file = e.currentTarget.files?.[0];
              e.currentTarget.value = ""; // so the same file can be chosen again
              if (file) void loadOwn(file);
            }}
          />
        </label>
        <p class="help">You're responsible for the licence of any model you load.</p>
        {failed[""] && <p class="error">{failed[""]}</p>}
      </div>
    ),
  };
  return (
    <Modal opens={opens} title="Styles" icon={<LayoutGrid />} wide onClose={() => { hide(); setTab("styles"); setLicence(null); onClose(); }}>
      <nav ref={nav}>
        {TABS.map(([id, label]) => <button type="button" key={id} value={id} aria-current={id === tab ? "true" : undefined} onClick={() => openTab(id)}>{label}</button>)}
      </nav>
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
