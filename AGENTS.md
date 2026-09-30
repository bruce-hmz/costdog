# CostDog

A local cost monitor for AI coding CLIs. It parses each tool's own logs, prices the token
usage, and shows the result in an always-on-top desktop bar.

Two independent frontends share one SQLite database at `~/.costdog/costdog.sqlite`:

- **Desktop bar** (`src-tauri/`) — Rust + Tauri v2. Self-contained: it does its own
  scanning, pricing and aggregation in Rust and needs no Node runtime.
- **CLI / web dashboard** (`src/`) — TypeScript. `costdog` for the terminal, `costdog web`
  for an Express dashboard on port 3456.

The two do **not** talk to each other. The web dashboard is currently a thinner, older view
(stats cards, cost by model, recent sessions) and lacks the budget, trend, cost-quality and
activity-category features the bar has.

## Data sources

| Source | Path | Notes |
|---|---|---|
| Claude Code | `~/.claude/projects/*/*.jsonl` | Session logs; token usage, tool calls, disk writes |
| Codex CLI | `~/.codex/sessions/*/rollout-*.jsonl` | `token_count` events are cumulative — last value wins. `input_tokens` includes cached, so cached is subtracted |
| ZCode | `~/.zcode/cli/db/db.sqlite` | `model_usage` rows aggregated per (session, local date) |
| OpenCode | `~/.local/share/opencode/opencode.db` | `session` table is pre-aggregated and carries its own `cost` |
| DeepSeek Harness | `~/.dsh/sessions/*/*/session.v4.jsonl.zstd` | zstd JSONL; `assistant/message.usage` per step is the billing unit. `inputTokens` is *uncached*, so cache hit is `cacheRead/(input+cacheRead)`. Sessions span days, so buckets are keyed by message timestamp's local date. Auxiliary `~/.dsh/storages/session_projcache/sessions/<id>.json` supplies model/CTX/timings without decompressing | Both frontends parse it: `src-tauri/src/dsh.rs` (ruzstd) and `src/parsers/dsh.ts` (node:zlib). Both need their own frame loop — every `zstdDecompress*` helper in both runtimes stops after the **first** frame, which is 226 bytes of `{"type":"session"}` and makes every session look empty.

Overrides: `CODEX_HOME`, `ZCODE_HOME`, `OPENCODE_DB`, `DSH_HOME`, `XDG_DATA_HOME`,
`COSTDOG_DATA_DIR`, `COSTDOG_PORT`.

Prices come from the OpenRouter API, cached 24h in `~/.costdog/pricing-cache.json` and
shared with the TypeScript side. A small **provider-level** table (`provider_price()` in
`lib.rs`, `PROVIDER_PRICING` in `src/utils/pricing.ts`) wins over OpenRouter: the same model
id can differ 2-10x between providers, and OpenRouter's cache-read estimate (input×0.1) is
Anthropic-shaped — it overstates a cache-heavy DeepSeek session by ~3.8x.

## Scanning

Scans are **incremental**, and the two mechanisms differ:

- **Claude Code / Codex / DeepSeek Harness** — per-file fingerprints (size + mtime) in
  `scan_files`. Unchanged files are skipped. DSH fingerprints each `session.v4.jsonl.zstd`
  and only re-decompresses the sessions that actually changed.
- **ZCode / OpenCode** — timestamp watermarks in `scan_watermarks`, minus a 24h lookback so
  rows that land after the timestamp they carry are not missed permanently.

ZCode selects *sessions* touched since the watermark and re-aggregates **every** row those
sessions own. Filtering the usage rows themselves would write a partial token total over the
real one.

Watermarks and fingerprints advance only after session rows commit, so a failed scan retries
from the same point.

The background thread ticks every 30s and scans on every tick while the bar is visible, or
every 10th tick when hidden. Visibility is tracked in a `BAR_VISIBLE` atomic rather than by
calling `is_visible()`, which dispatches to the main thread and blocks.

## Cost ledger

`session_costs` records how each cost was derived: `cost_basis` is `provider` (the tool
reported a real charge), `estimated` (computed from OpenRouter prices) or `unpriced` (no
price matched). The upsert deliberately allows `unpriced → priced` transitions, which is what
lets the "Re-fetch prices" button work.

Re-pricing (`refresh_pricing`) reads the `sessions` table rather than rescanning logs —
because scans are incremental, a rescan would only revisit the last day of source rows.

## Desktop bar

- 410×36 fixed, undecorated, always on top, not resizable. Expanding the detail panel resizes
  the window to 410×520.
- Five skins (classic / night / lcd / receipt / race), theme mode and accent colour, all
  persisted in `localStorage`. The accent only drives the classic skin — the other four are
  art-directed against fixed palettes.
- The detail panel leads with the stat grid and budget; trend, cost quality, contributions,
  the activity donut and source diagnostics live in a collapsible "Analysis & diagnostics"
  section so the numbers stay on the first screen.
- The whole UI is English. The skins' English typography is deliberate; a test fails on any
  CJK character in **user-visible text** of `src-tauri/embedded/index.html`. Comments (HTML,
  block and line) are stripped first — Chinese comments are this repo's norm and the file's
  CSS comments carry layout/iron-rule history, so they must not gate the check.
- On macOS it runs as an accessory app (no Dock icon) with a monochrome template tray icon.
  Closing hides the window on every platform; the tray is how it comes back.
- The frontend is a single embedded HTML file talking to Rust over IPC. There is no HTTP
  fallback — it never runs outside Tauri.

## Commands

```bash
npm run tauri:dev      # run the desktop bar
npm run tauri:build    # build installers
npm run test:ts        # TypeScript + desktop-config tests
cargo test --manifest-path src-tauri/Cargo.toml
npm run icons          # regenerate app icons + the tray template
npm run build && npm link   # CLI
```

## Conventions

- Comments explain *why*, not what. Several non-obvious invariants above exist only because a
  comment records them — keep them with the code they justify.
- Behaviour worth keeping is pinned by a test. `tests/tauri-config.test.ts` guards the
  desktop config and the HTML structure itself (block order, keyboard access, language).
