//! Networked node for the (6,3)-median-rule protocol suite: protocol cores behind a sans-IO round engine.

pub mod cli;
pub mod collect;
pub mod deploy;
pub mod digest;
pub mod engine;
pub mod harness;
pub mod net;
pub mod record;
pub mod rng;
pub mod schedule;
pub mod spec;
pub mod wire;

pub type NodeId = u32;
