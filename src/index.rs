use crate::model::Session;
use std::collections::BTreeMap;

/// A searchable document: a conversation turn, or (turn == -1) a metadata
/// document covering session name/id/repo/cwd/models/files.
#[derive(Clone, Copy)]
pub struct DocRef {
    pub session: u32,
    pub turn: i32,
}

pub struct SearchIndex {
    postings: BTreeMap<String, Vec<u32>>,
    pub docs: Vec<DocRef>,
}

pub fn tokenize(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for c in text.chars() {
        if c.is_alphanumeric() || c == '_' {
            for lc in c.to_lowercase() {
                cur.push(lc);
            }
            if cur.len() >= 40 {
                out.push(std::mem::take(&mut cur));
            }
        } else if !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out.retain(|t| t.len() >= 2);
    out
}

impl SearchIndex {
    pub fn build(sessions: &[Session]) -> Self {
        let mut postings: BTreeMap<String, Vec<u32>> = BTreeMap::new();
        let mut docs: Vec<DocRef> = Vec::new();
        for (si, s) in sessions.iter().enumerate() {
            let meta_id = docs.len() as u32;
            docs.push(DocRef {
                session: si as u32,
                turn: -1,
            });
            for tok in tokenize(&meta_text(s)) {
                postings.entry(tok).or_default().push(meta_id);
            }
            for (ti, t) in s.turns.iter().enumerate() {
                let doc_id = docs.len() as u32;
                docs.push(DocRef {
                    session: si as u32,
                    turn: ti as i32,
                });
                for tok in tokenize(&t.text) {
                    postings.entry(tok).or_default().push(doc_id);
                }
            }
        }
        for v in postings.values_mut() {
            v.sort_unstable();
            v.dedup();
        }
        SearchIndex { postings, docs }
    }

    /// AND query; the final token also matches by prefix so incremental
    /// typing works. Returns matching doc ids.
    pub fn query(&self, q: &str) -> Vec<u32> {
        let toks = tokenize(q);
        if toks.is_empty() {
            return Vec::new();
        }
        let mut lists: Vec<Vec<u32>> = Vec::new();
        let last = toks.len() - 1;
        for (i, t) in toks.iter().enumerate() {
            if i == last {
                // prefix union, capped to keep worst-case bounded
                let mut merged: Vec<u32> = Vec::new();
                for (key, ids) in self.postings.range(t.clone()..) {
                    if !key.starts_with(t.as_str()) {
                        break;
                    }
                    merged.extend_from_slice(ids);
                    if merged.len() > 400_000 {
                        break;
                    }
                }
                merged.sort_unstable();
                merged.dedup();
                lists.push(merged);
            } else {
                lists.push(self.postings.get(t).cloned().unwrap_or_default());
            }
        }
        if lists.iter().any(|l| l.is_empty()) {
            return Vec::new();
        }
        lists.sort_by_key(|l| l.len());
        let mut acc = lists[0].clone();
        for l in &lists[1..] {
            acc = intersect(&acc, l);
            if acc.is_empty() {
                break;
            }
        }
        acc
    }

    /// Session indices (into the sessions vec) that contain all query tokens
    /// in any document.
    pub fn matching_sessions(&self, q: &str) -> Vec<u32> {
        let mut out: Vec<u32> = self
            .query(q)
            .iter()
            .map(|&d| self.docs[d as usize].session)
            .collect();
        out.sort_unstable();
        out.dedup();
        out
    }
}

fn intersect(a: &[u32], b: &[u32]) -> Vec<u32> {
    let mut out = Vec::with_capacity(a.len().min(b.len()));
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        match a[i].cmp(&b[j]) {
            std::cmp::Ordering::Less => i += 1,
            std::cmp::Ordering::Greater => j += 1,
            std::cmp::Ordering::Equal => {
                out.push(a[i]);
                i += 1;
                j += 1;
            }
        }
    }
    out
}

pub fn meta_text(s: &Session) -> String {
    let mut parts: Vec<String> = vec![
        s.name.clone(),
        s.id.clone(),
        s.cwd.clone(),
        s.repo_label(),
    ];
    if let Some(v) = &s.repository {
        parts.push(v.clone());
    }
    if let Some(v) = &s.branch {
        parts.push(v.clone());
    }
    if let Some(v) = &s.client {
        parts.push(v.clone());
    }
    parts.extend(s.models_seen.iter().cloned());
    parts.extend(s.files.iter().cloned());
    parts.join("\n")
}
