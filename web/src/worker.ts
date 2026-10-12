// The worker entry: loads the module, then hands every command to the protocol.
import init, { Session } from "../pkg/plotline_wasm.js";
import { Protocol, type Command } from "./protocol";

// Commands that arrive while the module loads wait here and run once it is ready.
const queue: Command[] = [];
let protocol: Protocol | undefined;
// Why the module did not load; once set, every command is refused with it.
let failed: string | undefined;
const refuse = (cmd: Command) => self.postMessage({ id: cmd.id, ok: false, error: failed });
self.onmessage = (e: MessageEvent<Command>) => {
  if (protocol) protocol.handle(e.data);
  else if (failed !== undefined) refuse(e.data);
  else queue.push(e.data);
};

// No top-level await: Vite builds workers as IIFE by default, which cannot hold one.
init().then(() => {
  protocol = new Protocol(new Session(), (msg, transfer) => self.postMessage(msg, { transfer: transfer ?? [] }));
  for (const cmd of queue.splice(0)) protocol.handle(cmd);
}).catch((e) => {
  failed = e instanceof Error ? e.message : String(e);
  for (const cmd of queue.splice(0)) refuse(cmd);
});
