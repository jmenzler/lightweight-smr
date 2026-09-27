// the ?v= cache-buster from index.html rides on the worker URL; forward it so
// the wasm glue + binary are fetched as fresh as the worker itself
const v = self.location.search || '';
importScripts('./sim_wasm.js' + v);

let ready = fetch('./sim_wasm_bg.wasm' + v)
  .then(r => {
    if (!r.ok) throw new Error('wasm fetch ' + r.status);
    return r.arrayBuffer();
  })
  .then(buf => {
    wasm_bindgen.initSync({ module: buf });
    postMessage({ type: 'ready' });
  })
  .catch(e => postMessage({ type: 'initfail', error: String(e) }));

let live = null;
let smrLive = null;
let spill = null;

// Recovery chain: delta-encoded from a cursor the worker advances as commits
// land, so each message carries only the new blocks. {error} rides through
// untouched — the panel freezes on it instead of guessing.
let chainFrom = 0;
function chainDelta() {
  if (!smrLive) return null;
  // wasm-bindgen maps Rust u64 to JS BigInt, like inject's op
  const out = JSON.parse(smrLive.chain_json(BigInt(chainFrom)));
  if (out.error) return out;
  chainFrom = out.m;
  return out;
}

onmessage = ev => {
  const m = ev.data;
  ready.then(() => {
    try {
      switch (m.type) {
        case 'run': {
          const t0 = performance.now();
          const json = wasm_bindgen.run_scenario_json(JSON.stringify(m.spec));
          const out = JSON.parse(json);
          if (out.error) {
            postMessage({ type: 'error', error: out.error });
          } else {
            postMessage({ type: 'done', json, ms: Math.round(performance.now() - t0) });
          }
          break;
        }
        case 'live-new': {
          if (live) { live.free(); live = null; }
          live = wasm_bindgen.LiveSim.new_with_mode(JSON.stringify(m.spec), !!m.sticky);
          postMessage({ type: 'live-ready', json: live.trace_json() });
          break;
        }
        case 'live-step': {
          if (!live) throw new Error('no live session');
          postMessage({ type: 'live-round', json: live.step(m.fraction) });
          break;
        }
        case 'live-export': {
          if (!live) throw new Error('no live session');
          postMessage({ type: 'live-scenario', json: live.export_scenario() });
          break;
        }
        case 'live-trace': {
          if (!live) throw new Error('no live session');
          postMessage({ type: 'live-tracejson', json: live.trace_json() });
          break;
        }
        case 'smr-run': {
          const t0 = performance.now();
          const json = wasm_bindgen.run_smr_json(JSON.stringify(m.spec));
          const out = JSON.parse(json);
          if (out.error) {
            postMessage({ type: 'error', error: out.error });
          } else if (out.report.failure) {
            postMessage({
              type: 'smr-failed',
              mode: 'batch',
              report: JSON.stringify(out.report),
              scenario: JSON.stringify(m.spec),
              failure: out.report.failure,
              ms: Math.round(performance.now() - t0),
            });
          } else {
            postMessage({ type: 'smr-done', json, ms: Math.round(performance.now() - t0) });
          }
          break;
        }
        case 'smr-live-new': {
          if (smrLive) { smrLive.free(); smrLive = null; }
          chainFrom = 0;
          smrLive = wasm_bindgen.LiveSmr.new_with_mode(JSON.stringify(m.spec), !!m.sticky);
          postMessage({ type: 'smr-live-ready', report: smrLive.report_json(), certs: smrLive.certs_json(), chain: chainDelta() });
          break;
        }
        case 'smr-live-step': {
          if (!smrLive) throw new Error('no smr live session');
          const status = smrLive.step(m.fraction);
          const parsed = JSON.parse(status);
          if (parsed.failure) {
            const report = smrLive.report_json();
            const scenario = smrLive.export_scenario();
            postMessage({
              type: 'smr-failed',
              mode: 'live',
              status,
              report,
              scenario,
              failure: parsed.failure,
            });
          } else {
            postMessage({ type: 'smr-round', status, report: smrLive.report_json(), certs: smrLive.certs_json(), chain: chainDelta() });
          }
          break;
        }
        case 'smr-live-inject': {
          if (!smrLive) throw new Error('no smr live session');
          // wasm-bindgen maps Rust u64 to JS BigInt
          postMessage({ type: 'smr-injected', quiet: !!m.quiet, json: smrLive.inject(m.client, BigInt(m.op), m.target === null ? undefined : m.target), certs: smrLive.certs_json(), chain: chainDelta() });
          break;
        }
        case 'smr-capture-stale': {
          if (!smrLive) throw new Error('no smr live session');
          postMessage({ type: 'smr-captured', json: smrLive.capture_stale(m.client), certs: smrLive.certs_json() });
          break;
        }
        case 'smr-verify-all': {
          if (!smrLive) throw new Error('no smr live session');
          postMessage({ type: 'smr-verify', json: smrLive.verify_json() });
          break;
        }
        case 'smr-live-traffic': {
          if (!smrLive) throw new Error('no smr live session');
          postMessage({ type: 'smr-traffic-set', json: smrLive.set_traffic(JSON.stringify(m.pmf)) });
          break;
        }
        case 'smr-live-export': {
          if (!smrLive) throw new Error('no smr live session');
          postMessage({ type: 'smr-scenario', json: smrLive.export_scenario() });
          break;
        }
        case 'smr-node-detail': {
          if (!smrLive) throw new Error('no smr live session');
          postMessage({ type: 'smr-node-detail', node: m.node, pinned: !!m.pinned, json: smrLive.node_detail(m.node) });
          break;
        }
        case 'smr-live-block': {
          if (!smrLive) throw new Error('no smr live session');
          postMessage({ type: 'smr-blocked', node: m.node, blocked: !!m.blocked, json: smrLive.set_block(m.node, !!m.blocked) });
          break;
        }
        // The spill reader is Rust: parsing, reconstruction and digest
        // verification all happen inside SpillView, never in JS.
        case 'spill-load': {
          if (spill) { spill.free(); spill = null; }
          try {
            spill = new wasm_bindgen.SpillView(m.text);
          } catch (e) {
            postMessage({ type: 'spill-ready', json: JSON.stringify({ error: String(e) }) });
            break;
          }
          postMessage({ type: 'spill-ready', json: spill.summary_json() });
          break;
        }
        case 'spill-chain': {
          if (!spill) throw new Error('no spill loaded');
          postMessage({
            type: 'spill-chain',
            boundary: m.boundary,
            node: m.node,
            json: spill.chain_json(m.boundary, m.node, 0n),
          });
          break;
        }
      }
    } catch (e) {
      // A Rust panic reaches us as an opaque trap; the hook kept the real
      // message. The engine's memory is unusable afterwards, so drop the
      // session handle and let the main thread respawn.
      const panic = wasm_bindgen.last_panic();
      if (panic) { try { smrLive.free(); } catch (_) { /* already gone */ } smrLive = null; }
      postMessage({ type: 'error', error: String(e), panic: panic || null });
    }
  });
};
