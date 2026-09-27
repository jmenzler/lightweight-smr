// SMR view: run form + injections editor, batch render, live loop with
// interactive injection, audit-drawer command list, spread/position charts,
// prefix ribbon. Shell: A2 (spec 2026-07-23-lab-gui-revamp-design.md).
import { S, $, PALETTE, AUTO_CLIENT_BASE } from './state.js';
import { post } from './engine.js';
import { downloadJson } from './form.js';
import { createPmfEditor } from './pmf-editor.js';
import { onCertsSnapshot, onChainSnapshot, freezeChain, useEntrySelect, resetCertsDrawer, onVerifyTallies } from './smr-certs.js';
import { drawTimeline, useSpreadCurves } from './smr-timeline.js';

S.smr = { report: null, live: null, scenario: null, resultContext: null, nextClient: 1, nextOp: 1, lastStatus: null, sel: null, proto: 'extended', chart: 'spread', tw: null, program: null, detail: null, pin: null, manualBlocks: new Set(), grid: 'watched', batchSel: null, chain: null, ack: false, certsRec: false };

/* ---------- arrival traffic (engine pmf) ---------- */

const ARR = createPmfEditor('arr', 'rowArr');

// fixed rate r = point mass at index r; fractional = two-point mixture
function rateToPmf(rate) {
  const r = +rate;
  if (Number.isInteger(r)) {
    const w = new Array(r + 1).fill(0);
    w[r] = 1;
    return w;
  }
  return [1 - (r - Math.floor(r)), r - Math.floor(r)];
}

function formTrafficSpec() {
  const v = $('smrArrRate').value;
  if (v === '') return null;
  return { kind: 'pmf', arrivals_pmf: v === 'custom' ? ARR.weights() : rateToPmf(v) };
}

$('smrArrRate').addEventListener('change', () => {
  $('rowArr').classList.toggle('hidden', $('smrArrRate').value !== 'custom');
  if ($('smrArrRate').value === 'custom') ARR.draw();
});

/* ---------- shell: popovers, audit drawer, chart tabs, summary chip ---------- */

for (const pop of [$('smrInjectPop'), $('smrTrafficPop')]) {
  pop.querySelector('.popBtn').addEventListener('click', e => {
    e.stopPropagation();
    for (const o of document.querySelectorAll('.pop.open')) if (o !== pop) o.classList.remove('open');
    pop.classList.toggle('open');
  });
}
document.addEventListener('click', e => {
  if (!e.target.closest('.pop')) for (const o of document.querySelectorAll('.pop.open')) o.classList.remove('open');
});

$('auditClosedTag').addEventListener('click', () => setAuditOpen(true));
$('auditCollapse').addEventListener('click', () => setAuditOpen(false));
function setAuditOpen(open) {
  $('smrAuditCol').classList.toggle('open', open);
  $('smrAuditCol').classList.toggle('closed', !open);
  $('auditClosedTag').classList.toggle('hidden', open);
  $('auditOpenWrap').classList.toggle('hidden', !open);
}

for (const b of document.querySelectorAll('#smrChartTabs .ct')) {
  b.addEventListener('click', () => {
    S.smr.chart = b.dataset.chart;
    for (const x of document.querySelectorAll('#smrChartTabs .ct')) x.classList.toggle('on', x === b);
    if (S.smr.report) drawSpread(S.smr.report);
  });
}

function sumChipText() {
  const proto = S.smr.proto === 'compact' ? `compact T=${$('smrT').value}`
    : S.smr.proto === 'recovery' ? `recovery T=${$('smrTw').value}` : 'extended';
  const beta = +$('smrBeta').value;
  const mode = $('smrLiveMode').value;
  const rate = $('smrArrRate').value;
  return `n=${$('smrN').value} · seed ${$('smrSeed').value} · ${proto}` +
    (beta ? ` · β ${mode} ${beta}%` : '') + (rate ? ` · ${rate === 'custom' ? 'pmf' : rate + '/r'}` : '') + ' ▾';
}
function scenarioChipText(spec) {
  const proto = spec.proto.kind === 'compact' ? `compact T=${spec.proto.t_commit_rounds}`
    : spec.proto.kind === 'recovery' ? `recovery T=${spec.proto.t_window_rounds}` : 'extended';
  const schedule = spec.schedule;
  const beta = schedule?.kind === 'fresh_per_round' && schedule.fraction
    ? ` · β fresh ${schedule.fraction * 100}%`
    : schedule?.fractions ? ` · β replay ${schedule.fractions.length} rounds` : '';
  return `n=${spec.n} · seed ${spec.seed} · ${proto}${beta} ▾`;
}

function sessionStarted(summary = sumChipText()) {
  $('smrSumChip').textContent = summary;
  $('smrSumChip').classList.remove('hidden');
  $('smrFormCard').classList.add('hidden');
  $('smrFormCard').classList.remove('overlayMode');
}
$('smrSumChip').addEventListener('click', () => {
  const card = $('smrFormCard');
  const opening = card.classList.contains('hidden');
  card.classList.toggle('hidden', !opening);
  card.classList.toggle('overlayMode', opening);
});
function sessionEnded() {
  $('smrSumChip').classList.add('hidden');
  $('smrFormCard').classList.remove('hidden', 'overlayMode');
  $('smrCtl').classList.add('hidden');
}

/* ---------- injections editor ---------- */

function injRow(round = 2, client = S.smr.nextClient, op = S.smr.nextOp) {
  const row = document.createElement('div');
  row.className = 'injRow';
  row.innerHTML = `
    <input class="fInput injRound" type="number" min="1" value="${round}" title="round" />
    <input class="fInput injClient" type="number" min="0" value="${client}" title="client id" />
    <input class="fInput injOp" type="number" min="1" value="${op}" title="op (command id)" />
    <input class="fInput injTarget" type="number" min="0" placeholder="rnd" title="target node — empty = random per attempt (paper client)" />
    <button class="injDel" title="remove">✕</button>`;
  row.querySelector('.injDel').addEventListener('click', () => row.remove());
  $('smrInjRows').appendChild(row);
  S.smr.nextClient = client + 1;
  S.smr.nextOp = op + 1;
}
$('smrInjAdd').addEventListener('click', () => injRow(+$('smrRound').value || 2));

function readInjections() {
  return [...document.querySelectorAll('#smrInjRows .injRow')].map(row => {
    const inj = {
      round: Math.max(1, Math.round(+row.querySelector('.injRound').value || 1)),
      client: Math.max(0, Math.round(+row.querySelector('.injClient').value || 0)),
      op: Math.max(1, Math.round(+row.querySelector('.injOp').value || 1)),
    };
    const t = row.querySelector('.injTarget').value;
    if (t !== '') inj.target = Math.max(0, Math.round(+t));
    return inj;
  });
}

/* ---------- spec building ---------- */

const protoSpec = () => {
  if (S.smr.proto === 'compact') return { kind: 'compact', t_commit_rounds: Math.max(1, Math.round(+$('smrT').value || 8)) };
  if (S.smr.proto === 'recovery') return { kind: 'recovery', t_window_rounds: Math.max(1, Math.round(+$('smrTw').value || 40)), resend_until_acked: S.smr.ack };
  return { kind: 'extended' };
};

export function buildSmrSpec() {
  const proto = protoSpec();
  S.smr.tw = proto.kind === 'recovery' ? proto.t_window_rounds : null;
  S.smr.certsRec = proto.kind === 'recovery' && proto.resend_until_acked;
  const spec = {
    n: Math.min(Math.max(+$('smrN').value || 64, 4), 200000),
    seed: +$('smrSeed').value || 0,
    k: 6,
    ell: 3,
    sigma: Math.max(0.1, +$('smrSigma').value || 1),
    proto,
    injections: readInjections(),
    max_rounds: Math.min(Math.max(+$('smrMax').value || 200, 1), 5000),
    schedule: { kind: 'fresh_per_round', fraction: Math.min(Math.max(+$('smrBeta').value || 0, 0), 60) / 100 },
  };
  const traffic = formTrafficSpec();
  if (traffic) spec.traffic = traffic;
  return spec;
}

for (const btn of $('smrProtoToggle').children) {
  btn.addEventListener('click', () => {
    S.smr.proto = btn.dataset.proto;
    for (const b of $('smrProtoToggle').children) b.classList.toggle('on', b === btn);
    document.querySelectorAll('.compactOnly').forEach(el =>
      el.classList.toggle('hidden', S.smr.proto !== 'compact'));
    document.querySelectorAll('.recoveryOnly').forEach(el =>
      el.classList.toggle('hidden', S.smr.proto !== 'recovery'));
  });
}

for (const btn of $('smrAckToggle').children) {
  btn.addEventListener('click', () => {
    S.smr.ack = btn.dataset.ack === 'on';
    for (const b of $('smrAckToggle').children) b.classList.toggle('on', b === btn);
  });
}

/* ---------- scenario presets (JS-side β programs) ---------- */

// Each preset prefills the form and hands back a β program: piecewise
// [from, to) round ranges driving the slider, so the surge and its release
// happen at exactly the rounds the headline results were measured at.
const PRESETS = {
  custom: null,
  // A full blackout ending strictly INSIDE a window is the regime the
  // release law was measured in: the population is still ⊥ at release, so
  // recovery costs one window to re-arm every server to reset and a second to
  // roll them back. A blackout ending exactly on a boundary gets the re-arm
  // for free and beats the law by a window; a partial surge skips it too.
  surge: { n: 256, tw: 40, sigma: 1, max: 400, beta: 0, rate: '1',
    program: [{ from: 41, to: 111, beta: 100 }] },
  // 30% leaves ~4.2 expected replies — servers drop out without a spiral.
  partial: { n: 256, tw: 40, sigma: 1, max: 400, beta: 0, rate: '1',
    program: [{ from: 41, to: 141, beta: 30 }] },
  // T = 2 sits under the broadcast-time floor, so a sustained-but-survivable
  // β eventually hands a checkpoint a P its log no longer extends — the
  // engine refuses to continue and the run ends as an abort.
  cliff: { n: 128, tw: 2, sigma: 1, max: 200, beta: 0, rate: '1',
    program: [{ from: 5, to: 9999, beta: 30 }] },
};

function applyPreset(name) {
  const p = PRESETS[name];
  S.smr.program = p ? p.program : null;
  if (!p) return;
  $('smrN').value = p.n;
  $('smrTw').value = p.tw;
  $('smrSigma').value = p.sigma;
  $('smrMax').value = p.max;
  $('smrBeta').value = p.beta;
  // Load is not decoration here: an idle system has no backlog to re-age and
  // no batches to drain, so the queue and blocks panels stay blank without it.
  $('smrArrRate').value = p.rate;
  $('smrArrRate').dispatchEvent(new Event('change'));
}
for (const btn of $('smrPresetToggle').children) {
  btn.addEventListener('click', () => {
    for (const b of $('smrPresetToggle').children) b.classList.toggle('on', b === btn);
    applyPreset(btn.dataset.preset);
  });
}

// The program owns the slider only until the user grabs it.
function programBeta(round) {
  const prog = S.smr.program;
  if (!prog) return null;
  const seg = prog.find(s => round >= s.from && round < s.to);
  return seg ? seg.beta : 0;
}
function clearProgram() {
  S.smr.program = null;
  for (const b of $('smrPresetToggle').children) b.classList.toggle('on', b.dataset.preset === 'custom');
}

/* ---------- batch + live wiring ---------- */

function resetFailureOutcome() {
  S.smr.scenario = null;
  S.smr.resultContext = null;
  $('smrAbort').classList.add('hidden');
}

$('smrRunBtn').addEventListener('click', () => {
  if (S.engine !== 'wasm') { $('smrStatus').textContent = 'engine not ready'; return; }
  resetFailureOutcome();
  S.smr.live = null;
  $('smrCtl').classList.add('hidden');
  $('smrStatus').textContent = 'running…';
  post({ type: 'smr-run', spec: buildSmrSpec() });
});

$('smrLiveBtn').addEventListener('click', () => {
  if (S.engine !== 'wasm') { $('smrStatus').textContent = 'engine not ready'; return; }
  resetFailureOutcome();
  const sticky = $('smrLiveMode').value === 'sticky';
  $('smrStatus').textContent = 'starting live session…';
  post({ type: 'smr-live-new', spec: buildSmrSpec(), sticky });
});

export function onSmrDone(m) {
  S.smr.report = JSON.parse(m.json).report;
  S.smr.scenario = null;
  S.smr.resultContext = null;
  S.smr.lastStatus = null;
  $('smrStatus').textContent = `done: ${S.smr.report.metrics.length} rounds in ${m.ms} ms`;
  sessionStarted();
  renderSmr();
}

export function onSmrFailed(m) {
  const report = JSON.parse(m.report);
  const scenario = JSON.parse(m.scenario);
  const terminal = report.terminal.Failed;
  if (S.smr.live) {
    S.smr.live.playing = false;
    S.smr.live.awaiting = false;
    S.smr.live.runUntil = null;
  }
  S.smr.live = null;
  S.smr.report = report;
  S.smr.scenario = scenario;
  S.smr.resultContext = {
    proto: scenario.proto.kind,
    tw: scenario.proto.kind === 'recovery' ? scenario.proto.t_window_rounds : null,
    certsRec: scenario.proto.kind === 'recovery' && !!scenario.proto.resend_until_acked,
    n: scenario.n,
  };
  S.smr.lastStatus = m.status ? JSON.parse(m.status) : null;
  S.smr.detail = null;
  S.smr.pin = null;
  S.smr.chain = null;
  S.smr.manualBlocks = new Set();
  resetCertsDrawer();
  $('smrCtl').classList.add('hidden');
  updateSmrPlayBtn();
  sessionStarted(scenarioChipText(scenario));
  $('smrStatus').textContent = `FAILED: attempted ${terminal.attempted_round} · completed ${terminal.completed_round} · observed ${terminal.observed_round}`;
  $('smrAbort').innerHTML =
    `<b>FAILED</b> — recovery stopped before mutating boundary ${terminal.attempted_round}.` +
    `<span class="abortWhy">Completed through r${terminal.completed_round}; valid pre-boundary observations include r${terminal.observed_round}.</span>` +
    `<span class="abortWhy">Node ${m.failure.node}: prefix ${m.failure.violation.pre_len}, log ${m.failure.violation.log_len}, first mismatch ${m.failure.violation.first_mismatch}. The report and replay are frozen and exportable.</span>`;
  $('smrAbort').classList.remove('hidden');
  renderSmr();
}

export function onSmrReady(m) {
  S.smr.resultContext = null;
  $('smrAbort').classList.add('hidden');
  $('smrSlider').max = S.smr.proto === 'recovery' ? 100 : 60;
  S.smr.detail = null;
  S.smr.pin = null;
  S.smr.manualBlocks = new Set();
  S.smr.report = JSON.parse(m.report);
  S.smr.live = { playing: false, awaiting: false, runUntil: null };
  S.smr.lastStatus = null;
  S.smr.sel = null;
  sessionStarted();
  $('smrCtl').classList.remove('hidden');
  $('smrLiveRound').textContent = 'r 0';
  $('smrStatus').textContent = 'live session — β slider, inject commands anytime';
  updateSmrPlayBtn();
  // a previous session's rule may have fed the other chain feed
  resetCertsDrawer();
  S.smr.chain = null;
  onChainOrCerts(m);
  renderSmr();
}

// The chain panel has three feeds: compact streams the §5 certificate
// snapshot whole, recovery-with-certificates streams the same shape over the
// checkpoint forests (RQ11), and plain recovery streams committed-entry
// deltas that accumulate here.
function onChainOrCerts(m) {
  if (!isRec()) { onCertsSnapshot(m.certs); return; }
  if (displayCertsRec()) { onCertsSnapshot(m.certs, true); return; }
  if (!m.chain) return;
  if (m.chain.error) { freezeChain(m.chain.error); return; }
  const prev = S.smr.chain;
  S.smr.chain = {
    m: m.chain.m,
    peaks: m.chain.peaks,
    merges: m.chain.merges,
    entries: prev ? prev.entries.concat(m.chain.entries) : m.chain.entries,
  };
  onChainSnapshot(S.smr.chain);
}

// A committed block names a command; pinning it is the same act as clicking
// its button in the batch drawer.
useEntrySelect(ent => {
  const rep = S.smr.report;
  if (!rep || ent.kind !== 'cmd') return;
  const i = rep.commands.findIndex(c => c.client === ent.client && c.op === ent.op);
  if (i >= 0) selectCmd(i);
});

function scheduleSmrStep(session, delay) {
  setTimeout(() => {
    if (S.smr.live === session) smrStep();
  }, delay);
}

function smrStep() {
  const lv = S.smr.live;
  if (!lv || lv.awaiting) return;
  const programmed = programBeta((S.smr.lastStatus ? S.smr.lastStatus.round : 0) + 1);
  if (programmed !== null) {
    $('smrSlider').value = Math.min(programmed, +$('smrSlider').max);
    $('smrSliderVal').textContent = 'β ' + $('smrSlider').value + '%';
  }
  lv.awaiting = true;
  post({ type: 'smr-live-step', fraction: (+$('smrSlider').value) / 100 });
}
// arrival traffic is engine-side: the live pmf edit lands as a phase from the
// next round and rides in the exported spec (replays byte-identically)
$('smrTrafficApply').addEventListener('click', () => {
  if (!S.smr.live) { $('smrStatus').textContent = 'start a live session first'; return; }
  post({ type: 'smr-live-traffic', pmf: rateToPmf($('smrTraffic').value) });
  $('smrTrafficPopBtn').textContent = 'traffic: ' + $('smrTraffic').value + '/r';
});
export function onSmrTrafficSet(m) {
  const out = JSON.parse(m.json);
  $('smrStatus').textContent = out.error ? 'traffic: ' + out.error : 'arrival pmf applied — takes effect next round';
}
export function onSmrRound(m) {
  const lv = S.smr.live;
  if (!lv) return;
  lv.awaiting = false;
  const status = JSON.parse(m.status);
  S.smr.lastStatus = status;
  S.smr.report = JSON.parse(m.report);
  $('smrLiveRound').textContent = 'r ' + status.round;
  if (status.absorbed || status.dead !== null) lv.playing = false;
  updateSmrPlayBtn();
  onChainOrCerts(m);
  if (S.smr.pin !== null && isRec()) post({ type: 'smr-node-detail', node: S.smr.pin, pinned: true });
  renderSmr();
  if (lv.runUntil) {
    lv.runSteps++;
    const hit = lv.runUntil(status, S.smr.report);
    const done = hit === true || (typeof hit === 'string' && hit);
    const stuck = status.absorbed || status.dead !== null || lv.runSteps >= 2000;
    if (done || stuck) {
      $('smrStatus').textContent = done
        ? `stopped at r${status.round} — ${typeof hit === 'string' ? hit : lv.runLabel}`
        : `stopped at r${status.round} — no ${lv.runLabel} within ${lv.runSteps} rounds`;
      lv.runUntil = null;
    } else {
      scheduleSmrStep(lv, 0);
      return;
    }
  }
  if (lv.playing) scheduleSmrStep(lv, 720 / (+$('smrSpeed').value || 1));
}
function updateSmrPlayBtn() {
  $('smrPlayBtn').textContent = S.smr.live && S.smr.live.playing ? '⏸' : '▶️';
}
$('smrPlayBtn').addEventListener('click', () => {
  if (!S.smr.live) return;
  S.smr.live.playing = !S.smr.live.playing;
  updateSmrPlayBtn();
  if (S.smr.live.playing) smrStep();
});
$('smrStepBtn').addEventListener('click', () => {
  if (!S.smr.live) return;
  S.smr.live.playing = false;
  updateSmrPlayBtn();
  smrStep();
});
$('smrSlider').addEventListener('input', () => {
  $('smrSliderVal').textContent = 'β ' + $('smrSlider').value + '%';
  clearProgram();
});
$('smrSpeed').addEventListener('change', () => {});
$('smrAuditScope').addEventListener('change', () => {
  if (S.smr.report) renderAudit(S.smr.report);
});
$('smrEndBtn').addEventListener('click', () => {
  S.smr.live = null;
  S.smr.chain = null;
  sessionEnded();
  resetCertsDrawer();
});

/* ---------- interactive injection ---------- */

$('smrInjectBtn').addEventListener('click', () => {
  if (!S.smr.live) { $('smrStatus').textContent = 'start a live session first'; return; }
  const client = Math.max(0, Math.round(+$('smrInjClient').value || 0));
  const op = Math.max(1, Math.round(+$('smrInjOp').value || 1));
  const t = $('smrInjTarget').value;
  post({ type: 'smr-live-inject', client, op, target: t === '' ? null : Math.max(0, Math.round(+t)) });
});
export function onSmrInjected(m) {
  const out = JSON.parse(m.json);
  onChainOrCerts(m);
  if (m.quiet) return;
  if (out.error) { $('smrStatus').textContent = 'inject: ' + out.error; return; }
  $('smrStatus').textContent = `command accepted — lands round ${out.round}`;
  $('smrInjClient').value = +$('smrInjClient').value + 1;
  $('smrInjOp').value = +$('smrInjOp').value + 1;
  if (S.smr.live && !S.smr.live.playing) smrStep();
}

$('certVerifyBtn').addEventListener('click', () => {
  if (!S.smr.live) { $('smrStatus').textContent = 'start a live session first'; return; }
  post({ type: 'smr-verify-all' });
});
export function onSmrCaptured(m) {
  const out = JSON.parse(m.json);
  if (out.error) { $('smrStatus').textContent = 'capture: ' + out.error; return; }
  $('smrStatus').textContent = `stale cert captured — sn ${out.sn} frozen; two more commits from that client evict it`;
  onCertsSnapshot(m.certs);
}
export function onSmrVerify(m) {
  onVerifyTallies(m.json);
}

function exportSpec() {
  if (S.smr.live) { post({ type: 'smr-live-export' }); return; }
  const spec = S.smr.scenario || buildSmrSpec();
  downloadJson('smr-scenario.json', JSON.stringify(spec, null, 2));
}
$('smrExportSpec').addEventListener('click', exportSpec);
$('smrExportSpecLive').addEventListener('click', exportSpec);
$('smrExportReport').addEventListener('click', () => {
  if (S.smr.report) downloadJson('smr-report.json', JSON.stringify(S.smr.report, null, 2));
});
export function onSmrScenario(m) { downloadJson('smr-live-scenario.json', m.json); }

/* ---------- abort as an outcome ---------- */

// A violated oracle is a result, not a crash: the session stops but its
// history stays on screen with the engine's own message.
export function onSmrAbort(m) {
  S.smr.live = null;
  updateSmrPlayBtn();
  $('smrAbort').innerHTML =
    `<b>ABORTED</b> — the engine refused to continue.` +
    `<span class="abortWhy">${escapeHtml(m.panic || m.error)}</span>` +
    `<span class="abortWhy">The run up to here is still on screen and exportable. ` +
    `Start a new session to continue.</span>`;
  $('smrAbort').classList.remove('hidden');
}
function escapeHtml(s) {
  return String(s).replace(/[&<>]/g, c => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;' })[c]);
}

/* ---------- node drilldown + targeted blocking ---------- */

export function onSmrNodeDetail(m) {
  const d = JSON.parse(m.json);
  S.smr.detail = d.error ? null : d;
  renderSmr();
}
export function onSmrBlocked(m) {
  const out = JSON.parse(m.json);
  if (out.error) { $('smrStatus').textContent = 'block: ' + out.error; return; }
  if (m.blocked) S.smr.manualBlocks.add(m.node); else S.smr.manualBlocks.delete(m.node);
  $('smrStatus').textContent = `node ${m.node} ${m.blocked ? 'blocked' : 'released'} from round ${out.round}`;
  renderSmr();
}

/* ---------- recovery clock, phase, law ---------- */

const displayProto = () => S.smr.resultContext?.proto ?? S.smr.proto;
const displayTw = () => S.smr.resultContext?.tw ?? S.smr.tw;
const displayCertsRec = () => S.smr.resultContext?.certsRec ?? S.smr.certsRec;
const displayN = () => S.smr.resultContext?.n ?? 0;
const isRec = () => displayProto() === 'recovery' && !!displayTw();
const recRows = rep => (rep.recovery ? rep.recovery.rounds : null);

// The same scan the engine's SmrReport::recovered_round performs, twinned in
// crates/sim-wasm/tests/smr_recovery_live.rs — any change to this predicate
// must change that test too.
function rStarScan(rep, n, T) {
  if (rep.terminal?.Failed) return null;
  const rows = recRows(rep);
  if (!rows) return null;
  const ms = rep.metrics;
  const floor = Math.ceil(0.75 * n);
  const bs = [];
  for (let r = T; r <= ms.length; r += T) bs.push(r);
  const clean = b => rows[b - 1].reset === 0 && ms[b - 1].nonbot_logs >= floor;
  for (const rStar of bs) {
    const m = ms[rStar - 1];
    if (m.min_executed_len !== m.max_executed_len) continue;
    if (bs.every(b => b < rStar || clean(b))) return rStar;
  }
  return null;
}

// Release = the first round β fell back to 0 after being positive. Read off
// the recorded blocked counts, so it is whatever actually happened — slider,
// preset program or spec schedule alike.
function releaseRound(rep) {
  const ms = rep.metrics;
  let surged = false;
  for (let i = 0; i < ms.length; i++) {
    if (ms[i].blocked > 0) { surged = true; continue; }
    if (surged) return i + 1;
  }
  return null;
}

function renderRecChips(rep) {
  const on = isRec();
  $('smrRecChips').classList.toggle('hidden', !on);
  $('smrGridTabs').classList.toggle('hidden', !on);
  document.querySelectorAll('#smrBoundaryBtn, #smrEventBtn').forEach(b => b.classList.toggle('hidden', !on));
  if (!on) { $('smrLawCard').classList.add('hidden'); return; }
  const T = displayTw();
  const rows = recRows(rep);
  const round = rep.metrics.length;
  if (!rows || !round) return;
  const w = rows[round - 1].window;
  const nextAt = (Math.floor((round - 1) / T) + 1) * T;
  const n = S.smr.lastStatus ? S.smr.lastStatus.nodes.length : displayN();
  const failed = !!rep.terminal?.Failed;
  const release = releaseRound(rep);
  const rStar = failed || release === null ? null : rStarScan(rep, n, T);

  let clock = `W ${w} [${w * T + 1},${(w + 1) * T + 1}) · T ${T} · next in ${nextAt - round}`;
  if (release !== null && rStar !== null) clock += ` · r* ${rStar} · R ${rStar - release}`;
  $('smrClockChip').textContent = clock;

  // Honest gating: r*/law only exist once a release was observed, and only
  // harden to CONFIRMED once the scan finds a clean converged suffix.
  const phase = failed || release === null ? null : rStar === null ? 'PROVISIONAL' : 'CONFIRMED';
  $('smrPhaseChip').classList.toggle('hidden', phase === null);
  $('smrPhaseChip').textContent = phase === null ? '' : `${phase} · release r${release}`;
  $('smrPhaseChip').className = 'recChip mono' + (phase === 'CONFIRMED' ? ' ok' : phase ? ' warn' : '');

  const forkK = lastLatched(rows, 'cp_fork_k');
  $('smrForkChip').textContent = `cp-fork k ${forkK || '–'}`;
  $('smrForkChip').className = 'recChip mono' + (forkK > 1 ? ' alarm' : '');

  const settled = rep.commands.filter(c => c.all_logs_round !== null).length;
  const ckpt = lastLatched(rows, 'max_cp_p_len');
  const fin = rep.commands.filter(c => c.executed_round != null).length;
  const rolls = rows.reduce((a, r) => a + r.rollbacks, 0);
  $('smrLifeChip').textContent = `settled ${settled} → ckpt ${ckpt} → final ${fin} · ⟲ ${rolls}`;

  renderLawCard(release, rStar, T);
}

// Latched fields land on the row after a boundary and are 0 elsewhere; the
// chip should keep showing the newest one, not blank between boundaries.
function lastLatched(rows, key) {
  for (let i = rows.length - 1; i >= 0; i--) if (rows[i][key] !== 0) return rows[i][key];
  return 0;
}

function renderLawCard(release, rStar, T) {
  const card = $('smrLawCard');
  if (release === null || rStar === null) { card.classList.add('hidden'); return; }
  const phi = release % T;
  const predicted = 2 * T + ((T - phi) % T);
  const actual = rStar - release;
  card.innerHTML =
    `<b>Recovery took ${actual} rounds</b> — released at r${release}, recovered at r${rStar}.` +
    `<div class="lawEq">R = 2T + (T − φ) mod T = 2·${T} + (${T} − ${phi}) mod ${T} = ${predicted}` +
    (actual === predicted ? ' ✓' : ` · measured ${actual}`) + `</div>` +
    (actual === predicted ? '' :
      `<div>The law describes a FULL blackout released inside a window, which costs one ` +
      `window to re-arm every server to reset before a boundary can roll it back. This run ` +
      `skipped that window — either the surge left useful servers alive, or the release ` +
      `landed exactly on a boundary, where the re-arm comes free.</div>`) +
    `<div>φ = ${phi} is the release round's phase in its window: the two windows are the re-arm and the rollback, ` +
    `and (T − φ) mod T is the wait for the first boundary after release.</div>`;
  card.classList.remove('hidden');
}

/* ---------- committed sequence: one block per boundary ---------- */

// Nothing commits between boundaries, so a batch is the jump in executed
// length across consecutive post-boundary rows — row mT+1 against (m−1)T+1.
function renderBlocks(rep) {
  const strip = $('smrBlocks');
  const rows = recRows(rep);
  if (!rows) { strip.innerHTML = ''; return; }
  const T = displayTw(), ms = rep.metrics;
  const at = r => (r <= ms.length ? ms[r - 1].max_executed_len : null);
  const batches = [];
  let rolledBack = false;
  for (let m = 1; (m - 1) * T + 1 <= ms.length; m++) {
    const here = at(m * T + 1);
    if (here === null) break;
    const size = Math.max(0, here - at((m - 1) * T + 1));
    // DRAIN = the first batch that lands after a rollback: the catch-up.
    const drain = rolledBack && size > 0;
    if (drain) rolledBack = false;
    const rb = rows[m * T] ? rows[m * T].rollbacks : 0;
    if (rb > 0) rolledBack = true;
    batches.push({ m, boundary: m * T, size, drain, rb });
  }
  if (S.smr.batchSel !== null && !batches.some(b => b.m === S.smr.batchSel && b.size > 0))
    S.smr.batchSel = null;
  strip.innerHTML = batches.map(b => {
    if (b.size === 0) {
      const rolled = b.rb > 0;
      const title = rolled
        ? `boundary ${b.boundary}: rolled back ×${b.rb} — speculative entries discarded, clients re-send`
        : `boundary ${b.boundary}: nothing had aged a full clean window — no commit`;
      return `<div class="blk empty${rolled ? ' rolled' : ''}" title="${title}">` +
        `<span class="blkB">b${b.boundary}</span><span class="blkS">${rolled ? '⟲' : '0'}</span></div>`;
    }
    const on = S.smr.batchSel === b.m ? ' on' : '';
    return `<div class="blk${b.drain ? ' drain' : ''}${on}" data-m="${b.m}" ` +
      `style="width:${Math.min(140, 34 + b.size * 10)}px" ` +
      `title="boundary ${b.boundary}: ${b.size} entr${b.size === 1 ? 'y' : 'ies'} committed — click for the batch">` +
      `<span class="blkB">b${b.boundary}${b.rb > 0 ? ' ⟲' : ''}</span>` +
      `<span class="blkS">${b.size}${b.drain ? '<span class="blkTag"> DRAIN</span>' : ''}</span></div>`;
  }).join('') +
    `<div class="blk next"><span class="blkB">b${(batches.length + 1) * T}</span><span class="blkS">next</span></div>`;
  const total = batches.reduce((a, b) => a + b.size, 0);
  const rolled = batches.filter(b => b.size === 0 && b.rb > 0).length;
  const bare = batches.filter(b => b.size === 0 && b.rb === 0).length;
  $('smrBlocksNote').textContent = `${batches.length} boundaries · ${total} entries committed` +
    (rolled ? ` · ${rolled} ⟲ rolled back` : '') + (bare ? ` · ${bare} aged nothing` : '');
  renderBatchDrawer(rep, batches, total);
}

// The explorer half: which commands landed in the selected batch. Membership
// is exact — executed_round latches at the boundary pass, so it equals mT.
function renderBatchDrawer(rep, batches, total) {
  const box = $('smrBatchDrawer');
  const m = S.smr.batchSel;
  if (m === null) {
    if (total === 0 && batches.length > 0) {
      box.innerHTML = '<span class="bdDim">nothing committed yet — a boundary commits only entries ' +
        'that survived a full clean window; each rollback discards them and their resends re-age from zero</span>';
      box.classList.remove('hidden');
    } else box.classList.add('hidden');
    return;
  }
  const T = displayTw();
  const batch = batches.find(b => b.m === m);
  const cmds = rep.commands.map((c, i) => ({ c, i })).filter(x => x.c.executed_round === m * T);
  box.innerHTML =
    `<span class="bdHead mono">b${m * T} · ${batch.size} entr${batch.size === 1 ? 'y' : 'ies'}` +
    (cmds.length !== batch.size ? ` · ${cmds.length} client command${cmds.length === 1 ? '' : 's'}` : '') +
    `</span>` +
    (cmds.length
      ? cmds.map(x =>
          `<button class="bdCmd mono${S.smr.sel === x.i ? ' on' : ''}" data-i="${x.i}" ` +
          `style="color:${cmdColor(x.i)};border-color:${cmdColor(x.i)}44" ` +
          `title="injected r${x.c.injection_round} — click to pin on the charts">${cmdLabel(x.c)}</button>`).join('')
      : '<span class="bdDim">structural entries only — no client commands in this batch</span>');
  box.classList.remove('hidden');
}

/* ---------- steppers ---------- */

// One predicate drives the existing single-flight loop; onSmrRound checks it
// after the state update and either stops or queues the next step.
function runUntil(pred, label) {
  const lv = S.smr.live;
  if (!lv) { $('smrStatus').textContent = 'start a live session first'; return; }
  lv.playing = false;
  lv.runUntil = pred;
  lv.runLabel = label;
  lv.runSteps = 0;
  updateSmrPlayBtn();
  smrStep();
}

// Rows are PRE-boundary: a boundary's effects land on the row after it, so
// that is where the stepper stops.
const boundaryPred = st => st.round > 1 && (st.round - 1) % displayTw() === 0;

function eventPred(st, rep) {
  const rows = recRows(rep);
  if (!rows) return null;
  const row = rows[st.round - 1];
  const prev = st.round > 1 ? rows[st.round - 2] : null;
  if (row.rollbacks > 0) return `rollback ×${row.rollbacks} (depth ${row.rollback_depth})`;
  if (row.bot_r > 0 && (!prev || prev.bot_r === 0)) return 'first ⊥';
  if (row.cp_fork_k > 1) return `cp-fork k=${row.cp_fork_k}`;
  const release = releaseRound(rep);
  const n = st.nodes.length;
  if (release !== null && rStarScan(rep, n, displayTw()) === st.round) return 'recovery CONFIRMED';
  return null;
}

$('smrBoundaryBtn').addEventListener('click', () => runUntil(boundaryPred, 'boundary'));
$('smrEventBtn').addEventListener('click', () => runUntil(eventPred, 'event'));

$('smrBlocks').addEventListener('click', e => {
  const el = e.target.closest('.blk');
  if (!el || el.dataset.m === undefined) return;
  const m = +el.dataset.m;
  S.smr.batchSel = S.smr.batchSel === m ? null : m;
  renderSmr();
});
$('smrBatchDrawer').addEventListener('click', e => {
  const el = e.target.closest('.bdCmd');
  if (el) selectCmd(+el.dataset.i);
});

/* ---------- rendering ---------- */

const cmdColor = i => PALETTE[i % PALETTE.length];
const cmdLabel = cmd => cmd.auto
  ? `bg #${cmd.client - AUTO_CLIENT_BASE}`
  : `c${cmd.client} · op ${cmd.op}`;
const BADGE = { Complete: ['badge-ok', 'Complete'], Pending: ['badge-run', 'Pending'], Dead: ['badge-dead', 'Dead'] };
const OC = {
  Delivered: ['oc-delivered', 'delivered'],
  TargetBlocked: ['oc-blocked', 'target-blocked'],
  TargetBot: ['oc-bot', 'target-⊥'],
  Amplified: ['oc-amplified', 'amplified'],
  AckCommitted: ['oc-ack', 'ack-committed'],
  Ignored: ['oc-ignored', 'ignored'],
};

function renderSmr() {
  const rep = S.smr.report;
  if (!rep) return;
  // In recovery the two standalone panels ARE timeline lanes 5 and 6; showing
  // both would duplicate the same curve on two different axes.
  const rec = isRec();
  $('smrSpreadBox').classList.toggle('hidden', rec);
  $('smrPrefixBox').classList.toggle('hidden', rec);
  $('smrTimelineBox').classList.toggle('hidden', !rec);
  $('smrBlocksBox').classList.toggle('hidden', !rec);
  $('smrGhostRow').classList.toggle('hidden', !rec);
  if (rec) renderBlocks(rep);
  drawSmrNet(rep);
  if (rec) drawTimeline(rep); else { drawSpread(rep); drawPrefix(rep); }
  renderMetrics(rep);
  renderRecChips(rep);
  renderAudit(rep);
}

/* selected command: user pick, else the latest injected one */
function selectedCmd(rep) {
  if (S.smr.sel !== null && S.smr.sel < rep.commands.length) return S.smr.sel;
  return rep.commands.length ? rep.commands.length - 1 : null;
}
function selectCmd(i) {
  S.smr.sel = i;
  renderSmr();
}

function prepCanvas(cv) {
  const dpr = window.devicePixelRatio || 1;
  const W = cv.clientWidth, H = cv.clientHeight;
  if (W < 2 || H < 2) return null;
  cv.width = W * dpr; cv.height = H * dpr;
  const ctx = cv.getContext('2d');
  ctx.scale(dpr, dpr);
  ctx.clearRect(0, 0, W, H);
  return { ctx, W, H };
}

function drawSmrNet(rep) {
  const box = $('smrNetBox');
  const st = S.smr.lastStatus;
  const live = !!S.smr.live;
  box.classList.toggle('hidden', !live);
  if (!live || !st) return;
  const sel = selectedCmd(rep);
  const P = prepCanvas($('smrNet'));
  if (!P) return;
  const { ctx, W, H } = P;
  const n = st.nodes.length;
  const cols = Math.max(1, Math.ceil(Math.sqrt(n * W / Math.max(1, H))));
  const rows = Math.ceil(n / cols);
  const cw = W / cols, ch = H / rows;
  const rad = Math.max(1.5, Math.min(cw, ch) * 0.32);
  const blocked = new Set(st.blocked);
  let holders = null;
  if (sel !== null) {
    const sp = st.spreading.find(c => c.client === rep.commands[sel].client);
    if (sp) holders = new Set(sp.holders);
    else if (rep.commands[sel].all_logs_round !== null) holders = 'all';
    else holders = new Set();
  }
  const col = sel !== null ? cmdColor(sel) : '#3dd6c4';
  const rstate = isRec() && S.smr.grid === 'rstate';
  const at = i => ({ x: (i % cols) * cw + cw / 2, y: Math.floor(i / cols) * ch + ch / 2 });
  S.smr.gridGeom = { cols, cw, ch, rad, n };

  // Adoption and reset-vote edges fade over the next couple of rounds so a
  // paused stepper still shows what the last round's replies caused.
  if (rstate && st.rec) {
    const edges = [...st.rec.adoptions.map(e => [e, '#3dd6c4']),
                   ...st.rec.reset_votes.map(e => [e, '#e0af68'])];
    ctx.lineWidth = 1;
    for (const [[from, to], ecol] of edges) {
      if (from >= n || to >= n) continue;
      const a = at(from), b = at(to);
      ctx.strokeStyle = ecol;
      ctx.globalAlpha = 0.5;
      ctx.beginPath(); ctx.moveTo(a.x, a.y); ctx.lineTo(b.x, b.y); ctx.stroke();
    }
    ctx.globalAlpha = 1;
  }

  for (let i = 0; i < n; i++) {
    const { x, y } = at(i);
    const g = st.nodes[i];
    const bot = g.log_len === null;
    const holds = !bot && holders !== null && (holders === 'all' || holders.has(i));
    ctx.beginPath();
    ctx.arc(x, y, rad, 0, Math.PI * 2);
    // R-state palette mirrors the engine's r_code: 0 no-reset · 1 reset · 2 ⊥
    ctx.fillStyle = rstate
      ? (g.r === 0 ? '#3dd6c4' : g.r === 1 ? '#e0af68' : '#4b5563')
      : bot ? '#4b5563' : holds ? col : '#252c36';
    ctx.fill();
    if (blocked.has(i)) {
      ctx.beginPath();
      ctx.arc(x, y, rad + 1.5, 0, Math.PI * 2);
      ctx.strokeStyle = '#f7768e';
      ctx.lineWidth = 1;
      ctx.stroke();
    }
    // Dashed ring = blocked by hand, so an operator can tell their own
    // intervention from the schedule's sample.
    if (S.smr.manualBlocks.has(i)) {
      ctx.beginPath();
      ctx.arc(x, y, rad + 3, 0, Math.PI * 2);
      ctx.strokeStyle = '#f7768e'; ctx.lineWidth = 1; ctx.setLineDash([2, 2]);
      ctx.stroke(); ctx.setLineDash([]);
    }
    if (S.smr.pin === i) {
      ctx.beginPath();
      ctx.arc(x, y, rad + 4.5, 0, Math.PI * 2);
      ctx.strokeStyle = '#cfd6df'; ctx.lineWidth = 1.4; ctx.stroke();
    }
  }
  const cmd = sel !== null ? rep.commands[sel] : null;
  if (rstate) {
    const c = st.nodes.reduce((a, g) => (a[g.r]++, a), [0, 0, 0]);
    $('smrNetNote').textContent = `no-reset ${c[0]} · reset ${c[1]} · ⊥ ${c[2]}`;
  } else {
    $('smrNetNote').textContent = cmd
      ? `${cmdLabel(cmd)} · ${holders === 'all' ? 'all non-⊥' : holders.size + '/' + n}`
      : 'inject to watch';
  }
  renderDrill();
}

/* ---------- node drilldown ---------- */

for (const b of document.querySelectorAll('#smrGridTabs .ct')) {
  b.addEventListener('click', () => {
    S.smr.grid = b.dataset.grid;
    for (const x of document.querySelectorAll('#smrGridTabs .ct')) x.classList.toggle('on', x === b);
    if (S.smr.report) renderSmr();
  });
}

$('smrNet').addEventListener('click', ev => {
  const geom = S.smr.gridGeom;
  if (!geom || !S.smr.live || !isRec()) return;
  const r = $('smrNet').getBoundingClientRect();
  const col = Math.floor((ev.clientX - r.left) / geom.cw);
  const row = Math.floor((ev.clientY - r.top) / geom.ch);
  const node = row * geom.cols + col;
  if (node < 0 || node >= geom.n) return;
  S.smr.pin = S.smr.pin === node ? null : node;
  S.smr.detail = null;
  if (S.smr.pin === null) renderSmr(); else post({ type: 'smr-node-detail', node });
});

function renderDrill() {
  const panel = $('smrDrill');
  const pin = S.smr.pin;
  if (!isRec() || pin === null) { panel.classList.add('hidden'); return; }
  panel.classList.remove('hidden');
  const d = S.smr.detail;
  if (!d) { panel.innerHTML = `<div class="drillHead"><b>node ${pin}</b><span>loading…</span></div>`; return; }
  const R = ['no-reset', 'reset', '⊥'][d.r] || '?';
  const diverged = d.s_hash !== d.checkpoint_s_hash;
  const blocked = S.smr.manualBlocks.has(pin);
  panel.innerHTML =
    `<div class="drillHead"><b>node ${d.node}</b><span>R ${R}</span>` +
    `<span style="flex:1"></span>` +
    `<button class="drillBtn blk${blocked ? ' on' : ''}" id="drillBlock">⌖ ${blocked ? 'release' : 'block'}</button>` +
    `<button class="drillBtn on" id="drillUnpin">unpin</button></div>` +
    row('log |L|', d.log_len === null ? '⊥' : d.log_len) +
    row('executed', d.executed_len) +
    row('checkpoint', `W ${d.checkpoint_window} · |P| ${d.checkpoint_p_len === null ? '⊥' : d.checkpoint_p_len}`) +
    row('S hash', d.s_hash, diverged) +
    row('C.S hash', d.checkpoint_s_hash, diverged);
  panel.querySelector('#drillUnpin').addEventListener('click', () => {
    S.smr.pin = null; S.smr.detail = null; renderSmr();
  });
  panel.querySelector('#drillBlock').addEventListener('click', () => {
    post({ type: 'smr-live-block', node: pin, blocked: !blocked });
  });
}
function row(k, v, diverged = false) {
  return `<div class="drillRow${diverged ? ' diverged' : ''}"><span class="k">${k}</span><span class="v">${v}</span></div>`;
}

function gammaAt(cmd, round) {
  const p = cmd.spread.find(q => q.round === round) ||
    cmd.spread.filter(q => q.round <= round).at(-1);
  return p && p.useful_total ? p.useful_holders / p.useful_total : 0;
}

function chartFrame(ctx, W, H, rounds) {
  const padL = 44, padR = 18, padT = 16, padB = 28;
  const X = r => padL + (r / rounds) * (W - padL - padR);
  ctx.font = '10px ui-monospace, Menlo, monospace';
  ctx.textBaseline = 'alphabetic'; ctx.textAlign = 'center';
  const step = Math.max(1, Math.ceil(rounds / 6 / 5) * 5);
  for (let r = 0; r <= rounds; r += step) {
    ctx.strokeStyle = '#1c2129';
    ctx.beginPath(); ctx.moveTo(X(r), padT); ctx.lineTo(X(r), H - padB); ctx.stroke();
    ctx.fillStyle = '#565f6b'; ctx.fillText('r' + r, X(r), H - padB + 16);
  }
  // live round marker
  if (S.smr.live && S.smr.lastStatus) {
    const lx = X(S.smr.lastStatus.round);
    ctx.strokeStyle = '#e0af68'; ctx.lineWidth = 1; ctx.setLineDash([4, 4]);
    ctx.beginPath(); ctx.moveTo(lx, padT); ctx.lineTo(lx, H - padB); ctx.stroke();
    ctx.setLineDash([]);
    ctx.fillStyle = '#e0af68'; ctx.textAlign = 'left';
    ctx.fillText('r' + S.smr.lastStatus.round, Math.min(lx + 4, W - padR - 24), padT + 6);
  }
  return { padL, padR, padT, padB, X };
}

function drawSpread(rep) {
  const P = prepCanvas($('smrSpread'));
  if (!P) return;
  const { ctx, W, H } = P;
  const rounds = Math.max(1, rep.metrics.length);
  const { padL, padR, padT, padB, X } = chartFrame(ctx, W, H, rounds);
  if (S.smr.chart === 'position') { drawPosition(rep, ctx, W, H, X, padT, padB, padL); return; }
  const Y = g => (H - padB) - g * (H - padT - padB);
  const sel = selectedCmd(rep);

  ctx.font = '10px ui-monospace, Menlo, monospace';
  ctx.textBaseline = 'middle';
  for (const g of [0, 0.25, 0.5, 0.75, 1]) {
    ctx.strokeStyle = '#1c2129'; ctx.lineWidth = 1;
    ctx.beginPath(); ctx.moveTo(padL, Y(g)); ctx.lineTo(W - padR, Y(g)); ctx.stroke();
    if (g === 0 || g === 0.5 || g === 1) {
      ctx.fillStyle = '#565f6b'; ctx.textAlign = 'right';
      ctx.fillText(g.toFixed(1), padL - 8, Y(g));
    }
  }
  // γ = 1/3 window ceiling
  ctx.strokeStyle = '#2c3440'; ctx.setLineDash([3, 4]);
  ctx.beginPath(); ctx.moveTo(padL, Y(1 / 3)); ctx.lineTo(W - padR, Y(1 / 3)); ctx.stroke();
  ctx.setLineDash([]);
  ctx.textBaseline = 'alphabetic';
  ctx.save(); ctx.translate(12, (padT + H - padB) / 2); ctx.rotate(-Math.PI / 2);
  ctx.fillStyle = '#565f6b'; ctx.textAlign = 'center'; ctx.fillText('γ', 0, 0); ctx.restore();

  drawSpreadCurves(ctx, rep, X, Y, sel, {});
}

// Shared by the standalone spread panel and the timeline's γ lane, so both
// read the same curve. `thin` drops the landmark glyphs the lane has no room
// for; with no options it is the panel's original drawing exactly.
export function drawSpreadCurves(ctx, rep, X, Y, sel, opts) {
  // recent commands draw full-strength; older ones stay as faint ghosts
  // (capped) instead of silently vanishing — select via audit for full view
  const curveFrom = Math.max(0, rep.commands.length - 24);
  const ghostFrom = Math.max(0, curveFrom - 200);
  ctx.textAlign = 'center';
  rep.commands.forEach((cmd, i) => {
    if (!cmd.spread.length) return;
    if (i < ghostFrom && i !== sel) return;
    const ghost = i < curveFrom && i !== sel;
    const col = cmdColor(i);
    const active = sel === i;
    ctx.globalAlpha = ghost ? 0.14 : sel !== null && !active ? 0.3 : 1;
    ctx.strokeStyle = col;
    ctx.lineWidth = active ? 2.6 : 1.8;
    ctx.shadowColor = active ? col : 'transparent';
    ctx.shadowBlur = active ? 8 : 0;
    ctx.beginPath();
    cmd.spread.forEach((p, j) => {
      const y = Y(p.useful_total ? p.useful_holders / p.useful_total : 0);
      if (j === 0) ctx.moveTo(X(p.round), y); else ctx.lineTo(X(p.round), y);
    });
    ctx.stroke();
    ctx.shadowBlur = 0;
    if (ghost || opts.thin) { ctx.globalAlpha = 1; return; }

    ctx.font = 'bold 9px ui-monospace, Menlo, monospace';
    if (cmd.delivered_round !== null) {
      ctx.fillStyle = col;
      ctx.beginPath();
      ctx.arc(X(cmd.delivered_round), Y(gammaAt(cmd, cmd.delivered_round)), active ? 5 : 4, 0, Math.PI * 2);
      ctx.fill();
      ctx.strokeStyle = '#14181e'; ctx.lineWidth = 1.5; ctx.stroke();
    }
    const tick = (round, tcol, label) => {
      if (round === null) return;
      const x = X(round), y = Y(gammaAt(cmd, round));
      ctx.strokeStyle = tcol; ctx.lineWidth = active ? 2.2 : 1.6;
      ctx.beginPath(); ctx.moveTo(x, y - 7); ctx.lineTo(x, y + 7); ctx.stroke();
      ctx.fillStyle = tcol; ctx.fillText(label, x, y - 11);
    };
    tick(cmd.all_logs_round, '#7dcfff', 'B');
    tick(cmd.prefix_fixed_round, '#bb9af7', 'E');
    if (cmd.committed_ack_round !== null) {
      const x = X(cmd.committed_ack_round), y = Y(gammaAt(cmd, cmd.committed_ack_round));
      ctx.strokeStyle = '#5ec9bd'; ctx.lineWidth = 1.4;
      ctx.beginPath(); ctx.moveTo(x, y); ctx.lineTo(x, y - 14); ctx.stroke();
      ctx.fillStyle = '#5ec9bd';
      ctx.beginPath(); ctx.moveTo(x, y - 14); ctx.lineTo(x + 9, y - 11); ctx.lineTo(x, y - 8);
      ctx.closePath(); ctx.fill();
    }
    ctx.globalAlpha = 1;
  });
}

// position band: selected command's min/median/max log position per round
function drawPosition(rep, ctx, W, H, X, padT, padB, padL) {
  const sel = selectedCmd(rep);
  if (sel === null) return;
  const cmd = rep.commands[sel];
  const pts = cmd.spread.filter(p => p.pos_max !== undefined && p.pos_max !== null);
  ctx.font = '10px ui-monospace, Menlo, monospace';
  if (!pts.length) {
    ctx.fillStyle = '#565f6b'; ctx.textAlign = 'left';
    ctx.fillText('no position samples yet — command not in any log', padL + 8, padT + 14);
    return;
  }
  const maxPos = Math.max(...pts.map(p => p.pos_max), 1);
  const Y = pos => (H - padB) - (pos / maxPos) * (H - padT - padB - 8);
  ctx.textBaseline = 'middle'; ctx.textAlign = 'right'; ctx.fillStyle = '#565f6b';
  for (const f of [0, 0.5, 1]) {
    const pos = Math.round(maxPos * f);
    ctx.fillText(String(pos), padL - 8, Y(pos));
  }
  ctx.textBaseline = 'alphabetic';
  // band min..max
  ctx.fillStyle = 'rgba(61,214,196,0.18)';
  ctx.beginPath();
  pts.forEach((p, j) => { const y = Y(p.pos_max); j === 0 ? ctx.moveTo(X(p.round), y) : ctx.lineTo(X(p.round), y); });
  for (let j = pts.length - 1; j >= 0; j--) ctx.lineTo(X(pts[j].round), Y(pts[j].pos_min));
  ctx.closePath(); ctx.fill();
  // median line
  ctx.strokeStyle = '#3dd6c4'; ctx.lineWidth = 2.2;
  ctx.beginPath();
  pts.forEach((p, j) => { const y = Y(p.pos_med); j === 0 ? ctx.moveTo(X(p.round), y) : ctx.lineTo(X(p.round), y); });
  ctx.stroke();
  // T_E marker
  if (cmd.prefix_fixed_round !== null) {
    const x = X(cmd.prefix_fixed_round);
    ctx.strokeStyle = '#bb9af7'; ctx.lineWidth = 1.4;
    ctx.beginPath(); ctx.moveTo(x, H - padB); ctx.lineTo(x, H - padB + 6); ctx.stroke();
    ctx.fillStyle = '#bb9af7'; ctx.textAlign = 'center';
    ctx.fillText('T_E', x, H - 6);
  }
  ctx.fillStyle = '#6f7986'; ctx.textAlign = 'left';
  ctx.fillText(`${cmdLabel(cmd)} · position band (min/median/max)`, padL + 8, padT + 12);
}

// prefix ribbon: per-round agreed-prefix fraction
function drawPrefix(rep) {
  const P = prepCanvas($('smrPrefix'));
  if (!P) return;
  const { ctx, W, H } = P;
  const ms = rep.metrics;
  if (!ms.length) return;
  const bw = W / ms.length;
  ms.forEach((m, i) => {
    const denom = Math.max(m.max_log_len, m.max_executed_len, 1);
    const v = Math.max(0, Math.min(1, m.lcp_len / denom));
    ctx.fillStyle = v > 0.995 ? '#3dd6c4' : 'rgba(61,214,196,0.45)';
    const h = v * (H - 6);
    ctx.fillRect(i * bw + 0.5, H - 3 - h, Math.max(bw - 1, 0.75), h);
  });
  const last = ms[ms.length - 1];
  $('smrLcpStat').textContent = `lcp ${last.lcp_len} / max ${Math.max(last.max_log_len, last.max_executed_len)}`;
}

function renderMetrics(rep) {
  const m = rep.metrics[rep.metrics.length - 1];
  const failed = rep.terminal.Failed;
  const term = failed
    ? `FAILED attempted r${failed.attempted_round} · completed r${failed.completed_round} · observed r${failed.observed_round}`
    : rep.terminal.Dead ? `DEAD r${rep.terminal.Dead.round}` : `${rep.metrics.length} rounds`;
  const pending = rep.commands.filter(c => c.status === 'Pending').length;
  $('smrStrip').textContent = m
    ? `${term} · logs ${m.nonbot_logs} · useful ${m.useful} · max|L| ${m.max_log_len}` +
      ` · exec ${m.min_executed_len}–${m.max_executed_len}` +
      ` · in flight ${pending}/${rep.commands.length}` +
      (rep.safety_ok ? '' : ' · ⚠ SAFETY VIOLATION')
    : '–';
  $('smrCtlMetrics').innerHTML = m
    ? `logs <b>${m.nonbot_logs}</b> · useful <b>${m.useful}</b> · exec <b>${m.min_executed_len}–${m.max_executed_len}</b>` +
      (rep.safety_ok ? '' : ' · <b style="color:#f7768e">⚠ SAFETY</b>')
    : '';
  $('auditCnt').textContent = String(rep.commands.length);
}

function renderAudit(rep) {
  const box = $('smrAudit');
  box.innerHTML = '';
  if (!rep.commands.length) {
    box.innerHTML = '<div class="hint">no commands — add injections or submit one live</div>';
    return;
  }
  const sel = selectedCmd(rep);
  const scope = $('smrAuditScope').value;
  const cap = scope === 'all' ? rep.commands.length : +scope;
  const from = Math.max(0, rep.commands.length - cap);
  if (from > 0) {
    const done = rep.commands.slice(0, from).filter(c => c.status === 'Complete').length;
    const more = document.createElement('div');
    more.className = 'auditIntro';
    more.textContent = `${from} earlier commands (${done} complete) — widen the scope above or see the report export`;
    box.appendChild(more);
  }
  rep.commands.forEach((cmd, i) => {
    if (i < from && i !== sel) return;
    const b = BADGE[cmd.status];
    const card = document.createElement('div');
    card.className = 'auditCard' + (sel === i ? ' sel open' : rep.commands.length <= 3 ? ' open' : '');
    const tm = cmd.prefix_fixed_round !== null && cmd.all_logs_round !== null
      ? cmd.prefix_fixed_round - cmd.all_logs_round : null;
    const lms = [`<span class="lm">injected r${cmd.injection_round}</span>`];
    if (cmd.delivered_round !== null) lms.push(`<span class="lm">delivered r${cmd.delivered_round}</span>`);
    if (cmd.all_logs_round !== null) lms.push(`<span class="lm b">T_B r${cmd.all_logs_round}</span>`);
    if (cmd.prefix_fixed_round !== null)
      lms.push(`<span class="lm e">T_E r${cmd.prefix_fixed_round}${tm !== null ? ' · T_M ' + tm : ''}</span>`);
    if (cmd.committed_ack_round !== null) lms.push(`<span class="lm ack">ack r${cmd.committed_ack_round}</span>`);
    const attRow = a => {
      const oc = OC[a.outcome] || ['oc-ignored', a.outcome];
      return `<div class="att"><span class="r">r${a.round}</span><span class="nd">#${a.target}</span><span class="oc ${oc[0]}">${oc[1]}</span></div>`;
    };
    // runs of ≥3 consecutive ignored resends collapse to one row — the §5
    // resend-until-ack client would flood the trail otherwise
    const parts = [];
    for (let j = 0; j < cmd.attempts.length;) {
      let k = j;
      while (k < cmd.attempts.length && cmd.attempts[k].outcome === 'Ignored') k++;
      if (k - j >= 3) {
        const [cls, label] = OC.Ignored;
        parts.push(`<div class="att attRun"><span class="r">×${k - j}</span><span class="nd">r${cmd.attempts[j].round}–r${cmd.attempts[k - 1].round}</span><span class="oc ${cls}">${label}</span></div>`);
        j = k;
      } else {
        parts.push(attRow(cmd.attempts[j]));
        j++;
      }
    }
    const atts = parts.join('');
    card.innerHTML =
      `<div class="auditHead"><span class="sw" style="background:${cmdColor(i)}"></span>` +
      `<span class="auditLabel mono">${cmdLabel(cmd)}</span>` +
      `<span class="badge ${b[0]}">${b[1]}</span><span class="chev">▸</span></div>` +
      `<div class="auditBody">` +
      `<div class="lmRow">${lms.join('')}</div>` +
      `<div class="ampRow">amp-receivers: <b>${cmd.amp_receivers.length || '—'}</b></div>` +
      `<div class="attList"><div class="attCap">delivery attempts</div>${atts}</div>` +
      `</div>`;
    card.querySelector('.auditHead').addEventListener('click', () => {
      selectCmd(S.smr.sel === i && card.classList.contains('open') ? null : i);
    });
    box.appendChild(card);
  });
}

// one starter row so the form is runnable as-is
injRow(2);
window.addEventListener('resize', () => { if (S.smr.report && S.view === 'smr') renderSmr(); });
export { renderSmr };
useSpreadCurves(drawSpreadCurves);
