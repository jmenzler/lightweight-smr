//! The simulated world's round step, stages (b)–(e) plus recovery's boundary pass: client
//! stage, amplify, log requests, node steps. Stages (a0) arrivals and (a) masks are drawn by
//! `SmrState` itself. Everything here may draw from the run's RNG or change a node; the stage
//! order is the RNG contract. Each step function ends by calling into `observe` for stage (f).

pub(super) mod engine_comp;
pub(super) mod engine_ext;
pub(super) mod engine_rec;
pub(super) mod pool;
pub(super) mod stages;
