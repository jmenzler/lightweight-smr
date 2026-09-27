# sim

Deterministic round simulator for the protocol in `crates/protocol`. A run is fixed by its
scenario and seed: one seeded RNG stream feeds every random choice, and the stages of a round
draw from it in a fixed order. Same scenario, same seed, same bytes out.

The code splits into three parts: the simulated world, the observers that measure it, and the
tooling that runs experiments.

## World

Everything that changes a node's state or draws from the run's RNG.

- `src/consensus.rs`, `src/consensus/blocking.rs` – the single-value simulator
  (Algorithm 1) and the blocking schedules both simulators use.
- `src/smr.rs` – `SmrState`: the SMR run loop, stage (a0) traffic arrivals and stage (a)
  blocking masks, and the read-only accessors drivers and the lab call between rounds.
- `src/smr/engine/` – one round step per protocol (`engine_ext` Algorithm 3, `engine_comp`
  Algorithm 5, `engine_rec` Algorithm 6 with its between-window pass), the stages they share
  (`stages`: amplify, node applies) and the bounded client pool (`pool`).

The stage order (a0), (a)–(e) is the RNG contract: reordering stages changes what each
seed draws.

## Observers

Read-only measurement in `src/smr/observe/`. Nothing here draws from the RNG or mutates a node:
the executed-prefix safety check (`safety`), the canonical committed order and its digest
chain (`canonical`, `canon_cursor`, `entry_index`), log inversion for landmarks (`log_index`),
the per-round metrics row (`round_metrics`), census counters (`census_count`), memory
accounting (`accounting`), the canonical spill file (`spill`) and the lab's node detail
(`detail`).

`src/smr/tracker.rs` sits between the two. A command tracker holds the client's retry state,
which the client stage reads, and the landmarks (delivered, in all logs, prefix fixed,
committed, executed) the observation pass writes.

Two observer outputs reach the world on purpose:

- In recovery, the observer decides a command's `executed_round`. That frees a pool slot, and
  without acks it stops the client resending. Both change later RNG draws.
- In lean runs, the cursor floor and the safety latch bound how far nodes forget their
  committed prefix. Forgetting drops entries a node no longer needs; it must not change a
  protocol decision.

### Inertness test

`tests/regression/observer_inertness.rs` runs the same seeded compact and recovery scenarios
with every observer on (full history, census, §5 certificates, every accessor each round),
lean with spill, and grid-lean with the census off. After every round it compares each node's
log, executed sequence, committed sequence numbers and recovery checkpoint, and at the end the
metrics rows and every command's landmarks and RNG-drawn targets. An observer that draws once
from the RNG or writes world state fails it within a round or two of the first effect.

Narrower pins: `tests/regression/smr_lean_report.rs` (lean equals full on reports),
`tests/smr_census_scope.rs` (census on equals off), `tests/smr_cert_gating.rs` (certificates
on equals off).

## Tooling

`src/tooling/` – scenario specs, grid sweeps (`sweep_grid`), the run ledger
(`.runs/sim-runs.jsonl`), analysis and probes. Runners are in `src/bin/` and `examples/`.
They run the world through `run_smr*` and `SmrState` and report what the observers measured.

## Tests

```bash
cargo test -p sim                                    # all of it
cargo test -p sim --test regression observer_inertness
```

Integration tests are grouped into `tests/paper/` (claims stated in the paper's terms),
`tests/support/` (data structures and drivers) and `tests/regression/` (pins against
behaviour changes). Files that touch process-global state stay single-file binaries in
`tests/`.
