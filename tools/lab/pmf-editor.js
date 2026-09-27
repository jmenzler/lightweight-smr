// pmf curve editor factory — one instance per (id-prefix, row) pair; presets
// share one localStorage store across instances.
import { $, clamp01 } from './state.js';

export function createPmfEditor(prefix, rowId) {
  const el = name => $(prefix + name);
  const ed = {
    points: [[0, 0.5], [1, 0.5]],
    imported: null,
    analytic: null,
    drag: null,
    range() {
      const r = Math.round(+el('Range').value || 32);
      return Math.min(Math.max(r, 2), 9007199254740992);
    },
    m() { return Math.min(this.range(), 4096); },
    analyticWeights(m) {
      if (this.analytic === 'geometric') {
        // ratio picked so first:last stays 100:3 at any m — shape-stable
        const r = m > 1 ? Math.pow(0.03, 1 / (m - 1)) : 1;
        const w = [];
        let sum = 0;
        for (let i = 0; i < m; i++) { const v = Math.pow(r, i); w.push(v); sum += v; }
        return w.map(v => v / sum);
      }
      // zipf s=1: w_i ∝ 1/(i+1)
      const w = [];
      let sum = 0;
      for (let i = 0; i < m; i++) { const v = 1 / (i + 1); w.push(v); sum += v; }
      return w.map(v => v / sum);
    },
    curveAt(t) {
      const p = this.points;
      if (t <= p[0][0]) return p[0][1];
      if (t >= p[p.length - 1][0]) return p[p.length - 1][1];
      let i = 0;
      while (i < p.length - 2 && t > p[i + 1][0]) i++;
      const p0 = p[Math.max(0, i - 1)], p1 = p[i], p2 = p[i + 1], p3 = p[Math.min(p.length - 1, i + 2)];
      const u = (t - p1[0]) / ((p2[0] - p1[0]) || 1e-9), u2 = u * u, u3 = u2 * u;
      return 0.5 * (2 * p1[1] + (-p0[1] + p2[1]) * u + (2 * p0[1] - 5 * p1[1] + 4 * p2[1] - p3[1]) * u2 + (-p0[1] + 3 * p1[1] - 3 * p2[1] + p3[1]) * u3);
    },
    formula: null,
    formulaError: null,
    compileFormula(expr) {
      // ^ means power in formulas; JS xor is never what a pmf wants
      const body = expr.replace(/\^/g, '**');
      const fn = new Function(
        'i', 'm', 'x', 'exp', 'pow', 'sqrt', 'abs', 'log', 'sin', 'cos', 'min', 'max', 'floor', 'PI', 'E',
        'return (' + body + ');'
      );
      return i2 => {
        const m = this.m();
        return fn(
          i2, m, m > 1 ? i2 / (m - 1) : 0,
          Math.exp, Math.pow, Math.sqrt, Math.abs, Math.log, Math.sin, Math.cos, Math.min, Math.max, Math.floor, Math.PI, Math.E
        );
      };
    },
    formulaWeights(m) {
      const fn = this.compileFormula(this.formula);
      const w = [];
      let sum = 0;
      for (let i = 0; i < m; i++) {
        let v = Number(fn(i));
        if (!Number.isFinite(v) || v < 0) v = 0;
        w.push(v); sum += v;
      }
      if (sum <= 0) throw new Error('all weights zero');
      return w.map(v => v / sum);
    },
    weights() {
      if (this.formula) {
        try {
          const w = this.formulaWeights(this.m());
          this.formulaError = null;
          return w;
        } catch (e) {
          this.formulaError = String(e.message || e);
        }
      }
      if (this.analytic) return this.analyticWeights(this.m());
      if (this.imported) return this.imported;
      const m = this.m(), w = [];
      let sum = 0;
      for (let i = 0; i < m; i++) {
        const v = Math.max(0, this.curveAt((i + 0.5) / m));
        w.push(v); sum += v;
      }
      return sum > 0 ? w.map(v => v / sum) : w.map(() => 1 / m);
    },
    setPoints(pts) { this.points = pts.map(p => [p[0], p[1]]); this.imported = null; this.analytic = null; this.clearFormula(); this.draw(); },
    clearFormula() { this.formula = null; this.formulaError = null; const f = el('Formula'); if (f) f.value = ''; },
    toNorm(e) {
      const r = el('Canvas').getBoundingClientRect();
      return [(e.clientX - r.left) / r.width, 1 - (e.clientY - r.top) / r.height];
    },
    hit(e) {
      const n = this.toNorm(e);
      let best = -1, bd = 1e9;
      this.points.forEach((p, i) => {
        const d = (p[0] - n[0]) ** 2 + ((p[1] - n[1]) * 0.6) ** 2;
        if (d < bd) { bd = d; best = i; }
      });
      return bd < 0.004 ? best : -1;
    },
    draw() {
      const cv = el('Canvas');
      if (!cv || $(rowId).classList.contains('hidden')) return;
      const ctx = cv.getContext('2d');
      const dpr = window.devicePixelRatio || 1;
      const r = cv.getBoundingClientRect();
      if (cv.width !== Math.round(r.width * dpr)) { cv.width = Math.round(r.width * dpr); cv.height = Math.round(r.height * dpr); }
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      const w = r.width, h = r.height, padB = 14;
      ctx.clearRect(0, 0, w, h);
      const wts = this.weights(), m = wts.length, maxW = Math.max(...wts, 1e-9), bw = w / m;
      ctx.fillStyle = 'rgba(61,214,196,0.45)';
      if (bw >= 2.5) {
        wts.forEach((v, i) => {
          const bh = (v / maxW) * (h - padB - 22);
          ctx.fillRect(i * bw + 1, h - padB - bh, Math.max(bw - 2, 1), bh);
        });
      } else {
        // slices thinner than bars can show — draw the area under the curve
        ctx.beginPath();
        ctx.moveTo(0, h - padB);
        wts.forEach((v, i) => {
          const bh = (v / maxW) * (h - padB - 22);
          ctx.lineTo((i + 0.5) * bw, h - padB - bh);
        });
        ctx.lineTo(w, h - padB);
        ctx.closePath();
        ctx.fill();
      }
      const formulaActive = this.formula && !this.formulaError;
      if (!this.imported && !this.analytic && !formulaActive) {
        ctx.strokeStyle = '#eef2f6'; ctx.lineWidth = 1.5; ctx.beginPath();
        for (let px = 0; px <= w; px += 2) {
          const y = Math.max(0, Math.min(1, this.curveAt(px / w)));
          const cy = (1 - y) * (h - padB);
          px === 0 ? ctx.moveTo(px, cy) : ctx.lineTo(px, cy);
        }
        ctx.stroke();
        this.points.forEach(p => {
          ctx.beginPath();
          ctx.arc(p[0] * w, (1 - p[1]) * (h - padB), 5, 0, Math.PI * 2);
          ctx.fillStyle = '#3dd6c4'; ctx.fill();
          ctx.strokeStyle = '#0f1216'; ctx.lineWidth = 2; ctx.stroke();
        });
      }
      ctx.fillStyle = '#6f7986'; ctx.font = '10px ui-monospace, Menlo, monospace';
      ctx.fillText('0', 3, h - 3);
      const rLabel = this.range() > 4096 ? (this.range() - 1).toExponential(1) : String(this.range() - 1);
      ctx.fillText(rLabel, w - 8 - rLabel.length * 6, h - 3);
      const fmtW = v => v >= 0.001 || v === 0 ? v.toFixed(3) : v.toExponential(1);
      const head = wts.slice(0, 6).map(fmtW).join(', ');
      const tag = this.formula
        ? (this.formulaError ? 'formula error: ' + this.formulaError + ' — showing fallback · ' : 'w(i) = ' + this.formula + ' · ')
        : this.analytic
          ? (this.analytic === 'zipf' ? 'zipf s=1 (exact) ' : 'geometric (exact) ')
          : this.imported ? 'imported ' : '';
      const rangeNote = this.range() > m ? '  R=' + this.range().toExponential(2) + ' (' + m + ' slices)' : '  R=' + this.range();
      el('Out').textContent = tag + 'w = [' + head + (m > 6 ? ', …' : '') + ']  Σ=1' + rangeNote;
    },
    store() { return JSON.parse(localStorage.getItem('lab-pmf-presets') || '{}'); },
    refreshSaved() {
      const sel = el('Saved');
      while (sel.options.length > 1) sel.remove(1);
      Object.keys(this.store()).forEach(name => {
        const o = document.createElement('option');
        o.value = name; o.textContent = name;
        sel.appendChild(o);
      });
    },
  };

  el('Canvas').addEventListener('mousedown', e => {
    const i = ed.hit(e);
    if (i >= 0) { ed.drag = i; e.preventDefault(); }
  });
  window.addEventListener('mousemove', e => {
    if (ed.drag === null) return;
    const n = ed.toNorm(e), p = ed.points, i = ed.drag;
    const lo = i === 0 ? 0 : p[i - 1][0] + 0.01;
    const hi = i === p.length - 1 ? 1 : p[i + 1][0] - 0.01;
    p[i] = [i === 0 ? 0 : i === p.length - 1 ? 1 : Math.min(Math.max(n[0], lo), hi), clamp01(n[1])];
    ed.imported = null;
    ed.analytic = null;
    ed.draw();
  });
  window.addEventListener('mouseup', () => { ed.drag = null; });
  el('Canvas').addEventListener('dblclick', e => {
    const n = ed.toNorm(e);
    ed.points.push([Math.min(Math.max(n[0], 0.02), 0.98), clamp01(n[1])]);
    ed.points.sort((a, b) => a[0] - b[0]);
    ed.imported = null;
    ed.analytic = null;
    ed.draw();
  });
  el('Canvas').addEventListener('contextmenu', e => {
    e.preventDefault();
    const i = ed.hit(e);
    if (i > 0 && i < ed.points.length - 1) { ed.points.splice(i, 1); ed.imported = null; ed.analytic = null; ed.draw(); }
  });
  el('Formula').addEventListener('input', () => {
    const expr = el('Formula').value.trim();
    ed.formula = expr || null;
    if (expr) { ed.imported = null; ed.analytic = null; }
    ed.draw();
  });
  el('Uniform').addEventListener('click', () => ed.setPoints([[0, 0.5], [1, 0.5]]));
  el('Zipf').addEventListener('click', () => { ed.analytic = 'zipf'; ed.imported = null; ed.clearFormula(); ed.draw(); });
  el('Geo').addEventListener('click', () => { ed.analytic = 'geometric'; ed.imported = null; ed.clearFormula(); ed.draw(); });
  el('Bimodal').addEventListener('click', () => ed.setPoints([[0, 0.9], [0.2, 0.75], [0.5, 0.05], [0.8, 0.75], [1, 0.9]]));
  el('Save').addEventListener('click', () => {
    const name = (el('Name').value || '').trim();
    if (!name) return;
    const store = ed.store();
    store[name] = ed.formula && !ed.formulaError ? { formula: ed.formula } : ed.points;
    localStorage.setItem('lab-pmf-presets', JSON.stringify(store));
    ed.refreshSaved();
  });
  el('Saved').addEventListener('change', e => {
    const entry = ed.store()[e.target.value];
    if (!entry) return;
    if (Array.isArray(entry)) { ed.setPoints(entry); return; }
    ed.formula = entry.formula;
    el('Formula').value = entry.formula;
    ed.imported = null;
    ed.analytic = null;
    ed.draw();
  });
  el('Delete').addEventListener('click', () => {
    const sel = el('Saved');
    if (!sel.value) return;
    const store = ed.store();
    delete store[sel.value];
    localStorage.setItem('lab-pmf-presets', JSON.stringify(store));
    ed.refreshSaved();
  });
  el('Range').addEventListener('input', () => ed.draw());
  ed.refreshSaved();
  return ed;
}

export const PMF = createPmfEditor('pmf', 'rowPmf');
