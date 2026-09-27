import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import vm from 'node:vm';

const root = path.resolve(import.meta.dirname, '../../..');
const lab = path.join(root, 'tools/lab');

const failure = {
  node: 7,
  attempted_round: 100,
  completed_round: 99,
  observed_round: 100,
  phase: 'BoundaryPreflight',
  violation: {
    pre_len: 47,
    log_len: 119,
    first_mismatch: 41,
    expected: null,
    actual: null,
    command_prefix_ok: false,
  },
};
const failedTerminal = {
  Failed: {
    attempted_round: 100,
    completed_round: 99,
    observed_round: 100,
    phase: 'BoundaryPreflight',
  },
};
const metric = round => ({
  round,
  blocked: 0,
  nonbot_logs: 598,
  useful: 598,
  max_log_len: 119,
  min_executed_len: 47,
  max_executed_len: 47,
  lcp_len: 47,
});
const recoveryRow = round => ({
  noreset: 598,
  reset: 0,
  bot_r: 0,
  window: Math.floor(round / 20),
  rollbacks: 0,
  max_checkpoint_window: 5,
  rollback_depth: 0,
  cp_fork_k: 1,
  max_cp_p_len: 47,
});
const report = (tag, failed = false) => ({
  tag,
  metrics: failed ? Array.from({ length: 100 }, (_, i) => metric(i + 1)) : [],
  commands: [],
  terminal: failed ? failedTerminal : 'MaxRounds',
  safety_ok: true,
  ...(failed ? {
    failure,
    recovery: { fork_ok: true, rounds: Array.from({ length: 100 }, (_, i) => recoveryRow(i + 1)) },
  } : {}),
});
const certs = () => ({ servers: [], committed: [], clients: [], merges_last_step: [], roots_consistent: true });

class Element {
  constructor(id = '') {
    this.id = id;
    this.value = '';
    this.textContent = '';
    this.innerHTML = '';
    this.children = [];
    this.listeners = {};
    this.dataset = {};
    this.clientWidth = 0;
    this.clientHeight = 0;
    this.classes = new Set();
    this.classList = {
      add: (...xs) => xs.forEach(x => this.classes.add(x)),
      remove: (...xs) => xs.forEach(x => this.classes.delete(x)),
      contains: x => this.classes.has(x),
      toggle: (x, on = !this.classes.has(x)) => {
        on ? this.classes.add(x) : this.classes.delete(x);
        return on;
      },
    };
    this.queries = new Map();
  }
  addEventListener(type, fn) { (this.listeners[type] ??= []).push(fn); }
  dispatchEvent(event) { for (const fn of this.listeners[event.type] || []) fn(event); }
  click() { this.dispatchEvent({ type: 'click', target: this, stopPropagation() {} }); }
  querySelector(query) {
    if (!this.queries.has(query)) this.queries.set(query, new Element(query));
    return this.queries.get(query);
  }
  querySelectorAll() { return []; }
  appendChild(child) { this.children.push(child); return child; }
  getBoundingClientRect() { return { left: 0, top: 0, width: 0, height: 0 }; }
  remove() {}
}

function workerApi(owner) {
  let panic = null;
  return {
    initSync() {},
    last_panic: () => panic,
    run_smr_json(json) {
      const spec = JSON.parse(json);
      owner.lastSpec = spec;
      if (owner.panic) {
        panic = owner.panic;
        throw new WebAssembly.RuntimeError('unreachable');
      }
      const result = report(`batch-${spec.seed}`, owner.batchFailed);
      if (owner.batchFailed && owner.failedRelease) {
        result.metrics[0].blocked = spec.n;
        result.metrics[1].blocked = 0;
      }
      return JSON.stringify({ report: result });
    },
    LiveSmr: {
      new_with_mode(json) {
        const spec = JSON.parse(json);
        owner.lastSpec = spec;
        let round = 0;
        const fractions = [];
        let failed = false;
        const counts = owner.calls;
        return {
          free() {},
          step(fraction) {
            if (owner.panic) {
              panic = owner.panic;
              throw new WebAssembly.RuntimeError('unreachable');
            }
            if (failed) return JSON.stringify({ round, failure, absorbed: true });
            round++;
            fractions.push(fraction);
            failed = owner.liveFailsAt === round;
            return JSON.stringify({ round, failure: failed ? failure : null, absorbed: false, dead: null, nodes: [], blocked: [], spreading: [] });
          },
          report_json() { counts.report++; return JSON.stringify(report(`live-${spec.seed}-r${round}`, failed)); },
          export_scenario() {
            counts.export++;
            return JSON.stringify({ ...spec, max_rounds: round, schedule: { kind: 'per_round_fractions', fractions: [...fractions] } });
          },
          certs_json() { counts.certs++; return JSON.stringify(certs()); },
          chain_json() { counts.chain++; return JSON.stringify({ m: 0, entries: [], peaks: [], merges: [] }); },
          inject() { counts.mutate++; return '{}'; },
          set_traffic() { counts.mutate++; return '{}'; },
          set_block() { counts.mutate++; return '{}'; },
          node_detail() { counts.detail++; return '{}'; },
          capture_stale() { counts.detail++; return '{}'; },
          verify_json() { counts.detail++; return '{}'; },
        };
      },
    },
  };
}

async function setup() {
  const elements = new Map();
  const $ = id => {
    if (!elements.has(id)) elements.set(id, new Element(id));
    return elements.get(id);
  };
  for (const [id, value] of Object.entries({
    smrN: '598', smrSeed: '1', smrSigma: '1', smrMax: '200', smrBeta: '10',
    smrT: '8', smrTw: '20', smrLiveMode: 'fresh', smrArrRate: '',
    smrAuditScope: 'all', smrSlider: '30', smrSpeed: '1',
  })) $(id).value = value;
  $('smrAbort').classList.add('hidden');
  const S = { engine: 'loading', view: 'smr', live: null };
  const downloads = [];
  const workers = [];
  const timers = new Map();
  let timerId = 0;
  const document = {
    getElementById: $,
    querySelectorAll: () => [],
    addEventListener() {},
    createElement: tag => new Element(tag),
    createElementNS: (_, tag) => new Element(tag),
  };
  const context = vm.createContext({
    console,
    document,
    window: { devicePixelRatio: 1, addEventListener() {} },
    Event: class { constructor(type) { this.type = type; } },
    setTimeout: fn => { const id = ++timerId; timers.set(id, fn); return id; },
    clearTimeout: id => timers.delete(id),
  });

  class Worker {
    constructor() {
      this.terminated = false;
      this.outbox = [];
      this.batchFailed = false;
      this.failedRelease = false;
      this.liveFailsAt = null;
      this.panic = null;
      this.hold = false;
      this.held = [];
      this.lastSpec = null;
      this.calls = { report: 0, export: 0, certs: 0, chain: 0, mutate: 0, detail: 0 };
      workers.push(this);
      const workerContext = vm.createContext({
        self: { location: { search: '' } },
        importScripts() {},
        wasm_bindgen: workerApi(this),
        fetch: async () => ({ ok: true, arrayBuffer: async () => new ArrayBuffer(0) }),
        postMessage: message => {
          this.outbox.push(message);
          if (this.hold) this.held.push(message);
          else if (!this.terminated) this.onmessage?.({ data: message });
        },
        performance,
        console,
        WebAssembly,
      });
      vm.runInContext(fs.readFileSync(path.join(lab, 'worker.js'), 'utf8'), workerContext);
      this.workerContext = workerContext;
    }
    postMessage(message) { this.workerContext.onmessage({ data: message }); }
    release() {
      this.hold = false;
      for (const message of this.held.splice(0)) {
        if (!this.terminated) this.onmessage?.({ data: message });
      }
    }
    terminate() { this.terminated = true; }
  }
  context.Worker = Worker;

  const stubs = {
    'state.js': { S, $, PALETTE: ['#000'], AUTO_CLIENT_BASE: 2147483648 },
    'form.js': { downloadJson: (name, json) => downloads.push({ name, data: JSON.parse(json) }) },
    'pmf-editor.js': { createPmfEditor: () => ({ weights: () => [1], draw() {} }) },
    'smr-certs.js': {
      onCertsSnapshot() {}, onChainSnapshot() {}, freezeChain() {},
      useEntrySelect() {}, resetCertsDrawer() {}, onVerifyTallies() {},
    },
    'smr-timeline.js': { drawTimeline() {}, useSpreadCurves() {} },
    'render.js': { update() {}, draw() {}, setView() {} },
    'trace.js': { loadTrace() {} },
    'live.js': { onLiveRound() {}, updateLivePlayBtn() {} },
    'controls.js': {},
    'spill.js': { onSpillReady() {}, onSpillChain() {} },
  };
  const modules = new Map();
  const load = requested => {
    const name = path.basename(requested);
    if (modules.has(name)) return modules.get(name);
    const module = stubs[name]
      ? new vm.SyntheticModule(Object.keys(stubs[name]), function () {
          for (const [key, value] of Object.entries(stubs[name])) this.setExport(key, value);
        }, { context, identifier: name })
      : new vm.SourceTextModule(fs.readFileSync(path.join(lab, name), 'utf8'), { context, identifier: name });
    modules.set(name, module);
    return module;
  };
  const main = load('main.js');
  await main.link(load);
  await main.evaluate();
  const flush = async () => { for (let i = 0; i < 16; i++) await Promise.resolve(); };
  await flush();
  return { S, $, downloads, workers, timers, flush };
}

const hidden = element => element.classList.contains('hidden');

{
  const h = await setup();
  const worker = h.workers[0];
  worker.batchFailed = true;
  h.$('smrRunBtn').click();
  await h.flush();
  assert.equal(worker.terminated, false, 'typed batch failure keeps the worker');
  assert.equal(h.workers.length, 1, 'typed batch failure does not respawn');
  assert.equal(h.S.smr.report.failure.node, 7);
  assert.equal(h.S.smr.scenario.seed, 1);
  assert.match(h.$('smrStatus').textContent, /FAILED.*attempted 100.*completed 99.*observed 100/i);
  assert.equal(hidden(h.$('smrAbort')), false);

  h.$('smrSeed').value = '99';
  h.$('smrExportSpec').click();
  h.$('smrExportReport').click();
  assert.equal(h.downloads[0].data.seed, 1, 'failure export uses captured input, not edited form');
  assert.equal(h.downloads[1].data.failure.node, 7);
}

{
  const h = await setup();
  const worker = h.workers[0];
  h.S.smr.proto = 'recovery';
  h.S.smr.ack = true;
  h.$('smrTw').value = '20';
  worker.batchFailed = true;
  worker.hold = true;
  h.$('smrRunBtn').click();
  await h.flush();

  h.S.smr.proto = 'compact';
  h.$('smrSeed').value = '99';
  h.$('smrT').value = '8';
  worker.release();
  await h.flush();

  assert.equal(h.S.smr.proto, 'compact', 'failed display must not overwrite editable next-run config');
  assert.match(h.$('smrSumChip').textContent, /n=598.*seed 1.*recovery T=20/);
  assert.equal(hidden(h.$('smrTimelineBox')), false, 'captured recovery panels stay visible');
  assert.equal(hidden(h.$('smrSpreadBox')), true, 'compact panels do not replace failed recovery panels');

  h.$('smrSeed').value = '77';
  h.$('smrExportSpec').click();
  assert.equal(h.downloads.at(-1).data.seed, 1, 'post-failure edits do not change captured export');
  assert.equal(h.downloads.at(-1).data.proto.kind, 'recovery');
  assert.equal(h.downloads.at(-1).data.proto.t_window_rounds, 20);

  worker.batchFailed = false;
  h.$('smrRunBtn').click();
  await h.flush();
  assert.equal(worker.lastSpec.seed, 77, 'explicit next run uses edited form');
  assert.equal(worker.lastSpec.proto.kind, 'compact');
  assert.equal(h.S.smr.report.tag, 'batch-77');
  assert.equal(hidden(h.$('smrAbort')), true);
}

{
  const h = await setup();
  h.$('smrRunBtn').click();
  await h.flush();
  assert.equal(h.S.smr.report.tag, 'batch-1');
  h.$('smrSeed').value = '2';
  h.workers[0].batchFailed = true;
  h.$('smrRunBtn').click();
  await h.flush();
  assert.equal(h.S.smr.report.tag, 'batch-2', 'new failed report replaces prior success');
  assert.equal(h.S.smr.report.failure.node, 7);
}

{
  const h = await setup();
  const worker = h.workers[0];
  worker.liveFailsAt = 2;
  h.$('smrLiveBtn').click();
  await h.flush();
  h.$('smrSlider').value = '30';
  h.$('smrStepBtn').click();
  await h.flush();
  h.$('smrSlider').value = '40';
  h.$('smrStepBtn').click();
  await h.flush();

  assert.equal(h.S.smr.live, null, 'failed live session cannot resume');
  assert.equal(JSON.stringify(h.S.smr.scenario.schedule.fractions), '[0.3,0.4]');
  assert.equal(worker.calls.certs, 2, 'failure step skips certificate read');
  assert.equal(worker.calls.chain, 2, 'failure step skips chain read');
  assert.equal(worker.calls.report, 3, 'initial and both step reports are retained');
  assert.equal(worker.calls.export, 1, 'failed replay captured exactly once');
  assert.equal(h.workers.length, 1, 'typed live failure does not respawn');
  assert.equal(h.timers.size, 0, 'failed session leaves no autoplay callback');

  h.$('smrSeed').value = '88';
  h.$('smrExportSpecLive').click();
  assert.equal(JSON.stringify(h.downloads.at(-1).data.schedule.fractions), '[0.3,0.4]');

  worker.liveFailsAt = null;
  h.$('smrLiveBtn').click();
  await h.flush();
  assert.ok(h.S.smr.live, 'explicit new session starts cleanly');
  assert.equal(hidden(h.$('smrAbort')), true, 'new session clears failure banner');
}

{
  const h = await setup();
  const worker = h.workers[0];
  worker.liveFailsAt = 2;
  h.$('smrLiveBtn').click();
  await h.flush();
  h.$('smrPlayBtn').click();
  await h.flush();
  assert.equal(h.S.smr.lastStatus.round, 1);
  assert.ok(h.timers.size > 0, 'autoplay queued work for the first session');

  h.$('smrStepBtn').click();
  await h.flush();
  assert.equal(h.S.smr.live, null, 'manual step reached typed failure');
  const stale = [...h.timers.values()];
  h.timers.clear();

  worker.liveFailsAt = null;
  h.$('smrLiveBtn').click();
  await h.flush();
  assert.equal(h.S.smr.lastStatus, null);
  assert.equal(h.S.smr.live.playing, false);
  for (const callback of stale) callback();
  await h.flush();
  assert.equal(h.S.smr.lastStatus, null, 'old autoplay must not advance the new session');

  h.$('smrStepBtn').click();
  await h.flush();
  assert.equal(h.S.smr.lastStatus.round, 1, 'manual stepping remains available');
}

{
  const h = await setup();
  const worker = h.workers[0];
  h.S.smr.proto = 'recovery';
  h.S.smr.ack = true;
  h.$('smrTw').value = '20';
  worker.batchFailed = true;
  worker.failedRelease = true;
  worker.hold = true;
  h.$('smrRunBtn').click();
  await h.flush();
  worker.release();
  await h.flush();
  assert.doesNotMatch(h.$('smrPhaseChip').textContent, /CONFIRMED/);
  assert.doesNotMatch(h.$('smrClockChip').textContent, /r\* 20|R 18/);
  assert.equal(hidden(h.$('smrLawCard')), true);
}

{
  const h = await setup();
  const oldWorker = h.workers[0];
  oldWorker.panic = 'unexpected panic';
  h.$('smrRunBtn').click();
  await h.flush();
  assert.equal(oldWorker.terminated, true);
  assert.equal(h.workers.length, 2, 'unexpected panic still respawns');
}

console.log('smr failure adapter tests passed');
