// Undo and redo over whole setups (spec 2026-10-15-try-styles-and-undo section 3). A setup is the
// preset's style, canvas, paper, pens, machine and settings, kept as JSON text so equal setups compare
// equal as strings.
import type { State } from "./protocol";

export const setupOf = ({ style, canvas, paper, pens, machine, settings }: State): string =>
  JSON.stringify({ style, canvas, paper, pens, machine, settings });

export class History {
  past: string[] = [];
  future: string[] = [];
  private now: string | null = null;

  constructor(private readonly limit = 100) {}

  /** Takes the setup a reply shows. A different one is a new step unless `restored`: the old one
   * goes on the undo stack and the redo stack empties. */
  see(setup: string, restored = false): void {
    if (!restored && this.now !== null && setup !== this.now) {
      this.past.push(this.now);
      if (this.past.length > this.limit) this.past.shift();
      this.future = [];
    }
    this.now = setup;
  }

  /** The setup to restore, or undefined with nothing to undo. */
  undo(): string | undefined {
    return this.move(this.past, this.future);
  }

  /** The setup to restore, or undefined with nothing to redo. */
  redo(): string | undefined {
    return this.move(this.future, this.past);
  }

  private move(from: string[], to: string[]): string | undefined {
    const setup = from.pop();
    if (setup === undefined || this.now === null) return undefined;
    to.push(this.now);
    this.now = setup;
    return setup;
  }
}
