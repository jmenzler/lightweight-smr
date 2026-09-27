// live sessions: stepping loop, absorbing stops, liveCol wiring
import { S, $ } from './state.js';
import { post } from './engine.js';
import { update } from './render.js';
import { buildSpec, validateSpec } from './form.js';

/* ---------- live mode ---------- */
function appendComp(states, blockedLen) {
  const { values, vIndex, comp } = S.series;
  const counts = new Array(values.length).fill(0);
  let bot = 0;
  for (const v of states) { if (v === null) bot++; else counts[vIndex[v]]++; }
  comp.push({ counts, bot, blocked: blockedLen });
}
export function updateLivePlayBtn() {
  $('livePlayBtn').textContent = S.live && S.live.playing ? '⏸' : '▶️';
}
function liveStep() {
  if (!S.live || S.live.awaiting) return;
  S.live.awaiting = true;
  post({ type: 'live-step', fraction: (+$('liveSlider').value) / 100 });
}
export function onLiveRound(r) {
  if (!S.live) return;
  S.live.awaiting = false;
  if (!r.absorbed) {
    S.trace.rounds.push({ states: r.states, targets: r.targets, blocked: r.blocked });
    appendComp(r.states, r.blocked.length);
    $('scrub').max = S.trace.rounds.length;
    S.t = S.trace.rounds.length;
  }
  $('liveRound').textContent = 'r ' + r.round;
  const holdAgreed = r.agreed_value !== null && r.all_hold;
  const sliderZero = +$('liveSlider').value === 0;
  if (r.absorbed || r.all_undecided || (holdAgreed && sliderZero && S.live.bg === 0)) S.live.playing = false;
  updateLivePlayBtn();
  update();
  if (S.live.playing) setTimeout(liveStep, 700 / S.speed);
}
$('liveBtn').addEventListener('click', () => {
  if (S.engine !== 'wasm') { $('runStatus').textContent = 'engine not ready'; return; }
  const spec = buildSpec();
  const problem = validateSpec(spec);
  if (problem) { $('runStatus').textContent = 'error: ' + problem; return; }
  const sticky = $('liveMode').value === 'sticky';
  S.pendingLiveBg = sticky && spec.schedule.kind === 'fresh_per_round' ? spec.schedule.fraction : 0;
  $('runStatus').textContent = 'starting live session…';
  post({ type: 'live-new', spec, sticky });
});
$('liveSlider').addEventListener('input', () => {
  $('liveSliderVal').textContent = 'β ' + $('liveSlider').value + '%';
});
$('livePlayBtn').addEventListener('click', () => {
  if (!S.live) return;
  S.live.playing = !S.live.playing;
  updateLivePlayBtn();
  if (S.live.playing) liveStep();
});
$('liveStepBtn').addEventListener('click', () => {
  if (!S.live) return;
  S.live.playing = false;
  updateLivePlayBtn();
  liveStep();
});
$('liveExportSpec').addEventListener('click', () => { if (S.live) post({ type: 'live-export' }); });
$('liveExportTrace').addEventListener('click', () => { if (S.live) post({ type: 'live-trace' }); });
$('liveEndBtn').addEventListener('click', () => {
  S.live = null;
  $('liveCol').classList.add('hidden');
});
