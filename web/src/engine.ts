// The page's side of the worker: commands out as promises, run events out to `onEvent`.
import type { Event } from "./events";
import type { Reply, State } from "./protocol";

export class Engine {
  onEvent: (event: Event) => void = () => {};
  private nextId = 0;
  private failed: string | null = null;
  private pending = new Map<number, (reply: Reply) => void>();

  constructor(private readonly worker: Worker) {
    worker.onmessage = (e: MessageEvent<Reply | Event>) => {
      const msg = e.data;
      if ("type" in msg) return this.onEvent(msg);
      this.pending.get(msg.id)?.(msg);
      this.pending.delete(msg.id);
    };
    // A worker that failed (its module did not load, or it threw) answers nothing more, so its
    // calls are refused, those waiting and those to come.
    worker.onerror = (e) => {
      this.failed = e.message || "the engine stopped";
      for (const [id, settle] of this.pending) settle({ id, ok: false, error: this.failed });
      this.pending.clear();
    };
  }

  call<T = undefined>(method: string, args: unknown[] = [], transfer: Transferable[] = []): Promise<{ state: State; value: T }> {
    if (this.failed !== null) return Promise.reject(new Error(this.failed));
    const id = ++this.nextId;
    return new Promise((resolve, reject) => {
      this.pending.set(id, (reply) => (reply.ok ? resolve({ state: reply.state, value: reply.value as T }) : reject(new Error(reply.error))));
      this.worker.postMessage({ id, method, args }, transfer);
    });
  }
}
