// views + renderers: setView / update / draw over the shared state
import { S, $, BOT, BLOCKED, colorOf, statesAt, roundInfo } from './state.js';

export function setView(v) {
  S.view = v;
  // scope to the top bar — .vt is also used by in-form toggles (SMR proto)
  document.querySelectorAll('.viewTabs .vt').forEach(b => b.classList.toggle('on', b.dataset.view === v));
  document.querySelectorAll('.view').forEach(el => el.classList.toggle('on', el.id === 'view-' + v));
  // per-view chrome: the replay transport bows out of the smr view
  $('app').dataset.view = v;
  draw();
}
document.querySelectorAll('.viewTabs .vt').forEach(b => b.addEventListener('click', () => setView(b.dataset.view)));

export function update() {
  const tr = S.trace;
  if (!tr) { draw(); return; }
  const states = statesAt(S.t);
  const info = roundInfo(S.t);
  let holders = 0; const counts = {};
  for (const v of states) if (v !== null) { holders++; counts[v] = (counts[v] || 0) + 1; }
  const distinct = Object.keys(counts).sort((a, b) => counts[b] - counts[a]);

  const bf = tr.rounds.length ? Math.round(tr.rounds[0].blocked.length / tr.n * 100) : 0;
  $('metaLine').textContent = `n=${tr.n} · seed=${tr.seed} · ${bf}% blocked/round · ${tr.rounds.length} rounds`;

  const last = tr.rounds.length ? tr.rounds[tr.rounds.length - 1].states : tr.initial;
  const lastVals = new Set(last.filter(v => v !== null));
  const chip = $('outcomeChip');
  if (lastVals.size === 0) { chip.textContent = 'death spiral — all ⊥'; chip.className = 'chip chip-dead'; }
  else if (lastVals.size === 1) { chip.textContent = 'agreement on ' + [...lastVals][0]; chip.className = 'chip chip-ok'; }
  else { chip.textContent = 'no convergence'; chip.className = 'chip chip-run'; }

  $('roundLabel').textContent = $('roundLabel2').textContent = `${S.t} / ${tr.rounds.length}`;
  $('holdersLabel').textContent = `${holders} / ${tr.n}`;
  $('botCount').textContent = tr.n - holders;
  $('blockedCount').textContent = info ? info.blocked.length : '0 (initial)';
  $('distinctCount').textContent = distinct.length;
  $('scrub').value = S.t;
  $('playBtn').textContent = S.playing ? '⏸' : '▶️';

  let lg = '';
  const maxLegend = Math.min(distinct.length, 8);
  for (let d = 0; d < maxLegend; d++) {
    lg += `<div class="legendRow"><div class="sw" style="background:${colorOf(+distinct[d])}"></div><span class="lgLabel mono">value ${distinct[d]}</span><span class="rv mono">${counts[distinct[d]]}</span></div>`;
  }
  if (distinct.length > 8) lg += `<div class="legendRow"><div class="sw"></div><span class="lgLabel mono">+ ${distinct.length - 8} more values</span></div>`;
  if (tr.n - holders > 0) lg += `<div class="legendRow"><div class="sw" style="background:${BOT}"></div><span class="lgLabel mono">⊥ undecided</span><span class="rv mono">${tr.n - holders}</span></div>`;
  $('legend').innerHTML = lg;

  const insp = $('inspector');
  if (S.sel !== null) {
    insp.classList.remove('hidden');
    const s = S.sel;
    $('selId').textContent = '#' + s;
    $('selState').textContent = states[s] === null ? '⊥' : 'value ' + states[s];
    const isBlocked = info ? info.blocked.includes(s) : false;
    $('selBlocked').textContent = info ? (isBlocked ? 'yes' : 'no') : '–';
    const hasT = info && info.targets && info.targets.length === tr.n;
    if (hasT) {
      $('selTargets').textContent = info.targets[s].join(', ');
      const by = [];
      for (let q = 0; q < tr.n; q++) if (q !== s && info.targets[q].includes(s)) by.push(q);
      $('selSamplers').textContent = by.length ? by.join(', ') : '—';
    } else {
      $('selTargets').textContent = info ? 'not recorded (large n)' : '– (initial)';
      $('selSamplers').textContent = '–';
    }
  } else insp.classList.add('hidden');

  draw();
}

/* ---------- canvases ---------- */
function prep(id) {
  const cv = $(id);
  if (!cv) return null;
  const dpr = window.devicePixelRatio || 1;
  const w = cv.clientWidth, h = cv.clientHeight;
  if (w === 0 || h === 0) return null;
  if (cv.width !== Math.round(w * dpr) || cv.height !== Math.round(h * dpr)) { cv.width = Math.round(w * dpr); cv.height = Math.round(h * dpr); }
  const ctx = cv.getContext('2d');
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  return { ctx, w, h };
}

function layout(w, h, n) {
  const cols = Math.ceil(Math.sqrt(n * (w / Math.max(h, 1))));
  const rows = Math.ceil(n / cols);
  const cell = Math.min(w / cols, h / rows);
  return { cols, cell, ox: (w - cols * cell) / 2 + cell / 2, oy: (h - rows * cell) / 2 + cell / 2, r: Math.max(1, cell * 0.3) };
}
export const pos = (i, L) => ({ x: L.ox + (i % L.cols) * L.cell, y: L.oy + Math.floor(i / L.cols) * L.cell });

export function draw() { drawNet(); drawSpark(); drawStacked(); drawHeat(); }

function drawNet() {
  const P = prep('net');
  if (!P) return;
  const { ctx, w, h } = P;
  const tr = S.trace;
  if (!tr) { ctx.fillStyle = '#6f7986'; ctx.font = '14px ui-monospace'; ctx.fillText('no trace — use the run tab or drop a trace.json', 20, 30); return; }
  const L = layout(w - 24, h - 24, tr.n);
  L.ox += 12; L.oy += 12;
  S.layout = L;
  const pixelMode = L.cell < 6;
  const states = statesAt(S.t);
  const info = roundInfo(S.t);
  const hasTargets = info && info.targets && info.targets.length === tr.n;
  const blockedSet = new Set(info ? info.blocked : []);

  if (hasTargets && S.edgeMode === 'all' && !pixelMode) {
    ctx.globalAlpha = 0.05; ctx.strokeStyle = '#8b94a1'; ctx.lineWidth = 1;
    for (let i = 0; i < tr.n; i++) {
      const pi = pos(i, L);
      for (const t of info.targets[i]) {
        const pt = pos(t, L);
        ctx.beginPath(); ctx.moveTo(pi.x, pi.y); ctx.lineTo(pt.x, pt.y); ctx.stroke();
      }
    }
    ctx.globalAlpha = 1;
  }
  if (hasTargets && S.edgeMode !== 'none' && S.sel !== null && !pixelMode) {
    const s = S.sel, ps = pos(s, L);
    ctx.lineWidth = 1.5; ctx.strokeStyle = '#269c86'; ctx.globalAlpha = 0.95;
    for (const o of info.targets[s]) { const po = pos(o, L); ctx.beginPath(); ctx.moveTo(ps.x, ps.y); ctx.lineTo(po.x, po.y); ctx.stroke(); }
    ctx.strokeStyle = '#c05a9e'; ctx.setLineDash([4, 3]);
    for (let q = 0; q < tr.n; q++) {
      if (q !== s && info.targets[q].includes(s)) { const pq = pos(q, L); ctx.beginPath(); ctx.moveTo(pq.x, pq.y); ctx.lineTo(ps.x, ps.y); ctx.stroke(); }
    }
    ctx.setLineDash([]); ctx.globalAlpha = 1;
  }

  if (pixelMode) {
    const side = Math.max(1, L.cell * 0.92), half = side / 2;
    for (let k = 0; k < tr.n; k++) {
      const p = pos(k, L), v = states[k];
      ctx.fillStyle = colorOf(v);
      ctx.globalAlpha = v === null ? 0.4 : 1;
      ctx.fillRect(p.x - half, p.y - half, side, side);
    }
    ctx.globalAlpha = 1;
    ctx.fillStyle = '#6f7986'; ctx.font = '11px ui-monospace';
    ctx.fillText(`pixel mode (n = ${tr.n}) — rings, selection and edges need fewer nodes`, 14, h - 10);
  } else {
    for (let k = 0; k < tr.n; k++) {
      const p = pos(k, L), v = states[k];
      ctx.beginPath(); ctx.arc(p.x, p.y, L.r, 0, 7);
      ctx.fillStyle = colorOf(v);
      ctx.globalAlpha = v === null ? 0.45 : 1;
      ctx.fill(); ctx.globalAlpha = 1;
      if (blockedSet.has(k)) { ctx.lineWidth = 2; ctx.strokeStyle = BLOCKED; ctx.beginPath(); ctx.arc(p.x, p.y, L.r + 2.5, 0, 7); ctx.stroke(); }
      if (S.sel === k) { ctx.lineWidth = 2; ctx.strokeStyle = '#eef2f6'; ctx.beginPath(); ctx.arc(p.x, p.y, L.r + 5, 0, 7); ctx.stroke(); }
    }
  }
}

function drawSpark() {
  const P = prep('spark');
  if (!P || !S.trace || !S.series) return;
  const { ctx, w, h } = P;
  const T = S.trace.rounds.length;
  ctx.strokeStyle = '#3dd6c4'; ctx.lineWidth = 1.5; ctx.beginPath();
  for (let t = 0; t <= T; t++) {
    const holders = S.trace.n - S.series.comp[t].bot;
    const x = T === 0 ? 4 : t / T * (w - 8) + 4;
    const y = h - 6 - holders / S.trace.n * (h - 12);
    t === 0 ? ctx.moveTo(x, y) : ctx.lineTo(x, y);
  }
  ctx.stroke();
  const cx = T === 0 ? 4 : S.t / T * (w - 8) + 4;
  ctx.strokeStyle = '#8b94a1'; ctx.setLineDash([3, 3]);
  ctx.beginPath(); ctx.moveTo(cx, 2); ctx.lineTo(cx, h - 2); ctx.stroke();
  ctx.setLineDash([]);
}

function drawStacked() {
  const P = prep('stacked');
  if (!P || !S.trace || !S.series) return;
  const { ctx, w, h } = P;
  const tr = S.trace, T = tr.rounds.length, sr = S.series;
  const strip = 8, ph = h - strip - 2;
  const xAt = t => T === 0 ? 0 : t / T * w;
  const manyBands = sr.values.length > 24;
  for (let v = 0; v < sr.values.length; v++) {
    ctx.beginPath();
    for (let t = 0; t <= T; t++) {
      let base = 0;
      for (let u = 0; u < v; u++) base += sr.comp[t].counts[u];
      const y = ph - base / tr.n * ph;
      t === 0 ? ctx.moveTo(xAt(t), y) : ctx.lineTo(xAt(t), y);
    }
    for (let t = T; t >= 0; t--) {
      let base = 0;
      for (let u = 0; u <= v; u++) base += sr.comp[t].counts[u];
      ctx.lineTo(xAt(t), ph - base / tr.n * ph);
    }
    ctx.closePath();
    ctx.fillStyle = colorOf(sr.values[v]);
    ctx.globalAlpha = 0.9; ctx.fill(); ctx.globalAlpha = 1;
    if (!manyBands) { ctx.strokeStyle = '#14181e'; ctx.lineWidth = 1; ctx.stroke(); }
  }
  ctx.beginPath();
  for (let t = 0; t <= T; t++) {
    const hold = tr.n - sr.comp[t].bot;
    const y = ph - hold / tr.n * ph;
    t === 0 ? ctx.moveTo(xAt(t), y) : ctx.lineTo(xAt(t), y);
  }
  ctx.lineTo(xAt(T), 0); ctx.lineTo(0, 0); ctx.closePath();
  ctx.fillStyle = BOT; ctx.globalAlpha = 0.5; ctx.fill(); ctx.globalAlpha = 1;
  for (let t = 0; t <= T; t++) {
    const bfr = sr.comp[t].blocked / tr.n;
    if (bfr <= 0) continue;
    ctx.fillStyle = BLOCKED;
    ctx.globalAlpha = Math.min(0.25 + bfr * 2, 1);
    const x0 = t === 0 ? 0 : xAt(t - 0.5), x1 = t === T ? w : xAt(t + 0.5);
    ctx.fillRect(x0, ph + 2, x1 - x0, strip - 2);
    ctx.globalAlpha = 1;
  }
  const cx = xAt(S.t);
  ctx.strokeStyle = '#eef2f6'; ctx.lineWidth = 1; ctx.setLineDash([3, 3]);
  ctx.beginPath(); ctx.moveTo(cx, 0); ctx.lineTo(cx, h); ctx.stroke();
  ctx.setLineDash([]);
}

function drawHeat() {
  const P = prep('heat');
  if (!P || !S.trace || !S.rowOrder) return;
  const { ctx, w, h } = P;
  const tr = S.trace, T = tr.rounds.length;
  const maxRows = Math.max(Math.floor(h), 100);
  const stepN = Math.max(1, Math.ceil(tr.n / maxRows));
  const rowsShown = Math.ceil(tr.n / stepN);
  const cw = w / (T + 1), ch = h / rowsShown;
  for (let t = 0; t <= T; t++) {
    const st = statesAt(t);
    const info = roundInfo(t);
    const blockedSet = new Set(info ? info.blocked : []);
    for (let r = 0; r < rowsShown; r++) {
      const node = S.rowOrder[r * stepN];
      const v = st[node];
      ctx.fillStyle = colorOf(v);
      ctx.globalAlpha = v === null ? 0.35 : 0.95;
      ctx.fillRect(t * cw, r * ch, Math.ceil(cw), Math.ceil(ch));
      ctx.globalAlpha = 1;
      if (blockedSet.has(node)) { ctx.fillStyle = BLOCKED; ctx.fillRect(t * cw, r * ch, Math.max(1.5, cw * 0.25), Math.ceil(ch)); }
    }
  }
  $('heatNote').textContent = stepN > 1 ? `showing every ${stepN}. node of ${tr.n} · red notch = blocked` : 'rows = nodes (grouped by initial value) · red notch = blocked';
  const cx = (S.t + 0.5) * cw;
  ctx.strokeStyle = '#eef2f6'; ctx.lineWidth = 1; ctx.setLineDash([3, 3]);
  ctx.beginPath(); ctx.moveTo(cx, 0); ctx.lineTo(cx, h); ctx.stroke();
  ctx.setLineDash([]);
}
