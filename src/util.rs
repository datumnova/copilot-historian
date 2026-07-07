/// Parse an ISO-8601 UTC timestamp like `2026-07-06T18:19:22.164Z` into epoch milliseconds.
pub fn iso_to_epoch_ms(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 19 {
        return None;
    }
    let num = |r: std::ops::Range<usize>| -> Option<i64> { s.get(r)?.parse::<i64>().ok() };
    let y = num(0..4)?;
    let mo = num(5..7)?;
    let d = num(8..10)?;
    let h = num(11..13)?;
    let mi = num(14..16)?;
    let se = num(17..19)?;
    let mut ms: i64 = 0;
    if b.len() > 20 && b[19] == b'.' {
        let digits: String = s[20..].chars().take_while(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() {
            let frac: String = format!("{:0<3}", &digits[..digits.len().min(3)]);
            ms = frac.parse().unwrap_or(0);
        }
    }
    let days = days_from_civil(y, mo, d);
    Some((days * 86_400 + h * 3600 + mi * 60 + se) * 1000 + ms)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// Truncate to at most `max` characters, appending an ellipsis when cut.
pub fn truncate_chars(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

fn floor_boundary(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_boundary(s: &str, mut i: usize) -> usize {
    i = i.min(s.len());
    while i < s.len() && !s.is_char_boundary(i) {
        i += 1;
    }
    i
}

/// Extract a snippet of ~2*radius bytes centered on the first occurrence of
/// any of `toks` (case-insensitive). Falls back to the head of the text.
pub fn snippet(text: &str, toks: &[String], radius: usize) -> String {
    let lower = text.to_lowercase();
    let mut pos: Option<usize> = None;
    for t in toks {
        if let Some(p) = lower.find(t.as_str()) {
            pos = Some(match pos {
                Some(cur) => cur.min(p),
                None => p,
            });
        }
    }
    let center = pos.unwrap_or(0);
    // to_lowercase can shift byte offsets for rare non-ASCII; clamp defensively.
    let start = floor_boundary(text, center.saturating_sub(radius));
    let end = ceil_boundary(text, center.saturating_add(radius));
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.push_str(text[start..end].trim());
    if end < text.len() {
        out.push('…');
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn now_epoch_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
