// Master timeline: six lanes on ONE canvas and ONE x-mapping, so stimulus
// (β), response (windows, R-states, backlog) and outcome (spread, prefix)
// are read off the same round. Recovery mode only — it subsumes the
// standalone spread and prefix panels.
import { S, $ } from './state.js';

const PADL = 52, PADR = 14, PADT = 8, PADB = 26, GAP = 4;

const LANES = [
  { key: 'beta', h: 34, label: 'β', draw: drawBeta },
  { key: 'win', h: 40, label: 'W', draw: drawWindows },
  { key: 'rstate', h: 46, label: 'R', draw: drawRStates },
  { key: 'backlog', h: 40, label: 'queue', draw: drawBacklog },
  { key: 'gamma', h: 52, label: 'γ', draw: drawGamma },
  { key: 'prefix', h: 28, label: 'lcp', draw: drawPrefixLane },
];

let curves = null;
export function useSpreadCurves(fn) { curves = fn; }

// Derived series are recomputed per report; the report is replaced wholesale
// every round, so identity is a sound cache key.
let memo = { rep: null, data: null };
function derive(rep) {
  if (memo.rep === rep) return memo.data;
  const rounds = rep.metrics.length;
  const pending = new Array(rounds + 1).fill(0);
  const oldest = new Array(rounds + 1).fill(0);
  for (let r = 1; r <= rounds; r++) {
    let count = 0, first = null;
    for (const c of rep.commands) {
      if (c.injection_round > r) continue;
      if (c.executed_round != null && c.executed_round <= r) continue;
      count++;
      if (first === null || c.injection_round < first) first = c.injection_round;
    }
    pending[r] = count;
    oldest[r] = first === null ? 0 : r - first;
  }
  memo = { rep, data: { pending, oldest, maxPending: Math.max(1, ...pending), maxAge: Math.max(1, ...oldest) } };
  return memo.data;
}

export function drawTimeline(rep) {
  const cv = $('smrTimeline');
  const rows = rep.recovery ? rep.recovery.rounds : null;
  if (!rows || !rows.length) return;
  const total = LANES.reduce((a, l) => a + l.h + GAP, 0) - GAP;
  cv.style.height = (total + PADT + PADB) + 'px';
  const dpr = window.devicePixelRatio || 1;
  const W = cv.clientWidth, H = cv.clientHeight;
  if (W < 2 || H < 2) return;
  cv.width = W * dpr; cv.height = H * dpr;
  const ctx = cv.getContext('2d');
  ctx.scale(dpr, dpr);
  ctx.clearRect(0, 0, W, H);
  ctx.font = '9px ui-monospace, Menlo, monospace';

  const rounds = Math.max(1, rep.metrics.length);
  const T = S.smr.tw || 1;
  const X = r => PADL + (r / rounds) * (W - PADL - PADR);
  const n = rows[0].noreset + rows[0].reset + rows[0].bot_r || 1;
  const ctxArgs = { rep, rows, rounds, T, X, n, W, derived: derive(rep) };

  let y = PADT;
  for (const lane of LANES) {
    // Boundary gridlines run through every lane so the clock is one ruler.
    ctx.strokeStyle = '#1a1f27'; ctx.lineWidth = 1;
    for (let b = T; b <= rounds; b += T) {
      ctx.beginPath(); ctx.moveTo(X(b), y); ctx.lineTo(X(b), y + lane.h); ctx.stroke();
    }
    ctx.fillStyle = '#565f6b'; ctx.textAlign = 'right'; ctx.textBaseline = 'top';
    ctx.fillText(lane.label, PADL - 8, y + 2);
    lane.draw(ctx, y, lane.h, ctxArgs);
    y += lane.h + GAP;
  }

  // x axis
  ctx.textAlign = 'center'; ctx.textBaseline = 'top'; ctx.fillStyle = '#565f6b';
  const step = Math.max(T, Math.ceil(rounds / 8 / T) * T);
  for (let r = 0; r <= rounds; r += step) ctx.fillText('r' + r, X(r), H - PADB + 4);

  const cursor = S.smr.tl && S.smr.tl.cursor !== null ? S.smr.tl.cursor
    : S.smr.lastStatus ? S.smr.lastStatus.round : null;
  if (cursor !== null) {
    ctx.strokeStyle = '#e0af68'; ctx.setLineDash([3, 3]);
    ctx.beginPath(); ctx.moveTo(X(cursor), PADT); ctx.lineTo(X(cursor), H - PADB); ctx.stroke();
    ctx.setLineDash([]);
  }
  S.smr.tl = Object.assign(S.smr.tl || {}, { X, rounds, W });
}

function drawBeta(ctx, y0, h, { rep, rounds, X, n }) {
  ctx.fillStyle = 'rgba(247,118,142,0.30)';
  ctx.beginPath(); ctx.moveTo(X(0), y0 + h);
  rep.metrics.forEach((m, i) => ctx.lineTo(X(i + 1), y0 + h - (m.blocked / n) * h));
  ctx.lineTo(X(rounds), y0 + h); ctx.closePath(); ctx.fill();
  ctx.strokeStyle = '#f7768e'; ctx.lineWidth = 1;
  ctx.beginPath();
  rep.metrics.forEach((m, i) => {
    const x = X(i + 1), yy = y0 + h - (m.blocked / n) * h;
    i === 0 ? ctx.moveTo(x, yy) : ctx.lineTo(x, yy);
  });
  ctx.stroke();
}

// Boundary effects are latched onto the row AFTER the boundary, so a badge
// reads row mT+1 but must be drawn at the boundary it belongs to.
function drawWindows(ctx, y0, h, { rows, rounds, T, X }) {
  const maxW = Math.max(1, ...rows.map(r => r.max_checkpoint_window));
  ctx.strokeStyle = '#7dcfff'; ctx.lineWidth = 1.4;
  ctx.beginPath();
  rows.forEach((r, i) => {
    const x = X(i + 1), yy = y0 + h - (r.max_checkpoint_window / maxW) * (h - 10);
    i === 0 ? ctx.moveTo(x, yy) : ctx.lineTo(x, yy);
  });
  ctx.stroke();
  ctx.textAlign = 'center'; ctx.textBaseline = 'middle';
  for (let b = T; b <= rounds; b += T) {
    const row = rows[b];
    if (!row || row.rollbacks === 0) continue;
    ctx.fillStyle = '#f7768e';
    ctx.fillText(`⟲${row.rollbacks}${row.rollback_depth ? '/' + row.rollback_depth : ''}`, X(b), y0 + 6);
  }
}

function drawRStates(ctx, y0, h, { rows, X, n }) {
  const COLS = ['#3dd6c4', '#e0af68', '#4b5563'];
  const KEYS = ['noreset', 'reset', 'bot_r'];
  let base = new Array(rows.length).fill(0);
  KEYS.forEach((key, k) => {
    ctx.fillStyle = COLS[k];
    ctx.beginPath();
    rows.forEach((r, i) => {
      const x = X(i + 1), yy = y0 + h - (base[i] / n) * h;
      i === 0 ? ctx.moveTo(x, yy) : ctx.lineTo(x, yy);
    });
    for (let i = rows.length - 1; i >= 0; i--) {
      base[i] += rows[i][key];
      ctx.lineTo(X(i + 1), y0 + h - (base[i] / n) * h);
    }
    ctx.closePath(); ctx.fill();
  });
}

function drawBacklog(ctx, y0, h, { rounds, X, derived }) {
  const { pending, oldest, maxPending, maxAge } = derived;
  ctx.fillStyle = 'rgba(187,154,247,0.25)';
  ctx.beginPath(); ctx.moveTo(X(0), y0 + h);
  for (let r = 1; r <= rounds; r++) ctx.lineTo(X(r), y0 + h - (pending[r] / maxPending) * h);
  ctx.lineTo(X(rounds), y0 + h); ctx.closePath(); ctx.fill();
  // Oldest-age is the re-aging signal: a backlog that stops draining climbs.
  ctx.strokeStyle = '#bb9af7'; ctx.lineWidth = 1.2;
  ctx.beginPath();
  for (let r = 1; r <= rounds; r++) {
    const x = X(r), yy = y0 + h - (oldest[r] / maxAge) * h;
    r === 1 ? ctx.moveTo(x, yy) : ctx.lineTo(x, yy);
  }
  ctx.stroke();
}

function drawGamma(ctx, y0, h, { rep, rounds, X }) {
  const Y = g => y0 + h - g * h;
  ctx.strokeStyle = '#1c2129'; ctx.lineWidth = 1;
  ctx.beginPath(); ctx.moveTo(PADL, Y(1)); ctx.lineTo(X(rounds), Y(1)); ctx.stroke();
  if (curves) curves(ctx, rep, X, Y, null, { thin: true });
}

function drawPrefixLane(ctx, y0, h, { rep, rounds, X }) {
  const bw = Math.max(0.75, (X(rounds) - X(0)) / Math.max(1, rep.metrics.length) - 0.5);
  rep.metrics.forEach((m, i) => {
    const denom = Math.max(m.max_log_len, m.max_executed_len, 1);
    const v = Math.max(0, Math.min(1, m.lcp_len / denom));
    ctx.fillStyle = v > 0.995 ? '#3dd6c4' : 'rgba(61,214,196,0.45)';
    ctx.fillRect(X(i) + 0.25, y0 + h - v * h, bw, v * h);
  });
}

/* ---------- hover + seek ---------- */

function roundAt(ev) {
  const tl = S.smr.tl;
  if (!tl) return null;
  const r = $('smrTimeline').getBoundingClientRect();
  const frac = (ev.clientX - r.left - PADL) / (tl.W - PADL - PADR);
  return Math.max(1, Math.min(tl.rounds, Math.round(frac * tl.rounds)));
}

$('smrTimeline').addEventListener('mousemove', ev => {
  const rep = S.smr.report;
  const round = roundAt(ev);
  const tip = $('tip');
  if (!rep || round === null || !rep.recovery) { tip.classList.add('hidden'); return; }
  const m = rep.metrics[round - 1], row = rep.recovery.rounds[round - 1];
  if (!m || !row) { tip.classList.add('hidden'); return; }
  const d = derive(rep);
  const n = row.noreset + row.reset + row.bot_r || 1;
  tip.innerHTML =
    `<b>r${round}</b> · W ${row.window}<br>` +
    `β ${Math.round((m.blocked / n) * 100)}% · logs ${m.nonbot_logs}<br>` +
    `R ${row.noreset}/${row.reset}/${row.bot_r} (no-reset/reset/⊥)<br>` +
    `queue ${d.pending[round]} · oldest ${d.oldest[round]}r<br>` +
    `exec ${m.min_executed_len}–${m.max_executed_len} · cp W ${row.max_checkpoint_window}` +
    (row.rollbacks ? `<br>⟲ ${row.rollbacks} rollbacks, depth ${row.rollback_depth}` : '');
  tip.classList.remove('hidden');
  tip.style.left = (ev.clientX + 12) + 'px';
  tip.style.top = (ev.clientY + 12) + 'px';
});
$('smrTimeline').addEventListener('mouseleave', () => $('tip').classList.add('hidden'));

// Click-to-seek moves the derived cursor; the node grid is deliberately
// excluded — there is no per-node history to seek through.
$('smrTimeline').addEventListener('click', ev => {
  const lv = S.smr.live;
  if (lv && lv.playing) return;
  const round = roundAt(ev);
  if (round === null) return;
  S.smr.tl = Object.assign(S.smr.tl || {}, { cursor: round });
  if (S.smr.report) drawTimeline(S.smr.report);
});
