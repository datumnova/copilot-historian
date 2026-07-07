use crate::index::SearchIndex;
use crate::model::Session;
use crate::util::{snippet, truncate_chars};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};

pub struct AppState {
    pub sessions: Vec<Session>,
    pub index: SearchIndex,
    pub root: String,
    pub scanned_at: u64,
    pub scan_ms: u64,
}

impl AppState {
    pub fn by_id(&self, id: &str) -> Option<(usize, &Session)> {
        self.sessions
            .iter()
            .enumerate()
            .find(|(_, s)| s.id == id || s.id.starts_with(id))
    }
}

pub fn parse_query(url: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Some((_, qs)) = url.split_once('?') else {
        return out;
    };
    for pair in qs.split('&') {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        out.insert(percent_decode(k), percent_decode(v));
    }
    out
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < b.len() => {
                let hex = std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or("");
                if let Ok(v) = u8::from_str_radix(hex, 16) {
                    out.push(v);
                    i += 3;
                } else {
                    out.push(b[i]);
                    i += 1;
                }
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn r3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

fn date_of(ts: &str) -> Option<&str> {
    if ts.len() >= 10 {
        Some(&ts[..10])
    } else {
        None
    }
}

// ---------------------------------------------------------------- overview

pub fn overview(st: &AppState) -> Value {
    #[derive(Default)]
    struct Day {
        sessions: u64,
        user_turns: u64,
        input: u64,
        output: u64,
        cache_read: u64,
        premium: f64,
        aiu: f64,
    }
    let mut days: BTreeMap<String, Day> = BTreeMap::new();
    let mut models: BTreeMap<String, (u64, crate::model::ModelUsage)> = BTreeMap::new();
    let mut tools: BTreeMap<String, u64> = BTreeMap::new();
    let mut clients: BTreeMap<String, u64> = BTreeMap::new();
    let (mut input, mut output, mut cache_read) = (0u64, 0u64, 0u64);
    let (mut premium, mut aiu) = (0f64, 0f64);
    let (mut user_turns, mut assistant_msgs, mut tool_calls) = (0u64, 0u64, 0u64);
    let (mut api_ms, mut la, mut lr, mut files, mut active) = (0u64, 0u64, 0u64, 0u64, 0u64);
    let (mut subagents, mut errors) = (0u64, 0u64);

    for s in &st.sessions {
        input += s.input;
        output += s.output;
        cache_read += s.cache_read;
        premium += s.premium;
        aiu += s.aiu;
        user_turns += s.user_turns;
        assistant_msgs += s.assistant_msgs;
        tool_calls += s.tool_calls;
        api_ms += s.api_ms;
        la += s.lines_added;
        lr += s.lines_removed;
        files += s.files.len() as u64;
        subagents += s.subagents as u64;
        errors += s.errors as u64;
        if s.active {
            active += 1;
        }
        *clients
            .entry(s.client.clone().unwrap_or_else(|| "unknown".into()))
            .or_insert(0) += 1;
        if let Some(d) = date_of(&s.created_at) {
            days.entry(d.to_string()).or_default().sessions += 1;
        }
        for t in &s.turns {
            if t.role == 0 {
                if let Some(d) = date_of(&t.ts) {
                    days.entry(d.to_string()).or_default().user_turns += 1;
                }
            }
        }
        for seg in &s.segments_detail {
            if let Some(d) = date_of(&seg.ts) {
                let e = days.entry(d.to_string()).or_default();
                e.input += seg.input;
                e.output += seg.output;
                e.cache_read += seg.cache_read;
                e.premium += seg.premium;
                e.aiu += seg.aiu;
            }
        }
        for (name, mu) in &s.models {
            let e = models.entry(name.clone()).or_default();
            e.0 += 1;
            e.1.requests += mu.requests;
            e.1.premium_cost += mu.premium_cost;
            e.1.input += mu.input;
            e.1.output += mu.output;
            e.1.cache_read += mu.cache_read;
            e.1.cache_write += mu.cache_write;
            e.1.reasoning += mu.reasoning;
            e.1.aiu += mu.aiu;
        }
        for (name, c) in &s.tool_counts {
            *tools.entry(name.clone()).or_insert(0) += c;
        }
    }

    let mut model_rows: Vec<Value> = models
        .iter()
        .map(|(name, (sess, m))| {
            json!({"name": name, "sessions": sess, "requests": m.requests,
                   "premium": r3(m.premium_cost), "input": m.input, "output": m.output,
                   "cache_read": m.cache_read, "reasoning": m.reasoning, "aiu": r3(m.aiu)})
        })
        .collect();
    model_rows.sort_by(|a, b| {
        b["aiu"]
            .as_f64()
            .partial_cmp(&a["aiu"].as_f64())
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut tool_rows: Vec<Value> = tools
        .iter()
        .map(|(n, c)| json!({"name": n, "count": c}))
        .collect();
    tool_rows.sort_by_key(|v| std::cmp::Reverse(v["count"].as_u64().unwrap_or(0)));

    json!({
        "root": st.root,
        "scanned_at": st.scanned_at,
        "scan_ms": st.scan_ms,
        "totals": {
            "sessions": st.sessions.len(), "active": active,
            "user_turns": user_turns, "assistant_msgs": assistant_msgs, "tool_calls": tool_calls,
            "input": input, "output": output, "cache_read": cache_read,
            "premium": r3(premium), "aiu": r3(aiu), "api_ms": api_ms,
            "lines_added": la, "lines_removed": lr, "files": files,
            "subagents": subagents, "errors": errors
        },
        "models": model_rows,
        "daily": days.iter().map(|(d, v)| json!({
            "date": d, "sessions": v.sessions, "user_turns": v.user_turns,
            "input": v.input, "output": v.output, "cache_read": v.cache_read,
            "premium": r3(v.premium), "aiu": r3(v.aiu)
        })).collect::<Vec<_>>(),
        "repos": repo_rows(st),
        "clients": clients.iter().map(|(n, c)| json!({"name": n, "sessions": c})).collect::<Vec<_>>(),
        "tools": tool_rows,
    })
}

fn repo_rows(st: &AppState) -> Vec<Value> {
    #[derive(Default)]
    struct R {
        sessions: u64,
        input: u64,
        output: u64,
        cache_read: u64,
        premium: f64,
        aiu: f64,
        user_turns: u64,
        lines_added: u64,
        lines_removed: u64,
        last: String,
        branches: BTreeMap<String, u64>,
        cwds: BTreeMap<String, u64>,
        top: Vec<(f64, String, String)>,
    }
    let mut repos: BTreeMap<String, R> = BTreeMap::new();
    for s in &st.sessions {
        let r = repos.entry(s.repo_label()).or_default();
        r.sessions += 1;
        r.input += s.input;
        r.output += s.output;
        r.cache_read += s.cache_read;
        r.premium += s.premium;
        r.aiu += s.aiu;
        r.user_turns += s.user_turns;
        r.lines_added += s.lines_added;
        r.lines_removed += s.lines_removed;
        if s.updated_at > r.last {
            r.last = s.updated_at.clone();
        }
        if let Some(b) = &s.branch {
            *r.branches.entry(b.clone()).or_insert(0) += 1;
        }
        *r.cwds.entry(s.cwd.clone()).or_insert(0) += 1;
        r.top.push((s.aiu, s.id.clone(), s.name.clone()));
    }
    let mut rows: Vec<Value> = repos
        .iter_mut()
        .map(|(name, r)| {
            r.top
                .sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
            json!({
                "name": name, "sessions": r.sessions, "input": r.input, "output": r.output,
                "cache_read": r.cache_read, "premium": r3(r.premium), "aiu": r3(r.aiu),
                "user_turns": r.user_turns, "lines_added": r.lines_added, "lines_removed": r.lines_removed,
                "last": r.last,
                "branches": r.branches.keys().cloned().collect::<Vec<_>>(),
                "cwds": r.cwds.keys().cloned().collect::<Vec<_>>(),
                "top_sessions": r.top.iter().take(3).map(|(a, id, n)| json!({"id": id, "name": n, "aiu": r3(*a)})).collect::<Vec<_>>()
            })
        })
        .collect();
    rows.sort_by(|a, b| {
        b["aiu"]
            .as_f64()
            .partial_cmp(&a["aiu"].as_f64())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    rows
}

pub fn repos(st: &AppState) -> Value {
    json!({"repos": repo_rows(st)})
}

// ---------------------------------------------------------------- sessions

fn session_card(s: &Session) -> Value {
    json!({
        "id": s.id, "name": s.name, "repo": s.repo_label(),
        "repository": s.repository, "branch": s.branch, "cwd": s.cwd,
        "client": s.client, "created_at": s.created_at, "updated_at": s.updated_at,
        "active": s.active, "user_turns": s.user_turns, "tool_calls": s.tool_calls,
        "models": s.models_seen.iter().cloned().collect::<Vec<_>>(),
        "input": s.input, "output": s.output, "cache_read": s.cache_read,
        "total_tokens": s.total_tokens(), "premium": r3(s.premium), "aiu": r3(s.aiu),
        "wall_ms": s.wall_ms, "api_ms": s.api_ms,
        "lines_added": s.lines_added, "lines_removed": s.lines_removed,
        "files": s.files.len(), "errors": s.errors, "subagents": s.subagents,
        "has_usage": s.segments > 0,
        "summary": s.summaries.first().map(|x| truncate_chars(x, 280)),
    })
}

pub fn sessions(st: &AppState, q: &HashMap<String, String>) -> Value {
    let empty = String::new();
    let text = q.get("q").unwrap_or(&empty).trim();
    let repo = q.get("repo").unwrap_or(&empty).trim();
    let model = q.get("model").unwrap_or(&empty).trim();
    let from = q.get("from").unwrap_or(&empty).trim();
    let to = q.get("to").unwrap_or(&empty).trim();
    let sort = q.get("sort").map(|s| s.as_str()).unwrap_or("updated");
    let asc = q.get("dir").map(|d| d == "asc").unwrap_or(false);
    let limit: usize = q.get("limit").and_then(|v| v.parse().ok()).unwrap_or(100);
    let offset: usize = q.get("offset").and_then(|v| v.parse().ok()).unwrap_or(0);

    let allowed: Option<Vec<u32>> = if text.is_empty() {
        None
    } else {
        Some(st.index.matching_sessions(text))
    };

    let mut idxs: Vec<usize> = st
        .sessions
        .iter()
        .enumerate()
        .filter(|(i, s)| {
            if let Some(a) = &allowed {
                if a.binary_search(&(*i as u32)).is_err() {
                    return false;
                }
            }
            if !repo.is_empty() && s.repo_label() != repo {
                return false;
            }
            if !model.is_empty() && !s.models_seen.contains(model) {
                return false;
            }
            let d = date_of(&s.updated_at).unwrap_or("");
            if !from.is_empty() && d < from {
                return false;
            }
            if !to.is_empty() && d > to {
                return false;
            }
            true
        })
        .map(|(i, _)| i)
        .collect();

    idxs.sort_by(|&a, &b| {
        let (sa, sb) = (&st.sessions[a], &st.sessions[b]);
        let ord = match sort {
            "created" => sa.created_at.cmp(&sb.created_at),
            "tokens" => sa.total_tokens().cmp(&sb.total_tokens()),
            "aiu" => sa
                .aiu
                .partial_cmp(&sb.aiu)
                .unwrap_or(std::cmp::Ordering::Equal),
            "premium" => sa
                .premium
                .partial_cmp(&sb.premium)
                .unwrap_or(std::cmp::Ordering::Equal),
            "turns" => sa.user_turns.cmp(&sb.user_turns),
            "duration" => sa.wall_ms.cmp(&sb.wall_ms),
            _ => sa.updated_at.cmp(&sb.updated_at),
        };
        if asc {
            ord
        } else {
            ord.reverse()
        }
    });

    let total = idxs.len();
    let items: Vec<Value> = idxs
        .iter()
        .skip(offset)
        .take(limit)
        .map(|&i| session_card(&st.sessions[i]))
        .collect();
    json!({"total": total, "items": items})
}

pub fn session_detail(st: &AppState, id: &str) -> Option<Value> {
    let (_, s) = st.by_id(id)?;
    let turns: Vec<Value> = s
        .turns
        .iter()
        .map(|t| {
            let truncated = t.text.chars().count() > 8000;
            json!({
                "role": t.role, "ts": t.ts, "model": t.model, "tools": t.tools,
                "text": truncate_chars(&t.text, 8000), "truncated": truncated
            })
        })
        .collect();
    let mut card = session_card(s);
    let extra = json!({
        "git_root": s.git_root, "host_type": s.host_type,
        "copilot_version": s.copilot_version,
        "compactions": s.compactions, "segments": s.segments,
        "events_bytes": s.events_bytes,
        "models_detail": s.models.iter().map(|(n, m)| json!({
            "name": n, "requests": m.requests, "premium": r3(m.premium_cost),
            "input": m.input, "output": m.output, "cache_read": m.cache_read,
            "cache_write": m.cache_write, "reasoning": m.reasoning, "aiu": r3(m.aiu)
        })).collect::<Vec<_>>(),
        "tool_counts": s.tool_counts,
        "files_list": s.files.iter().cloned().collect::<Vec<_>>(),
        "summaries": s.summaries,
        "segments_detail": s.segments_detail,
        "turns": turns,
    });
    if let (Some(a), Some(b)) = (card.as_object_mut(), extra.as_object()) {
        for (k, v) in b {
            a.insert(k.clone(), v.clone());
        }
    }
    Some(card)
}

// ---------------------------------------------------------------- search

pub fn search(st: &AppState, q: &HashMap<String, String>) -> Value {
    let empty = String::new();
    let text = q.get("q").unwrap_or(&empty).trim();
    let limit: usize = q.get("limit").and_then(|v| v.parse().ok()).unwrap_or(80);
    let offset: usize = q.get("offset").and_then(|v| v.parse().ok()).unwrap_or(0);
    let role_filter: Option<u8> = q.get("role").and_then(|v| v.parse().ok());
    if text.is_empty() {
        return json!({"total": 0, "hits": [], "tokens": []});
    }
    let toks = crate::index::tokenize(text);
    let mut doc_ids = st.index.query(text);
    // newest first: sort by the timestamp of the underlying doc
    doc_ids.sort_by(|&a, &b| {
        let ta = doc_ts(st, a);
        let tb = doc_ts(st, b);
        tb.cmp(ta)
    });
    let filtered: Vec<u32> = doc_ids
        .into_iter()
        .filter(|&d| {
            let dr = st.index.docs[d as usize];
            match role_filter {
                None => true,
                Some(rf) => {
                    if dr.turn < 0 {
                        rf == 3
                    } else {
                        st.sessions[dr.session as usize].turns[dr.turn as usize].role == rf
                    }
                }
            }
        })
        .collect();
    let total = filtered.len();
    let hits: Vec<Value> = filtered
        .iter()
        .skip(offset)
        .take(limit)
        .map(|&d| {
            let dr = st.index.docs[d as usize];
            let s = &st.sessions[dr.session as usize];
            if dr.turn < 0 {
                let meta = crate::index::meta_text(s);
                json!({
                    "session_id": s.id, "session_name": s.name, "repo": s.repo_label(),
                    "ts": s.updated_at, "role": 3, "model": null, "turn": -1,
                    "snippet": snippet(&meta, &toks, 140)
                })
            } else {
                let t = &s.turns[dr.turn as usize];
                json!({
                    "session_id": s.id, "session_name": s.name, "repo": s.repo_label(),
                    "ts": t.ts, "role": t.role, "model": t.model, "turn": dr.turn,
                    "snippet": snippet(&t.text, &toks, 140)
                })
            }
        })
        .collect();
    json!({"total": total, "hits": hits, "tokens": toks})
}

fn doc_ts(st: &AppState, doc: u32) -> &str {
    let dr = st.index.docs[doc as usize];
    let s = &st.sessions[dr.session as usize];
    if dr.turn < 0 {
        &s.updated_at
    } else {
        &s.turns[dr.turn as usize].ts
    }
}
