//! The index of Lab runs stored on this device (`labListRuns`, `labDeleteRun`). The
//! runtime keeps the files; the engine keeps their metadata, persisted as JSON under
//! the `lab/index` key and handed back through `EngineConfig::lab_runs`.

use mesh_api::{LabRunInfo, LabSummary};

use crate::effects::{Effect, EffectQueue, LabFile};

/// Persist key of the run index.
pub const INDEX_KEY: &str = "lab/index";
/// At most this many runs are indexed; the oldest entries drop off (their files stay
/// on the device until deleted from the Files app / app storage).
pub const MAX_RUNS: usize = 200;

/// Stored runs, newest first.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RunIndex {
    runs: Vec<LabRunInfo>,
}

impl RunIndex {
    pub fn new(runs: Vec<LabRunInfo>) -> Self {
        let mut r = RunIndex { runs };
        r.runs.truncate(MAX_RUNS);
        r
    }

    pub fn list(&self) -> Vec<LabRunInfo> {
        self.runs.clone()
    }

    pub fn len(&self) -> usize {
        self.runs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.runs.is_empty()
    }

    /// The files of a run (`<runId>.jsonl`, `<runId>-summary.csv`).
    pub fn files(run_id: &str) -> Vec<String> {
        vec![
            LabFile::Jsonl.file_name(run_id),
            LabFile::SummaryCsv.file_name(run_id),
        ]
    }

    /// A run started (its summary is still missing).
    pub fn started(&mut self, info: LabRunInfo, fx: &mut EffectQueue) {
        self.runs.retain(|r| r.run_id != info.run_id);
        self.runs.insert(0, info);
        self.runs.truncate(MAX_RUNS);
        self.persist(fx);
    }

    /// A run finished: duration and verdict are known.
    pub fn finished(&mut self, run_id: &str, s: &LabSummary, fx: &mut EffectQueue) {
        if let Some(r) = self.runs.iter_mut().find(|r| r.run_id == run_id) {
            r.duration_s = Some(s.duration_s);
            r.verdict = Some(s.verdict);
            self.persist(fx);
        }
    }

    /// Deletes a run and its files. `false` when unknown.
    pub fn delete(&mut self, run_id: &str, fx: &mut EffectQueue) -> bool {
        let before = self.runs.len();
        self.runs.retain(|r| r.run_id != run_id);
        if self.runs.len() == before {
            return false;
        }
        fx.push(Effect::LabDelete {
            run_id: run_id.to_string(),
        });
        self.persist(fx);
        true
    }

    fn persist(&self, fx: &mut EffectQueue) {
        if let Ok(bytes) = serde_json::to_vec(&self.runs) {
            fx.push(Effect::Persist {
                key: INDEX_KEY.into(),
                value: Some(bytes),
            });
        }
    }

    /// Parses a persisted index (the runtime calls this to fill `EngineConfig`).
    pub fn parse(bytes: &[u8]) -> Vec<LabRunInfo> {
        serde_json::from_slice(bytes).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mesh_api::{LabAnchor, LabAudio, LabBench, LabRate, LabVerdict};

    fn info(id: &str) -> LabRunInfo {
        LabRunInfo {
            run_id: id.into(),
            bench: LabBench::T0,
            audio: LabAudio::Off,
            anchor: LabAnchor::Audio,
            started_at: "2026-10-23T09:12:03.120Z".into(),
            duration_s: None,
            verdict: None,
            files: RunIndex::files(id),
        }
    }

    #[test]
    fn lifecycle_and_persistence() {
        let mut fx = EffectQueue::new();
        let mut idx = RunIndex::default();
        idx.started(info("r-1"), &mut fx);
        idx.started(info("r-2"), &mut fx);
        let s = LabSummary {
            duration_s: 60.0,
            tx_kbps: LabRate { mean: 1.0 },
            loss: 0.0,
            owd_ms: None,
            rtt_ms: None,
            stalls: 0,
            verdict: LabVerdict::Go,
        };
        idx.finished("r-1", &s, &mut fx);
        assert_eq!(idx.list()[0].run_id, "r-2");
        assert_eq!(idx.list()[1].verdict, Some(LabVerdict::Go));
        let Some(Effect::Persist { value: Some(v), .. }) = fx.iter().last().cloned() else {
            panic!("no persist");
        };
        assert_eq!(RunIndex::parse(&v), idx.list());
        assert!(idx.delete("r-2", &mut fx));
        assert!(!idx.delete("r-2", &mut fx));
        assert_eq!(idx.len(), 1);
    }
}
