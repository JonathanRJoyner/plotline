// The page's side of the worker: commands out as promises, run events out to `onEvent`.
import type { Event } from "./events";
import type { Reply, State } from "./protocol";

export class Engine {
  onEvent: (event: Event) => void = () => {};
  private nextId = 0;
  private pending = new Map<number, (reply: Reply) => void>();

  constructor(private readonly worker: Worker) {
    worker.onmessage = (e: MessageEvent<Reply | Event>) => {
      const msg = e.data;
      if ("type" in msg) return this.onEvent(msg);
      this.pending.get(msg.id)?.(msg);
      this.pending.delete(msg.id);
    };
  }

  call<T = undefined>(method: string, args: unknown[] = [], transfer: Transferable[] = []): Promise<{ state: State; value: T }> {
    const id = ++this.nextId;
    return new Promise((resolve, reject) => {
      this.pending.set(id, (reply) => (reply.ok ? resolve({ state: reply.state, value: reply.value as T }) : reject(new Error(reply.error))));
      this.worker.postMessage({ id, method, args }, transfer);
    });
  }
}
