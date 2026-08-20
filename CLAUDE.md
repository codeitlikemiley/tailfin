# tailfin

**Queue and last facts: [STATUS.md](STATUS.md).** This file is invariants.

A Rust proxy that infers **task structure** from LLM wire traffic, so per-task cost
accounting — and later a per-task budget — becomes possible for agents nobody modified.

The claim that defines the project: every product that enforces a per-task ceiling today
requires the caller to declare the task boundary (an SDK context manager, a registered
agent, a trace-id header, a hosted harness). None can draw a boundary around an unmodified
coding agent that just opens an HTTPS connection. tailfin draws it anyway. Everything else is
downstream of that.

v0.1 **defaults to observation**: task tree + fan-out ledger + report. `--max-per-task` (M9) is opt-in enforcement.

## Invariants — violating any of these is a bug, whatever the tests say

1. **Passthrough discipline.** The relay never owns a response body. Never
   `to_bytes()` a body to inspect it; never reserialize; never reorder or drop headers
   you don't understand. Observation happens on a tee (cloned frames into an mpsc
   channel consumed off the critical path). If the metering task panics, the user's
   request must still complete — keep the test that proves it.
2. **Streams are sacred.** Forward SSE bytes as they arrive. Clients time streams
   (Claude Code aborts after 300s of silence) and count relayed bytes. No buffering,
   no coalescing, flush eagerly.
3. **Cache tiers stay separate.** Cache reads price at 0.1x input; 5-minute writes at
   1.25x; 1-hour writes at 2x. Never sum them into one number. Summing is how tools
   report savings while the bill goes up.
4. **Incomplete is not zero.** A stream that never delivered its terminal usage frame
   is recorded as incomplete and surfaced in reports — never silently counted as zero.
5. **The ceiling is honest.** Cost is knowable only after a response completes, so any
   ceiling is hard only to within one in-flight request per branch. `Admission::Last`
   encodes this. Every user-facing mention of budgets states it plainly.
6. **Rate cards are config, never constants.** Providers change prices; contracts have
   discounts. Hardcoded prices are a lie with a start date.
7. **Identity confidence is recorded.** Declared identity = 1.0; inferred carries its
   prefix depth; anonymous = 0. Reports disclose how much of their own attribution
   they trust.
8. **A shared system prompt alone must never merge two sessions.** Minimum prefix
   match depth is 2. There is a test for this; it stays.
9. **Ship binaries, never a compile step as the only path.** Release = prebuilt
   macOS arm64/x64 + Linux x64, an install script, a Homebrew tap. crates.io
   (`cargo install tailfin`) is a secondary compile-from-source option. `cargo
   install --git` as the *only* path is a launch-killer (measured: tools that
   required it died at ~50 stars).

## Crate map and dependency rules

| crate | role | allowed heavy deps |
|---|---|---|
| tailfin-ident | task identity: header parsing + rolling prefix digest | none |
| tailfin-wire | incremental SSE decode + usage extraction | serde_json only |
| tailfin-tree | task arena, roll-up, admission | none beyond siblings |
| tailfin-proxy | hyper/tokio relay + tee | tokio, hyper, rustls |
| tailfin-ledger | append-only JSONL (capture/replay/stamp/doctor) | none beyond siblings |
| tailfin | `run` `report` `replay` `stamp` `blame` `doctor` | clap |

The three core crates stay free of HTTP, async, and I/O forever — that is what makes
them testable without a socket and reusable as cdylib/wasm later. New heavy
dependencies anywhere require a one-line justification in the commit message.

## Client stop-signal table (M9 fuse)

| client | clean stop | without it |
|---|---|---|
| Claude Code | `429` + `x-should-retry: false` | retries up to 10x with backoff |
| opencode / Cline / Continue (Vercel AI SDK) | `402` or `400` | `429` retried twice first |
| aider | `400` | retries on budget-shaped errors |
| Codex | typed quota error | unknown status → `UnexpectedStatus` → retried |

Order at the ceiling: synthetic `end_turn` first (ends the turn cleanly, leaves a
summary in the transcript), hard status for every later request in that task. The
synthetic alone lets spending resume next turn; the hard status alone discards
in-progress work. The synthetic technique has no prior art — test per agent, and mark
synthetic text unmistakably because it persists in history and prompt cache.

## Known caveats to preserve in user-facing docs

- Claude Code under subscription auth (`ANTHROPIC_BASE_URL` set, no API key): traffic
  passes through tailfin but billing follows the subscription's opaque quota. We meter
  tokens; we do not see their bill. Never claim otherwise.
- Codex headers (`x-codex-turn-metadata`, `root_turn_id`) verified on the Responses
  API path only; `wire_api = "chat"` behaviour unverified. Test before claiming.
- Cursor: desktop chat can use a custom base URL; background/headless `cursor-agent`
  cannot. Keep Cursor out of the headline compatibility list.
- Fan-out figures in the launch material came from one real session (48% of
  tokens in a subagent, 1.92× the main thread) priced at illustrative list rates.
  Ratios are real token counts; dollars are not a bill. See docs/launch.md.
- OpenRouter upstream is `https://openrouter.ai/api` (keep `/api`). Dropping the
  prefix 404s. HTTP WebSocket is tunneled; HTTPS upgrades are not. opencodex
  disables Responses WebSocket (426) and Codex falls back to SSE — we meter that.

## Working protocol

- **Queue is [STATUS.md](STATUS.md).** ROADMAP.md is a closed M0–M12 archive.
  JOURNAL.md is append-only. If STATUS.md's queue is empty, stop and ask.
- Tests before code. `cargo test --workspace` and
  `cargo clippy --workspace --all-targets --locked -- -D warnings` before a commit.
- Commit via PR (main is protected). After each block: update STATUS.md, append
  one JOURNAL.md line: `YYYY-MM-DD HH:MM — <what> — <next>`.
- Never weaken a test to make it pass. Blocked: `BLOCKED:` in JOURNAL.md, then
  ask — do not start an unqueued item.
- Proxy or wire change: rebuild `tailfin`, restart the proxy from that binary,
  live-test the agent the change affects (Claude Code, Codex, or OpenCode). Do
  not reuse the previous process. Do not use the human as a test harness.
- Releases: STATUS.md "How to cut". Tag `v*` publishes GitHub Releases and
  crates.io via Actions. Never `cargo publish` locally. Formula sha256s are a
  follow-up PR after `SHA256SUMS` exists.

## North star (docs/endgame.md) — design consequences only

The endgame ladder: the task boundary makes work **visible** (v0.1), **comparable**
(replay), **conserved** (vouchers), **attributable** (stamps), and eventually
**priceable** (actuarial — parked). L2–L4 software shipped; their live calendar
gates were skipped. Read docs/endgame.md once; it changes DESIGN DECISIONS, never
the current STATUS.md queue.

1. **Ledger schema is versioned and capture-capable.** Every record carries a
   `schema_version` and a task id stable across process restarts. Full request-body
   capture is an opt-in flag (`--capture`), local-only, with a retention knob.
   Retrofitting capture after the schema ships is surgery; carrying the fields now
   is free.
2. **Admission is per-node, not just per-root.** The arena keeps an allowance field
   on nodes, defaulting to "inherit from root," so vouchers (M9) become bookkeeping
   rather than a refactor of the admission path.
3. **No telemetry, ever, in this codebase.** Captured data never leaves the machine.
   The endgame's final level lives or dies on trust, and trust dies on exactly one
   exception — so there are none, including "anonymous usage stats."
4. **Records preserve enough structure to compute "task shape" later** — message
   count, tool-call mix, token histogram, fan-out — without ever re-reading bodies.
   Shape from metadata, never from content.
5. **The endgame is not a license for scope creep.** If a change serves the endgame
   but is not on the current milestone, it becomes a ROADMAP line under the right
   milestone — never code today.
