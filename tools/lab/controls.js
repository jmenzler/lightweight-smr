// replay transport, keyboard, canvas interactions, trace file loading
import { S, $ } from './state.js';
import { update, draw, pos } from './render.js';
import { loadTrace } from './trace.js';
import { looksLikeSpill, loadSpill } from './spill.js';

/* ---------- interactions ---------- */
function step(d) {
  if (!S.trace) return;
  S.t = Math.min(Math.max(S.t + d, 0), S.trace.rounds.length);
  S.playing = false;
  update();
}
$('toStart').addEventListener('click', () => { S.t = 0; S.playing = false; update(); });
$('toEnd').addEventListener('click', () => { if (S.trace) { S.t = S.trace.rounds.length; S.playing = false; update(); } });
$('stepBack').addEventListener('click', () => step(-1));
$('stepFwd').addEventListener('click', () => step(1));
$('playBtn').addEventListener('click', () => { S.playing = !S.playing; update(); });
$('scrub').addEventListener('input', e => { S.t = +e.target.value; S.playing = false; update(); });
$('speed').addEventListener('change', e => S.speed = +e.target.value);
$('edgeMode').addEventListener('change', e => { S.edgeMode = e.target.value; draw(); });

let last = 0;
setInterval(() => {
  if (!S.playing || !S.trace) return;
  const now = Date.now();
  if (!last) last = now;
  if (now - last >= 700 / S.speed) {
    last = now;
    if (S.t >= S.trace.rounds.length) S.playing = false;
    else S.t++;
    update();
  }
}, 60);

window.addEventListener('keydown', e => {
  if (e.target.tagName === 'INPUT' || e.target.tagName === 'SELECT') return;
  if (e.key === 'ArrowRight') { e.preventDefault(); step(1); }
  else if (e.key === 'ArrowLeft') { e.preventDefault(); step(-1); }
  else if (e.key === ' ') { e.preventDefault(); S.playing = !S.playing; update(); }
});

$('net').addEventListener('click', e => {
  if (!S.trace || !S.layout || S.layout.cell < 6) return;
  const rect = e.target.getBoundingClientRect();
  const x = e.clientX - rect.left, y = e.clientY - rect.top;
  let best = null, bestD = Infinity;
  for (let i = 0; i < S.trace.n; i++) {
    const p = pos(i, S.layout);
    const d = (p.x - x) ** 2 + (p.y - y) ** 2;
    if (d < bestD) { bestD = d; best = i; }
  }
  S.sel = bestD <= S.layout.cell * S.layout.cell * 0.4 ? best : null;
  update();
});

const xToT = e => {
  const rect = e.target.getBoundingClientRect();
  return Math.min(Math.max(Math.round((e.clientX - rect.left) / rect.width * S.trace.rounds.length), 0), S.trace.rounds.length);
};
for (const id of ['stacked', 'heat']) {
  const cv = $(id);
  cv.addEventListener('click', e => { if (S.trace) { S.t = xToT(e); S.playing = false; update(); } });
  cv.addEventListener('mousemove', e => {
    if (!S.trace || !S.series) return;
    if (e.buttons === 1) { S.t = xToT(e); S.playing = false; update(); }
    const t = xToT(e);
    const comp = S.series.comp[t];
    const lines = ['round ' + t];
    const order = [];
    for (let i = 0; i < S.series.values.length; i++) if (comp.counts[i] > 0) order.push(i);
    order.sort((a, b) => comp.counts[b] - comp.counts[a]);
    const shown = Math.min(order.length, 8);
    for (let o = 0; o < shown; o++) lines.push(`value ${S.series.values[order[o]]}: ${Math.round(comp.counts[order[o]] / S.trace.n * 100)}%`);
    if (order.length > shown) lines.push(`+ ${order.length - shown} more values`);
    if (comp.bot > 0) lines.push(`⊥: ${Math.round(comp.bot / S.trace.n * 100)}%`);
    lines.push('blocked: ' + comp.blocked);
    const tip = $('tip');
    tip.innerHTML = lines.join('<br/>');
    tip.style.display = 'block';
    tip.style.left = Math.min(e.clientX + 14, window.innerWidth - 170) + 'px';
    tip.style.top = (e.clientY - 10) + 'px';
  });
  cv.addEventListener('mouseleave', () => $('tip').style.display = 'none');
}

/* load / drag-drop */
// A spill is JSONL, so a whole-file JSON.parse throws on it. Sniff the first
// line before parsing, the same way form.js discriminates a trace from a spec.
const ingest = text => { if (looksLikeSpill(text)) loadSpill(text); else loadTrace(JSON.parse(text)); };

$('loadBtn').addEventListener('click', () => {
  const inp = document.createElement('input');
  inp.type = 'file'; inp.accept = '.json,.jsonl,application/json';
  inp.onchange = () => {
    const f = inp.files && inp.files[0];
    if (!f) return;
    f.text().then(t => ingest(t)).catch(e => console.warn(e));
  };
  inp.click();
});
document.addEventListener('dragover', e => { e.preventDefault(); $('dropHint').classList.add('on'); });
document.addEventListener('dragleave', e => { if (e.relatedTarget === null) $('dropHint').classList.remove('on'); });
document.addEventListener('drop', e => {
  e.preventDefault();
  $('dropHint').classList.remove('on');
  const f = e.dataTransfer && e.dataTransfer.files && e.dataTransfer.files[0];
  if (f) f.text().then(t => ingest(t)).catch(err => console.warn(err));
});
