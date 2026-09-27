// run form: spec building, presets, scenario import/export
import { S, $, clamp01 } from './state.js';
import { post } from './engine.js';
import { loadTrace } from './trace.js';
import { looksLikeSpill, loadSpill } from './spill.js';
import { PMF } from './pmf-editor.js';

/* ---------- run form ---------- */

const VALUE_COUNT_CAP = { random: 4294967296, even: 200000 };
function syncFormRows() {
  const init = $('runInit').value;
  $('rowSplitFrac').classList.toggle('hidden', init !== 'split');
  $('rowRandK').classList.toggle('hidden', !(init === 'random' || init === 'even'));
  $('rowPmf').classList.toggle('hidden', init !== 'weighted');
  if (VALUE_COUNT_CAP[init]) {
    $('runRandK').max = VALUE_COUNT_CAP[init];
    $('runRandK').title = 'up to ' + VALUE_COUNT_CAP[init].toLocaleString('en-US') + ' values for this init';
  }
  if (init === 'weighted') PMF.draw();
}
$('runInit').addEventListener('change', syncFormRows);
$('runRandK').addEventListener('input', () => {
  const cap = VALUE_COUNT_CAP[$('runInit').value];
  if (cap && +$('runRandK').value > cap) $('runRandK').value = cap;
});

function baseInitSpec(kind) {
  if (kind === 'split') return { kind: 'split', fraction: clamp01(+$('runSplitFrac').value || 0) };
  if (kind === 'random') return { kind: 'uniform_random', k: Math.min(Math.max(1, Math.round(+$('runRandK').value || 4)), VALUE_COUNT_CAP.random) };
  if (kind === 'even') return { kind: 'even_split', values: Math.min(Math.max(1, Math.round(+$('runRandK').value || 4)), VALUE_COUNT_CAP.even) };
  if (kind === 'weighted') {
    const weights = PMF.weights();
    const spec = { kind: 'weighted', weights };
    const range = PMF.range();
    if (range > weights.length) spec.range = range;
    return spec;
  }
  return { kind: 'distinct' };
}
function buildInitSpec() {
  const base = baseInitSpec($('runInit').value);
  const useful = clamp01(+$('runUseful').value);
  if (useful < 1) {
    return { kind: 'with_undecided', useful_fraction: useful, inner: base };
  }
  return base;
}
export function buildSpec() {
  const n = Math.min(Math.max(+$('runN').value || 1000, 10), 200000);
  const beta = Math.min(Math.max(+$('runBeta').value || 0, 0), 60);
  const maxRounds = Math.min(Math.max(+$('runMax').value || 1000, 1), 5000);
  return {
    n,
    seed: +$('runSeed').value || 0,
    k: Math.round(+$('runK').value || 6),
    ell: Math.round(+$('runEll').value || 3),
    init: buildInitSpec(),
    max_rounds: maxRounds,
    schedule: { kind: $('runSchedule').value, fraction: beta / 100 },
  };
}
export function validateSpec(spec) {
  if (!(spec.k > 1 && spec.ell > 1 && spec.k >= spec.ell && spec.ell % 2 === 1)) {
    return 'config needs k, ℓ > 1, k ≥ ℓ, ℓ odd';
  }
  return null;
}
$('runBtn').addEventListener('click', () => {
  if (S.engine !== 'wasm') { $('runStatus').textContent = 'engine not ready'; return; }
  const spec = buildSpec();
  const problem = validateSpec(spec);
  if (problem) { $('runStatus').textContent = 'error: ' + problem; return; }
  $('runStatus').textContent = 'running…';
  post({ type: 'run', spec });
});

/* ---------- E2 presets (three-regime bracket) ---------- */
function applyPreset(p) {
  $('runInit').value = p.init;
  $('runUseful').value = p.useful !== undefined ? p.useful : 1;
  $('runSplitFrac').value = 0.5;
  $('runSchedule').value = p.schedule;
  $('runBeta').value = p.beta;
  $('runMax').value = p.maxRounds;
  syncFormRows();
  $('runStatus').textContent = p.label + ' preset loaded — press run';
}
$('presetA').addEventListener('click', () => applyPreset({
  label: 'A collapse', init: 'split', useful: 0.30,
  schedule: 'fresh_per_round', beta: 0, maxRounds: 200,
}));
$('presetB').addEventListener('click', () => applyPreset({
  label: 'B converge-hold', init: 'split', useful: 5 / 9,
  schedule: 'fresh_per_round', beta: 10, maxRounds: 500,
}));
$('presetC').addEventListener('click', () => applyPreset({
  label: 'C death spiral', init: 'split',
  schedule: 'permanent', beta: 30, maxRounds: 500,
}));

/* ---------- scenario import / export ---------- */
export function downloadJson(name, text) {
  const a = document.createElement('a');
  a.href = URL.createObjectURL(new Blob([text], { type: 'application/json' }));
  a.download = name;
  a.click();
  URL.revokeObjectURL(a.href);
}
$('exportSpecBtn').addEventListener('click', () => {
  downloadJson('scenario.json', JSON.stringify(buildSpec(), null, 2));
});

/* Fill the form when the spec is expressible; false = run it directly. */
function fillFormFromSpec(spec) {
  const sched = spec.schedule || { kind: 'fresh_per_round', fraction: 0 };
  if (sched.kind !== 'fresh_per_round' && sched.kind !== 'permanent') return false;
  const setBase = (init, prefix) => {
    if (init.kind === 'split') { $(prefix).value = 'split'; $('runSplitFrac').value = init.fraction; return true; }
    if (init.kind === 'uniform_random') { $(prefix).value = 'random'; $('runRandK').value = init.k; return true; }
    if (init.kind === 'distinct') { $(prefix).value = 'distinct'; return true; }
    if (init.kind === 'even_split') { $(prefix).value = 'even'; $('runRandK').value = init.values; return true; }
    if (init.kind === 'weighted') {
      $(prefix).value = 'weighted';
      $('pmfRange').value = init.range || init.weights.length;
      PMF.imported = init.weights;
      return true;
    }
    return false;
  };
  if (spec.init.kind === 'with_undecided') {
    $('runUseful').value = spec.init.useful_fraction;
    if (!setBase(spec.init.inner, 'runInit')) return false;
  } else {
    $('runUseful').value = 1;
    if (!setBase(spec.init, 'runInit')) return false;
  }
  $('runN').value = spec.n;
  $('runSeed').value = spec.seed;
  $('runK').value = spec.k;
  $('runEll').value = spec.ell;
  $('runMax').value = spec.max_rounds;
  $('runSchedule').value = sched.kind;
  $('runBeta').value = Math.round((sched.fraction || 0) * 100);
  syncFormRows();
  return true;
}
$('importSpecBtn').addEventListener('click', () => {
  const inp = document.createElement('input');
  inp.type = 'file';
  inp.accept = '.json,.jsonl,application/json';
  inp.onchange = () => {
    const f = inp.files[0];
    if (!f) return;
    f.text().then(t => {
      // JSONL: a whole-file JSON.parse would throw, so sniff the first line
      // first, exactly as the trace-versus-spec discrimination below does.
      if (looksLikeSpill(t)) { loadSpill(t); return; }
      const parsed = JSON.parse(t);
      if (Array.isArray(parsed.rounds)) { loadTrace(parsed); return; }
      if (fillFormFromSpec(parsed)) {
        $('runStatus').textContent = 'scenario loaded — press run';
      } else {
        $('runStatus').textContent = 'running imported scenario…';
        post({ type: 'run', spec: parsed });
      }
    }).catch(e => { $('runStatus').textContent = 'import failed: ' + e; });
  };
  inp.click();
});
