// The guided tour: a card that walks through the page one feature at a time, ringing the element
// it talks about. A step names its element by the element's data-tour attribute; a step without
// one, or whose element is not on screen, shows in the middle of the window. A new feature adds
// its step to STEPS, and the test checks that every target is on an element.
import { X } from "lucide-preact";
import type { JSX } from "preact";
import { useEffect, useLayoutEffect, useRef, useState } from "preact/hooks";

export type Step = { target?: string; title: string; text: string };

export const STEPS: Step[] = [
  { title: "Welcome to plotline", text: "plotline turns an image into lines a pen plotter can draw. This tour follows the work in order: open an image, shape the drawing, check it, set the page and pens, and export. Tour in the toolbar opens it again." },
  { target: "image", title: "Open an image", text: "Open a PNG, JPEG or WebP, or drop or paste one anywhere on the page. Sample image opens a still life to try things on. Once an image is open, Suggestions at the top of the right-hand column list what would suit it, such as turning the page or a set of ink pens for a colour image; nothing changes until you click Apply, and the cross hides one you don't want." },
  { target: "style", title: "Pick a style and a look", text: "Each style turns the image's tone into a different kind of mark, and the line under the menu says what it does. A look sets a few of the style's settings for an aim: Balanced is the defaults, Faithful follows the image's tones most closely, Quick plots in about half an hour and Fine draws the densest marks that still plot cleanly. A switch of style keeps the settings you changed wherever the new style has them, and Reset to defaults puts the style back as it came." },
  { target: "styles", title: "Try every style", text: "Try all styles draws every style small on your image, page and pens, so you can choose by eye. Point at a picture, or tab to it, to see it larger, and click it to switch to that style. Closing the window leaves your own drawing as it was." },
  { target: "settings", title: "Shape the drawing", text: "The style's own settings come first. Path order and Image are collapsed below them; Image holds the levels, gamma, blur, Response and Flatten paper, which evens out the grey or yellowed paper of a scan. Focus in the toolbar marks points whose detail every style keeps: click the drawn image, or press Enter on the focused preview to mark its centre. A change redraws on its own and reruns only the stages after it." },
  { target: "preview", title: "Look closely", text: "The preview draws every mark at the pen's real width and colour, on your paper. Drag to pan and scroll to zoom; with the preview focused, the arrow keys pan, plus and minus zoom and 0 fits the page. Source shows the image under the drawing, Travel the pen-up moves, and Compare, at the preview's top left, swipes the drawing against the previous run or the image. In a narrow window the toolbar shows icons alone: hover one for its name." },
  { target: "status", title: "Check the plot", text: "Each stage shows its progress here; Run draws again and Stop ends a run after its current step. Then come the path count, the pen-down and pen-up lengths, the plot time and how far the tones are from the image's. After each run plotline checks the drawing for trouble on paper, such as marks drawn over each other, specks or many pen lifts, and Apply changes the setting that fixes one." },
  { target: "fit", title: "Meet a budget or match the image", text: "Fit and optimize, under the stats, holds two searches. Fit to target reruns the style with its density setting changed until the plot time, path count, metres of line or dot count is near the number you type; the time leaves out acceleration, so dots and short strokes plot slower than it says. Optimize suggests the density that brings the tones closest to the image, within the target if you typed one, and changes nothing until you click Apply." },
  { target: "undo", title: "Undo a change", text: "Undo and Redo step through changes to the style, its settings, the page, the paper and the pens, so trying something costs nothing. Ctrl+Z and Ctrl+Shift+Z do the same, except while you type in a field." },
  { target: "page", title: "Set the page", text: "The paper size, orientation and margin. Fit keeps the whole image on the page; fill covers the drawing area and crops the rest. The paper's colour is what the preview and the style pictures draw the ink onto, so cream or black paper looks as it will plot." },
  { target: "pens", title: "Choose pens", text: "Edit pens opens the pens dialog: pick a maker's pen from the bank at its published line width, or set a width yourself, and choose an ink colour. Save this pen keeps a pen in My pens, in this browser, to pick again later or export as a file. The tick hides a pen in the preview. Pen use shares the drawing between the pens: light to dark in list order, or each pen by its colour; pens plot in list order, so list light inks first. Test chart draws a page of labelled patches for the first pen, to find on paper where spaced lines merge and how big dots come out." },
  { target: "export", title: "Export and keep a setup", text: "SVG writes the drawing in millimetres, one layer per pen, for your plotter's own software; JSON writes the paths for other programs, and HPGL one file for plotters that read it. A preset, under Export, keeps the style, page, pens and settings as a small file; an exported SVG or JSON holds one too, and the page remembers your last setup." },
  { target: "machine", title: "Plot from G-code", text: "Settings holds your machine: its drawable area, its feeds, and how it lifts the pen, by Z moves or by its own lines for a servo or a solenoid. Once one is set, Export writes a G-code file per pen." },
  { target: "help", title: "Help on every setting", text: "Hover over a setting's name to read its help, or turn on Show help to see it under every control." },
];

type Rect = { left: number; top: number; right: number; bottom: number };

// Where the card goes: below the target, above it, to its right or to its left, the first with
// room for the card, kept inside the window; centred when there is no target or no room.
export function place(r: Rect | null, w: number, h: number, width: number, height: number): { left: number; top: number } {
  const gap = 12;
  const x = (left: number) => Math.max(8, Math.min(left, width - w - 8));
  const y = (top: number) => Math.max(8, Math.min(top, height - h - 8));
  if (r) {
    if (r.bottom + gap + h <= height) return { left: x(r.left), top: r.bottom + gap };
    if (r.top - gap - h >= 0) return { left: x(r.left), top: r.top - gap - h };
    if (r.right + gap + w <= width) return { left: r.right + gap, top: y(r.top) };
    if (r.left - gap - w >= 0) return { left: r.left - gap - w, top: y(r.top) };
  }
  return { left: (width - w) / 2, top: (height - h) / 2 };
}

// Whether the tour has never opened in this browser; the first call marks it opened. Without
// storage the tour waits for its button, rather than opening on every visit.
const TOUR = "plotline.tour";
export function firstVisit(): boolean {
  try {
    if (localStorage.getItem(TOUR)) return false;
    localStorage.setItem(TOUR, "seen");
    return true;
  } catch {
    return false;
  }
}

// Not modal, so the page stays usable while the card points at it. Each change of `opens` starts
// it from the first step. Closing gives focus back to the element that had it on opening, or to
// the toolbar's Tour button when that is gone, or nothing had it.
export function Tour({ opens }: { opens: number }): JSX.Element {
  const ref = useRef<HTMLDialogElement>(null);
  const [open, setOpen] = useState(false);
  const [i, setI] = useState(0);
  const from = useRef<Element | null>(null);
  const step = STEPS[i];
  const close = () => {
    setOpen(false);
    (from.current?.isConnected ? (from.current as HTMLElement) : document.getElementById("tour-button"))?.focus();
  };
  useEffect(() => {
    if (opens > 0) {
      // Kept from the first opening only: the Tour button, pressed while the tour is open, is no place to go back to.
      if (!open) from.current = document.activeElement === document.body ? null : document.activeElement;
      setI(0);
      setOpen(true);
    }
  }, [opens]);
  useLayoutEffect(() => {
    if (!open) return;
    const card = ref.current!;
    const el = step.target ? document.querySelector<HTMLElement>(`[data-tour="${step.target}"]`) : null;
    el?.setAttribute("data-tour-active", "");
    el?.scrollIntoView({ block: "nearest" });
    const layout = () => {
      const r = el?.getBoundingClientRect();
      const { left, top } = place(r && r.width > 0 ? r : null, card.offsetWidth, card.offsetHeight, innerWidth, innerHeight);
      card.style.left = `${left}px`;
      card.style.top = `${top}px`;
    };
    layout();
    card.querySelector<HTMLElement>(".primary")?.focus();
    const key = (e: KeyboardEvent) => e.key === "Escape" && close();
    addEventListener("resize", layout);
    document.addEventListener("keydown", key);
    return () => {
      el?.removeAttribute("data-tour-active");
      removeEventListener("resize", layout);
      document.removeEventListener("keydown", key);
    };
  }, [open, i]);
  const last = i === STEPS.length - 1;
  return (
    <dialog ref={ref} class="tour" open={open} aria-labelledby="tour-title">
      <header>
        <h2 id="tour-title">{step.title}</h2>
        <button type="button" class="icon" title="Close the tour" aria-label="Close the tour" onClick={close}><X /></button>
      </header>
      <p>{step.text}</p>
      <div class="actions">
        <span class="count">{i + 1} of {STEPS.length}</span>
        <button type="button" disabled={i === 0} onClick={() => setI((j) => Math.max(0, j - 1))}>Back</button>
        <button type="button" class="primary" onClick={() => (last ? close() : setI((j) => Math.min(STEPS.length - 1, j + 1)))}>{last ? "Done" : "Next"}</button>
      </div>
    </dialog>
  );
}
