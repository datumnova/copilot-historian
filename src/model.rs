use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Default, Clone, Serialize)]
pub struct ModelUsage {
    pub requests: u64,
    pub premium_cost: f64,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub reasoning: u64,
    pub aiu: f64,
}

#[derive(Clone, Serialize)]
pub struct Turn {
    pub role: u8, // 0 user, 1 assistant, 2 task summary
    pub ts: String,
    pub model: Option<String>,
    pub tools: Vec<String>,
    pub text: String,
}

#[derive(Clone, Serialize)]
pub struct Segment {
    pub ts: String,
    pub shutdown_type: String,
    pub premium: f64,
    pub aiu: f64,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub api_ms: u64,
}

#[derive(Default, Clone, Serialize)]
pub struct Session {
    pub id: String,
    pub name: String,
    pub cwd: String,
    pub git_root: Option<String>,
    pub repository: Option<String>,
    pub branch: Option<String>,
    pub host_type: Option<String>,
    pub client: Option<String>,
    pub copilot_version: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub active: bool,
    pub user_turns: u64,
    pub assistant_msgs: u64,
    pub tool_calls: u64,
    pub tool_counts: BTreeMap<String, u64>,
    pub models: BTreeMap<String, ModelUsage>,
    pub models_seen: BTreeSet<String>,
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub premium: f64,
    pub aiu: f64,
    pub api_ms: u64,
    pub wall_ms: i64,
    pub lines_added: u64,
    pub lines_removed: u64,
    pub files: BTreeSet<String>,
    pub summaries: Vec<String>,
    pub compactions: u32,
    pub subagents: u32,
    pub errors: u32,
    pub segments: u32,
    pub segments_detail: Vec<Segment>,
    pub events_bytes: u64,
    #[serde(skip)]
    pub turns: Vec<Turn>,
}

impl Session {
    /// Grouping label: GitHub repo when known, otherwise the last path
    /// component of the working directory.
    pub fn repo_label(&self) -> String {
        if let Some(r) = &self.repository {
            if !r.is_empty() {
                return r.clone();
            }
        }
        let c = self.cwd.trim_end_matches(['\\', '/']);
        if c.is_empty() {
            return "(unknown)".into();
        }
        let last = c.rsplit(['\\', '/']).next().unwrap_or(c);
        format!("dir:{last}")
    }

    pub fn total_tokens(&self) -> u64 {
        self.input + self.output + self.cache_read
    }
}
