// The guided tour: a card that walks through the page one feature at a time, ringing the element
// it talks about. A step names its element by the element's data-tour attribute; a step without
// one, or whose element is not on screen, shows in the middle of the window. A new feature adds
// its step to STEPS, and the test checks that every target is on an element.
import { X } from "lucide-preact";
import type { JSX } from "preact";
import { useEffect, useLayoutEffect, useRef, useState } from "preact/hooks";

export type Step = { target?: string; title: string; text: string };

export const STEPS: Step[] = [
  { title: "Welcome to plotline", text: "plotline turns an image into lines a pen plotter can draw. This tour shows where everything is. Tour in the toolbar opens it again." },
  { target: "image", title: "Open an image", text: "Open a PNG, JPEG or WebP, or drop or paste one anywhere on the page. Sample image opens a still life to try things on." },
  { target: "suggestions", title: "Take a suggestion", text: "Once an image is open, plotline lists what would suit it, such as turning the page, each with an Apply button. Nothing changes until you click. For a colour image it picks two to four ink pens that draw its colours best and sets Pen use to by colour." },
  { target: "style", title: "Pick a style", text: "Each style turns the image's tone into a different kind of mark. The line under the menu says what the chosen one does. A switch keeps the settings you changed wherever the new style has them, so the Voronoi styles reuse their points without placing them again." },
  { target: "look", title: "Pick a look", text: "Most styles offer a few looks, chosen by measuring test images: Balanced is the defaults, Faithful follows the image's tones most closely, Quick plots in about half an hour, and Fine draws the densest marks that still plot cleanly. A look changes only the few settings that set it, so the image, page, pens and Pen use stay; Custom shows once you change one of them, and Undo takes a look back." },
  { target: "styles", title: "Try every style", text: "Try all styles draws every style small on your image, page and pens, so you can choose by eye. Point at a picture, or tab to it, to see it larger, and click it to switch to that style. Closing the window stops the drawing and leaves your own drawing as it was." },
  { target: "reset", title: "Start over", text: "Reset to defaults puts every setting of the chosen style back to its default, Pen use included. The image, page and pens stay as they are, and Undo takes the reset back." },
  { target: "settings", title: "Shape the drawing", text: "The style's own settings come first. Path order and Image, with the levels, gamma and blur, are collapsed below. A change reruns only the stages after it." },
  { target: "run", title: "Run it", text: "Run draws the image with these settings. Stop ends a run after its current step." },
  { target: "undo", title: "Undo a change", text: "Undo and Redo step back and forth through changes to the style, its settings, the page and the pens, so trying something costs nothing. Ctrl+Z and Ctrl+Shift+Z do the same, except while you type in a field." },
  { target: "status", title: "Follow the run", text: "Each stage shows its progress here. Under it are the path count, the pen-down and pen-up lengths and an estimate of the plot time." },
  { target: "lint", title: "Check the plot", text: "After each run plotline checks the drawing for trouble on paper: marks drawn over each other, specks, many pen lifts, a drawing too light for the image's darks, marks off the page. Apply changes the setting that fixes one." },
  { target: "fit", title: "Fit a budget", text: "Type a plot time in minutes, a path count, metres of pen-down line or a dot count, and press Fit to target. plotline reruns the style up to eight times with its density setting, such as the line spacing, changed, and keeps the run nearest the target. The plot time leaves out acceleration, so styles of short strokes or dots plot slower than it says." },
  { target: "optimize", title: "Match the image", text: "The tone error after the stats says how far the drawing's tones are from the image's, seen from a metre or two; lower is closer. Optimize reruns the style eight times with its density setting changed and suggests the value that brings the tones closest, within the target if you typed one; Apply takes it, and nothing changes until you do. Response in the Image settings goes further: on auto it measures how much paper the style covers at each grey and corrects the image for it. Matching the image is not always the aim, so both are yours to choose." },
  { target: "preview", title: "Look closely", text: "The preview draws every mark at the pen's real width and colour. Drag to pan and scroll to zoom. With the preview focused, the arrow keys pan, plus and minus zoom and 0 fits the page." },
  { target: "view", title: "Check the drawing", text: "Source shows the image under the drawing. Travel shows the pen-up moves between paths. Fit page shows the whole page again." },
  { target: "page", title: "Set the page", text: "The paper size, orientation and margin. Fit keeps the whole image on the page; fill covers the drawing area and crops the rest." },
  { target: "pens", title: "Choose pens", text: "Edit pens opens the pens dialog: pick a maker's pen from the bank at its published line width, or set a width yourself, and choose an ink colour. The tick hides a pen in the preview. Pen use shares the drawing between the pens: light to dark in list order, or each pen by its colour; pens plot in list order, so list light inks first." },
  { target: "export", title: "Export", text: "SVG writes the drawing in millimetres, one layer per pen that draws, for your plotter's own software. JSON writes the paths for other programs." },
  { target: "presets", title: "Keep a setup", text: "A preset is a small file with the style, page, pens and settings, so you can come back to a setup or share it. The page also remembers your last setup." },
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
// it from the first step.
export function Tour({ opens }: { opens: number }): JSX.Element {
  const ref = useRef<HTMLDialogElement>(null);
  const [open, setOpen] = useState(false);
  const [i, setI] = useState(0);
  const step = STEPS[i];
  useEffect(() => {
    if (opens > 0) {
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
    const key = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
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
        <button type="button" class="icon" title="Close the tour" aria-label="Close the tour" onClick={() => setOpen(false)}><X /></button>
      </header>
      <p>{step.text}</p>
      <div class="actions">
        <span class="count">{i + 1} of {STEPS.length}</span>
        <button type="button" disabled={i === 0} onClick={() => setI((j) => Math.max(0, j - 1))}>Back</button>
        <button type="button" class="primary" onClick={() => (last ? setOpen(false) : setI((j) => Math.min(STEPS.length - 1, j + 1)))}>{last ? "Done" : "Next"}</button>
      </div>
    </dialog>
  );
}
