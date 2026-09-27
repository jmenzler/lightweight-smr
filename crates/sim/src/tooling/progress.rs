//! Mid-run progress status file; never touches stdout, the CSV, the ledger or the RNG stream.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub struct ProgressFile {
    path: PathBuf,
    min_interval: Duration,
    state: Mutex<Inner>,
}

struct Inner {
    cells: BTreeMap<String, CellState>,
    last_write: Option<Instant>,
}

struct CellState {
    round: u64,
    max_rounds: u64,
    started: Instant,
    elapsed_s: f64,
    done: bool,
}

impl ProgressFile {
    pub fn new(path: PathBuf) -> Self {
        Self::with_interval(path, Duration::from_secs(5))
    }

    fn with_interval(path: PathBuf, min_interval: Duration) -> Self {
        ProgressFile {
            path,
            min_interval,
            state: Mutex::new(Inner {
                cells: BTreeMap::new(),
                last_write: None,
            }),
        }
    }

    pub fn tick(&self, cell: &str, round: u64, max_rounds: u64) {
        let mut inner = self.state.lock().expect("progress lock");
        let now = Instant::now();
        let entry = inner
            .cells
            .entry(cell.to_string())
            .or_insert_with(|| CellState {
                round: 0,
                max_rounds,
                started: now,
                elapsed_s: 0.0,
                done: false,
            });
        entry.round = round;
        entry.elapsed_s = now.duration_since(entry.started).as_secs_f64();
        let due = inner
            .last_write
            .is_none_or(|w| now.duration_since(w) >= self.min_interval);
        if due {
            self.write(&mut inner, now);
        }
    }

    pub fn done(&self, cell: &str) {
        let mut inner = self.state.lock().expect("progress lock");
        let now = Instant::now();
        if let Some(entry) = inner.cells.get_mut(cell) {
            entry.done = true;
            entry.elapsed_s = now.duration_since(entry.started).as_secs_f64();
        }
        self.write(&mut inner, now);
    }

    // Failures are swallowed by design: observability must never take down a run.
    fn write(&self, inner: &mut Inner, now: Instant) {
        let cells: serde_json::Map<String, serde_json::Value> = inner
            .cells
            .iter()
            .map(|(k, c)| {
                (
                    k.clone(),
                    serde_json::json!({
                        "round": c.round,
                        "max_rounds": c.max_rounds,
                        "elapsed_s": (c.elapsed_s * 10.0).round() / 10.0,
                        "done": c.done,
                    }),
                )
            })
            .collect();
        let body = serde_json::json!({ "cells": cells }).to_string();
        let tmp = self.path.with_extension("tmp");
        if std::fs::write(&tmp, body).is_ok() && std::fs::rename(&tmp, &self.path).is_ok() {
            inner.last_write = Some(now);
        }
    }
}

static SINK: OnceLock<ProgressFile> = OnceLock::new();
static RUN_ID: AtomicU64 = AtomicU64::new(0);

pub fn activate(path: PathBuf) {
    let _ = SINK.set(ProgressFile::new(path));
}

pub struct ProgressCell {
    key: Option<String>,
    max_rounds: u64,
}

pub(crate) fn cell(n: usize, seed: u64, max_rounds: usize) -> ProgressCell {
    let key = SINK.get().map(|_| {
        let id = RUN_ID.fetch_add(1, Ordering::Relaxed);
        format!("run{id}-n{n}-s{seed}")
    });
    ProgressCell {
        key,
        max_rounds: max_rounds as u64,
    }
}

impl ProgressCell {
    pub(crate) fn tick(&self, round: usize) {
        if let (Some(key), Some(sink)) = (&self.key, SINK.get()) {
            sink.tick(key, round as u64, self.max_rounds);
        }
    }

    pub(crate) fn done(&self) {
        if let (Some(key), Some(sink)) = (&self.key, SINK.get()) {
            sink.done(key);
        }
    }
}

#[cfg(test)]
mod tests;
