// The worker entry: loads the module, then hands every command to the protocol.
import init, { Session } from "../pkg/plotline_wasm.js";
import { loadModelsModule } from "./modelsModule";
import { Protocol, type Command } from "./protocol";

// Commands wait here, in order, while the engine module loads or the model module loads for an
// `addModel`, and run once it is ready.
const queue: Command[] = [];
let busy = true;
let protocol: Protocol | undefined;
// Why the module did not load; once set, every command is refused with it.
let failed: string | undefined;
const message = (e: unknown) => (e instanceof Error ? e.message : String(e));
const refuse = (cmd: Command, error = failed) => self.postMessage({ id: cmd.id, ok: false, error });

function take(cmd: Command): void {
  if (failed !== undefined) return refuse(cmd);
  if (busy) return void queue.push(cmd);
  if (cmd.method !== "addModel") return protocol!.handle(cmd);
  busy = true;
  loadModelsModule().then(() => protocol!.handle(cmd), (e) => refuse(cmd, message(e))).finally(ready);
}
function ready(): void {
  busy = false;
  while (!busy && queue.length) take(queue.shift()!);
}
self.onmessage = (e: MessageEvent<Command>) => take(e.data);

// Commands can arrive before init() resolves, so the handler above is set first and queues them.
init().then(() => {
  protocol = new Protocol(new Session(), (msg, transfer) => self.postMessage(msg, { transfer: transfer ?? [] }));
  ready();
}).catch((e) => {
  failed = message(e);
  for (const cmd of queue.splice(0)) refuse(cmd);
});
