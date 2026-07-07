use crate::model::{ModelUsage, Segment, Session, Turn};
use crate::util::iso_to_epoch_ms;
use rayon::prelude::*;
use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::Path;

/// Event types we fully parse. Everything else (notably multi-megabyte
/// tool.execution_* payloads) is skipped via a cheap head-of-line check.
const MARKERS: &[&str] = &[
    "\"session.start\"",
    "\"session.resume\"",
    "\"user.message\"",
    "\"assistant.message\"",
    "\"session.model_change\"",
    "\"session.shutdown\"",
    "\"session.task_complete\"",
    "\"session.compaction_complete\"",
    "\"subagent.started\"",
    "\"session.error\"",
];

pub fn scan_root(root: &Path) -> Vec<Session> {
    let mut dirs: Vec<std::path::PathBuf> = match fs::read_dir(root) {
        Ok(rd) => rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect(),
        Err(_) => Vec::new(),
    };
    dirs.sort();
    let mut sessions: Vec<Session> = dirs
        .par_iter()
        .filter_map(|d| parse_session_dir(d))
        .collect();
    sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    sessions
}

fn parse_session_dir(dir: &Path) -> Option<Session> {
    let id = dir.file_name()?.to_string_lossy().to_string();
    let mut s = Session {
        id,
        ..Default::default()
    };

    let ws = dir.join("workspace.yaml");
    let has_ws = ws.exists();
    if has_ws {
        parse_workspace_yaml(&ws, &mut s);
    }

    let ev = dir.join("events.jsonl");
    let has_ev = ev.exists();
    if has_ev {
        parse_events(&ev, &mut s);
    }
    if !has_ws && !has_ev {
        return None;
    }

    // Active if a live lock file is present.
    s.active = fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok()).any(|e| {
                let n = e.file_name().to_string_lossy().to_string();
                n.starts_with("inuse.") && n.ends_with(".lock")
            })
        })
        .unwrap_or(false);

    if s.name.is_empty() {
        s.name = s
            .turns
            .iter()
            .find(|t| t.role == 0)
            .map(|t| crate::util::truncate_chars(t.text.trim(), 64))
            .unwrap_or_else(|| format!("session {}", &s.id[..8.min(s.id.len())]));
    }
    Some(s)
}

fn parse_workspace_yaml(path: &Path, s: &mut Session) {
    let Ok(text) = fs::read_to_string(path) else {
        return;
    };
    for line in text.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let v = v.trim().trim_matches('"').trim_matches('\'').to_string();
        if v.is_empty() || v == "null" {
            continue;
        }
        match k.trim() {
            "name" => s.name = v,
            "cwd" => s.cwd = v,
            "client_name" => s.client = Some(v),
            "created_at" => s.created_at = v,
            "updated_at" => s.updated_at = v,
            _ => {}
        }
    }
}

fn parse_events(path: &Path, s: &mut Session) {
    let Ok(f) = fs::File::open(path) else {
        return;
    };
    s.events_bytes = f.metadata().map(|m| m.len()).unwrap_or(0);
    let mut r = BufReader::with_capacity(1 << 18, f);
    let mut buf: Vec<u8> = Vec::with_capacity(1 << 16);
    let mut first_ts: Option<String> = None;
    let mut last_ts: Option<String> = None;

    loop {
        buf.clear();
        match r.read_until(b'\n', &mut buf) {
            Ok(0) => break,
            Ok(_) => {}
            Err(_) => break,
        }
        // The "type" key always precedes the (potentially huge) "data" payload,
        // so inspecting the head of the line is a safe pre-filter.
        let head_len = buf.len().min(240);
        let head = String::from_utf8_lossy(&buf[..head_len]);
        if !MARKERS.iter().any(|m| head.contains(m)) {
            continue;
        }
        let line = String::from_utf8_lossy(&buf);
        let Ok(e) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let typ = e["type"].as_str().unwrap_or("");
        let ts = e["timestamp"].as_str().unwrap_or("").to_string();
        if !ts.is_empty() {
            if first_ts.as_deref().map(|f| ts.as_str() < f).unwrap_or(true) {
                first_ts = Some(ts.clone());
            }
            if last_ts.as_deref().map(|l| ts.as_str() > l).unwrap_or(true) {
                last_ts = Some(ts.clone());
            }
        }
        let d = &e["data"];
        match typ {
            "session.start" | "session.resume" => {
                if let Some(v) = d["copilotVersion"].as_str() {
                    s.copilot_version = Some(v.to_string());
                }
                let ctx = &d["context"];
                if let Some(v) = ctx["cwd"].as_str() {
                    if s.cwd.is_empty() {
                        s.cwd = v.to_string();
                    }
                }
                for (key, slot) in [
                    ("gitRoot", &mut s.git_root),
                    ("repository", &mut s.repository),
                    ("branch", &mut s.branch),
                    ("hostType", &mut s.host_type),
                ] {
                    if let Some(v) = ctx[key].as_str() {
                        if !v.is_empty() {
                            *slot = Some(v.to_string());
                        }
                    }
                }
            }
            "user.message" => {
                let text = content_to_text(&d["content"]);
                if !text.trim().is_empty() {
                    s.user_turns += 1;
                    s.turns.push(Turn {
                        role: 0,
                        ts,
                        model: None,
                        tools: vec![],
                        text,
                    });
                }
            }
            "assistant.message" => {
                s.assistant_msgs += 1;
                let model = d["model"].as_str().map(|m| m.to_string());
                if let Some(m) = &model {
                    s.models_seen.insert(m.clone());
                }
                let mut tools: Vec<String> = Vec::new();
                if let Some(reqs) = d["toolRequests"].as_array() {
                    s.tool_calls += reqs.len() as u64;
                    for t in reqs {
                        if let Some(n) = t["name"].as_str() {
                            *s.tool_counts.entry(n.to_string()).or_insert(0) += 1;
                            if !tools.contains(&n.to_string()) {
                                tools.push(n.to_string());
                            }
                        }
                    }
                }
                let text = content_to_text(&d["content"]);
                if !text.trim().is_empty() {
                    s.turns.push(Turn {
                        role: 1,
                        ts,
                        model,
                        tools,
                        text,
                    });
                }
            }
            "session.model_change" => {
                if let Some(m) = d["newModel"].as_str() {
                    s.models_seen.insert(m.to_string());
                }
            }
            "session.task_complete" => {
                if let Some(sum) = d["summary"].as_str() {
                    if !sum.trim().is_empty() {
                        s.summaries.push(sum.to_string());
                        s.turns.push(Turn {
                            role: 2,
                            ts,
                            model: None,
                            tools: vec![],
                            text: sum.to_string(),
                        });
                    }
                }
            }
            "session.compaction_complete" => s.compactions += 1,
            "subagent.started" => s.subagents += 1,
            "session.error" => s.errors += 1,
            "session.shutdown" => apply_shutdown(s, d, ts),
            _ => {}
        }
    }

    if s.created_at.is_empty() {
        if let Some(f) = &first_ts {
            s.created_at = f.clone();
        }
    }
    if s.updated_at.is_empty() {
        if let Some(l) = &last_ts {
            s.updated_at = l.clone();
        }
    }
    if let (Some(f), Some(l)) = (&first_ts, &last_ts) {
        if let (Some(a), Some(b)) = (iso_to_epoch_ms(f), iso_to_epoch_ms(l)) {
            s.wall_ms = (b - a).max(0);
        }
    }
}

/// Each shutdown event reports usage for one run segment; totals are the sum.
fn apply_shutdown(s: &mut Session, d: &Value, ts: String) {
    s.segments += 1;
    let premium = d["totalPremiumRequests"].as_f64().unwrap_or(0.0);
    let aiu = d["totalNanoAiu"].as_f64().unwrap_or(0.0) / 1e9;
    s.premium += premium;
    s.aiu += aiu;
    let api_ms = d["totalApiDurationMs"].as_u64().unwrap_or(0);
    s.api_ms += api_ms;

    let tok = |k: &str| d["tokenDetails"][k]["tokenCount"].as_u64().unwrap_or(0);
    let (inp, out, cr) = (tok("input"), tok("output"), tok("cache_read"));
    s.input += inp;
    s.output += out;
    s.cache_read += cr;

    let cc = &d["codeChanges"];
    s.lines_added += cc["linesAdded"].as_u64().unwrap_or(0);
    s.lines_removed += cc["linesRemoved"].as_u64().unwrap_or(0);
    if let Some(files) = cc["filesModified"].as_array() {
        for f in files {
            if let Some(p) = f.as_str() {
                s.files.insert(p.to_string());
            }
        }
    }

    if let Some(mm) = d["modelMetrics"].as_object() {
        for (name, m) in mm {
            s.models_seen.insert(name.clone());
            let entry = s.models.entry(name.clone()).or_insert_with(ModelUsage::default);
            entry.requests += m["requests"]["count"].as_u64().unwrap_or(0);
            entry.premium_cost += m["requests"]["cost"].as_f64().unwrap_or(0.0);
            let u = &m["usage"];
            entry.input += u["inputTokens"].as_u64().unwrap_or(0);
            entry.output += u["outputTokens"].as_u64().unwrap_or(0);
            entry.cache_read += u["cacheReadTokens"].as_u64().unwrap_or(0);
            entry.cache_write += u["cacheWriteTokens"].as_u64().unwrap_or(0);
            entry.reasoning += u["reasoningTokens"].as_u64().unwrap_or(0);
            entry.aiu += m["totalNanoAiu"].as_f64().unwrap_or(0.0) / 1e9;
        }
    }

    s.segments_detail.push(Segment {
        ts,
        shutdown_type: d["shutdownType"].as_str().unwrap_or("?").to_string(),
        premium,
        aiu,
        input: inp,
        output: out,
        cache_read: cr,
        api_ms,
    });
}

/// Message content is usually a plain string but may be an array of typed parts.
fn content_to_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| {
                p.as_str()
                    .map(|s| s.to_string())
                    .or_else(|| p["text"].as_str().map(|s| s.to_string()))
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}
