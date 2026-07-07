# Copilot Historian

<img src="data:image/svg+xml;utf8,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='%23b11f4b' stroke-width='2.2' stroke-linecap='round' stroke-linejoin='round' width='48'%3E%3Cpath d='M3.5 12a8.5 8.5 0 1 0 2.2-5.7'/%3E%3Cpath d='M3.5 4v4.5H8'/%3E%3Cpath d='M12 7.5V12l3.2 2'/%3E%3C/svg%3E" width="48" align="right">

A self-contained dashboard for exploring your **GitHub Copilot CLI session history** —
full-text search, token & AIU consumption, premium requests, models, repos, summaries,
and complete conversation drill-down.

Single static binary. No runtime dependencies, no database, no network calls.
It reads your local `~/.copilot/session-state` directory (read-only) and serves an
embedded web UI on localhost.

## Quick start

```
cargo build --release
./target/release/copilot-historian      # scans ~/.copilot/session-state, opens browser
```

```
USAGE: copilot-historian [--dir <session-state dir>] [--port <port>] [--no-open]

  --dir, -d    session directory (default: ~/.copilot/session-state)
  --port, -p   listen port (default: 4577)
  --no-open    don't launch the browser
```

The binary is fully portable: copy it to any machine (same OS/arch) and point it at a
session-state directory with `--dir`. Cross-compile with the usual
`cargo build --release --target <triple>` for other platforms.

## What you get

| View | Contents |
|---|---|
| **Overview** | Totals (sessions, prompts, tool calls, input/output/cache tokens, premium requests, AIU, lines of code changed, model API time), daily activity chart, per-model usage table, top repos, tool usage |
| **Sessions** | Filterable/sortable table (full-text, repo, model, date range) with per-session tokens, AIU, premium, code delta |
| **Search** | Indexed full-text search across your prompts, assistant replies, task summaries, and metadata (file paths, repos, model names). Last word matches by prefix; results show highlighted snippets and jump into the conversation |
| **Repos** | Per repo/directory aggregation: sessions, AIU, premium, tokens, prompts, code delta, branches, top sessions |
| **Session drawer** | Full metadata (repo, branch, cwd, client, CLI version, wall span, API time), usage cards, per-model table, run segments, files modified, tools used, task summaries, and the whole conversation |

## How it works

```mermaid
flowchart LR
    subgraph disk["📁 ~/.copilot/session-state — read-only"]
        WS["workspace.yaml<br/><small>name · client · timestamps</small>"]
        EV["events.jsonl<br/><small>messages · git context ·<br/>session.shutdown usage</small>"]
    end
    subgraph bin["⚙️ copilot-historian — single static binary"]
        SC["Parallel scanner<br/><small>rayon · head-of-line type filter<br/>skips MB-scale tool payloads</small>"]
        MO["In-memory model<br/><small>sessions · turns · files<br/>tokens · premium · AIU (nanoAiu ÷ 10⁹)</small>"]
        IX["Inverted search index<br/><small>word → message · AND queries<br/>prefix match on last word</small>"]
        HT["tiny_http server<br/><small>127.0.0.1 only · embedded UI</small>"]
    end
    BR["🌐 Your browser<br/><small>Overview · Sessions · Search · Repos<br/>session drill-down drawer</small>"]
    WS --> SC
    EV --> SC
    SC --> MO
    SC --> IX
    MO --> HT
    IX --> HT
    HT -->|"GET / (embedded UI)<br/>/api/* JSON"| BR
    BR -.->|"⟳ rescan"| SC
```

- Scans every `<session-id>/` directory under the session-state root in parallel (rayon).
- Parses `workspace.yaml` (name, client, timestamps) and `events.jsonl`.
- Only relevant event types are fully parsed (`session.start/resume`, `user.message`,
  `assistant.message`, `session.shutdown`, `session.task_complete`, …). Multi-megabyte
  tool outputs are skipped via a cheap head-of-line check, so ~200 MB of history scans
  in a few seconds.
- Usage data comes from `session.shutdown` events, which the CLI emits **per run
  segment** (each launch/resume). Historian sums segments per session:
  - `tokenDetails` → input / output / cache-read tokens
  - `totalPremiumRequests` → premium requests
  - `totalNanoAiu` / 1e9 → **AIU** (AI units)
  - `modelMetrics` → per-model requests, tokens, reasoning tokens, AIU
  - `codeChanges` → lines added/removed, files modified
- Sessions that never shut down cleanly (or are still running) show "—" for usage.
- An in-memory inverted index powers search; use **Rescan** in the top bar to pick up
  new sessions without restarting.

## API

Everything the UI uses is plain JSON, handy for scripting:

```
GET /api/overview
GET /api/sessions?q=&repo=&model=&from=YYYY-MM-DD&to=&sort=updated|created|tokens|aiu|premium|turns|duration&dir=asc|desc&limit=&offset=
GET /api/session/<id-or-prefix>
GET /api/search?q=&role=0|1|2|3&limit=&offset=
GET /api/repos
GET /api/rescan
```

## Privacy

Read-only, localhost-only (`127.0.0.1`), zero telemetry, nothing leaves your machine.
