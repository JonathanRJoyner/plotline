// The compare view: the current drawing swiped against the previous run's drawing or the source
// image, on the same page at the same zoom. The preview paints the swipe; this file holds the
// rules for what can be compared and the controls over the preview.
import { Columns2 } from "lucide-preact";
import type { JSX } from "preact";
import type { ResultEvent } from "./events";

export type Against = "previous" | "source";
/** `at` is the divider's place in percent of the preview's width: the comparison shows left of it. */
export type Compare = { against: Against; at: number };

/** The divider's x in pixels on a canvas `width` pixels wide. */
export const dividerX = (at: number, width: number): number => (Math.min(100, Math.max(0, at)) / 100) * width;

/** The divider's place in percent for a pointer `x` pixels from the left edge of a box `width` wide, in steps of 0.5 like the slider. */
export const atFromX = (x: number, width: number): number => (width > 0 ? Math.min(100, Math.max(0, Math.round((x / width) * 200) / 2)) : 50);

/** What there is to compare with: the previous drawing once a run has replaced one, the source once the result has a tone map. */
export const offers = (previous: ResultEvent | null, result: ResultEvent | null): Record<Against, boolean> => ({
  previous: previous !== null,
  source: result?.tone != null,
});

/** The wanted comparison, or the other one when only that is on offer; null when neither is. */
export function choice(want: Against, offered: Record<Against, boolean>): Against | null {
  if (offered[want]) return want;
  const other = want === "previous" ? "source" : "previous";
  return offered[other] ? other : null;
}

type Props = {
  on: boolean;
  /** False while there is no drawing at all. */
  enabled: boolean;
  /** What is compared, or null when there is nothing to compare with yet. */
  against: Against | null;
  offered: Record<Against, boolean>;
  at: number;
  onOn: (on: boolean) => void;
  onAgainst: (against: Against) => void;
  onAt: (at: number) => void;
};

// Over the preview, which positions it: the card at the top left with the Compare chip and, once it
// is on, the menu; the slider along the bottom; and a handle on the divider, which drags like the
// slider. The handle comes first, so the others sit over it.
export function CompareControls({ on, enabled, against, offered, at, onOn, onAgainst, onAt }: Props): JSX.Element {
  const swipe = on && against !== null;
  return (
    <>
      {swipe && (
        <div
          class="compare-handle" aria-hidden="true" style={{ left: `${at}%` }}
          onPointerDown={(e) => e.currentTarget.setPointerCapture(e.pointerId)}
          onPointerMove={(e) => {
            if (!e.currentTarget.hasPointerCapture(e.pointerId)) return;
            const box = e.currentTarget.parentElement!; // the preview; its border is outside the canvas
            onAt(atFromX(e.clientX - box.getBoundingClientRect().left - box.clientLeft, box.clientWidth));
          }}
        />
      )}
      <div class="compare-pick">
        <label class="chip" title="Compare" data-tour="compare">
          <input class="visually-hidden" type="checkbox" checked={on} disabled={!enabled} onChange={(e) => onOn(e.currentTarget.checked)} />
          <Columns2 /> <span class="label">Compare</span>
        </label>
        {on && against === null && <span>Run again to compare with the previous drawing.</span>}
        {swipe && (
          <select aria-label="Compare with" value={against} onChange={(e) => onAgainst(e.currentTarget.value as Against)}>
            <option value="previous" disabled={!offered.previous}>previous run</option>
            <option value="source" disabled={!offered.source}>source image</option>
          </select>
        )}
      </div>
      {swipe && (
        <div class="compare-swipe">
          <input type="range" aria-label="Swipe position" min="0" max="100" step="0.5" value={at} onInput={(e) => onAt(e.currentTarget.valueAsNumber)} />
        </div>
      )}
    </>
  );
}
