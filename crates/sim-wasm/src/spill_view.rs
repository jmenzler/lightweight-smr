use crate::helpers::chain_snapshot;
use wasm_bindgen::prelude::wasm_bindgen;

/// A parsed canonical spill: the lab's view of a lean recovery run.
#[wasm_bindgen]
pub struct SpillView {
    spill: sim::smr::Spill,
    verify_error: Option<String>,
}

#[wasm_bindgen]
impl SpillView {
    #[wasm_bindgen(constructor)]
    pub fn new(text: &str) -> Result<SpillView, String> {
        let spill = sim::smr::Spill::parse(text)?;
        let verify_error = spill.verify().err();
        Ok(SpillView {
            spill,
            verify_error,
        })
    }

    /// Provenance, per-boundary layout, and verification status for the viewer.
    pub fn summary_json(&self) -> String {
        let boundaries: Vec<serde_json::Value> = self
            .spill
            .boundaries
            .iter()
            .scan(0u64, |committed, b| {
                *committed += b.appended.len() as u64;
                Some(serde_json::json!({
                    "w": b.w,
                    "round": b.round,
                    "release": b.release,
                    "digest": b.digest,
                    "committed": *committed,
                    "lens": b.lens,
                    "offsets": b.offsets,
                }))
            })
            .collect();
        serde_json::json!({
            "commit": self.spill.header.commit,
            "dirty": self.spill.header.dirty,
            "seed": self.spill.header.seed,
            "n": self.spill.header.n,
            "t_window": self.spill.header.t_window,
            "proto": self.spill.header.proto,
            "verified": self.verify_error.is_none(),
            "verify_error": self.verify_error,
            "complete": self.spill.trailer.is_some(),
            "diverged": self.spill.trailer.as_ref().map(|t| t.diverged),
            "boundaries": boundaries,
        })
        .to_string()
    }

    /// The recovery chain forest over one node's reconstructed view at one boundary.
    pub fn chain_json(&self, boundary: usize, node: usize, from_len: u64) -> String {
        match self.spill.node_view(boundary, node) {
            Ok(entries) => chain_snapshot(&entries, from_len),
            Err(error) => serde_json::json!({ "error": error }).to_string(),
        }
    }
}
