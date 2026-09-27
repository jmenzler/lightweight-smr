// WASM worker wrapper — main.js supplies the message dispatch
// cache-busted: Chrome caches worker scripts aggressively across deploys
let worker = new Worker('./worker.js?v=' + Date.now());
let handler = null;
export const post = msg => worker.postMessage(msg);
export function onMessage(h) { handler = h; worker.onmessage = ev => handler(ev.data); }
// A trapped panic leaves wasm memory untrustworthy — the only safe recovery
// is a fresh worker, which re-inits the module from scratch.
export function respawn() {
  worker.terminate();
  worker = new Worker('./worker.js?v=' + Date.now());
  if (handler) worker.onmessage = ev => handler(ev.data);
}
