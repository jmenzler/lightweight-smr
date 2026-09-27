// bootstrap: worker dispatch + initial paint
import { S, $ } from './state.js';
import { onMessage, respawn } from './engine.js';
import { update, draw, setView } from './render.js';
import { loadTrace } from './trace.js';
import { downloadJson } from './form.js';
import { onLiveRound, updateLivePlayBtn } from './live.js';
import './controls.js';
import { onSmrDone, onSmrReady, onSmrRound, onSmrInjected, onSmrScenario, onSmrTrafficSet, onSmrCaptured, onSmrVerify, onSmrNodeDetail, onSmrBlocked, onSmrFailed, onSmrAbort } from './smr.js';
import { onSpillReady, onSpillChain } from './spill.js';

if (globalThis.location?.hostname.endsWith('github.io')) $('hostNote').classList.remove('hidden');

onMessage(m => {
  if (m.type === 'ready') { S.engine = 'wasm'; $('engineChip').textContent = 'engine: Rust WASM — canonical seeds'; $('engineChip').className = 'chip chip-ok engineChip'; }
  else if (m.type === 'initfail') { S.engine = 'failed'; $('engineChip').textContent = 'engine failed: ' + m.error + ' — serve via http, not file://'; $('engineChip').className = 'chip chip-dead engineChip'; }
  else if (m.type === 'done') {
    const out = JSON.parse(m.json);
    S.report = out.report;
    loadTrace(out.trace);
    setView('replay');
    $('runStatus').textContent = 'done: ' + S.trace.rounds.length + ' rounds in ' + m.ms + ' ms';
  }
  else if (m.type === 'live-ready') {
    loadTrace(JSON.parse(m.json));
    S.live = { playing: false, awaiting: false, bg: S.pendingLiveBg || 0 };
    $('liveCol').classList.remove('hidden');
    $('liveRound').textContent = 'r 0';
    updateLivePlayBtn();
    setView('replay');
    $('runStatus').textContent = 'live session running — use the β slider';
  }
  else if (m.type === 'live-round') { onLiveRound(JSON.parse(m.json)); }
  else if (m.type === 'live-scenario') { downloadJson('live-scenario.json', m.json); }
  else if (m.type === 'live-tracejson') { downloadJson('live-trace.json', m.json); }
  else if (m.type === 'smr-done') { onSmrDone(m); }
  else if (m.type === 'smr-failed') { onSmrFailed(m); }
  else if (m.type === 'smr-live-ready') { onSmrReady(m); }
  else if (m.type === 'smr-round') { onSmrRound(m); }
  else if (m.type === 'smr-injected') { onSmrInjected(m); }
  else if (m.type === 'smr-scenario') { onSmrScenario(m); }
  else if (m.type === 'smr-traffic-set') { onSmrTrafficSet(m); }
  else if (m.type === 'smr-captured') { onSmrCaptured(m); }
  else if (m.type === 'smr-verify') { onSmrVerify(m); }
  else if (m.type === 'smr-node-detail') { onSmrNodeDetail(m); }
  else if (m.type === 'smr-blocked') { onSmrBlocked(m); }
  else if (m.type === 'spill-ready') { onSpillReady(m); }
  else if (m.type === 'spill-chain') { onSpillChain(m); }
  else if (m.type === 'error') {
    if (S.live) { S.live.playing = false; S.live.awaiting = false; updateLivePlayBtn(); }
    if (S.smr && S.smr.live) { S.smr.live.playing = false; S.smr.live.awaiting = false; }
    $('runStatus').textContent = 'error: ' + m.error;
    $('smrStatus').textContent = 'error: ' + m.error;
    if (m.panic) {
      // Post-trap wasm memory is untrustworthy, so both sessions die with it.
      onSmrAbort(m);
      S.live = null;
      $('liveCol').classList.add('hidden');
      respawn();
    }
  }
});

window.addEventListener('resize', draw);
draw();
