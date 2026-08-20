# tailfin roadmap — v0.1 = observe only

Gate = the sentence that must be true before the milestone is ticked.

**Where we are (2026-08-20):** M0–M12 *software* is ticked. Skipped: 2-day
dogfood, M8 live-week replay, M11 published-PR stamp, L5 telemetry. Closed
live: M10 opencode (undeclared) + Codex (declared headers); M12 doctor on
BerriAI/litellm `proxy_server_config.yaml`. Nothing else is queued.

## M0 — bootstrap  `[x]`
- [x] extracted arbor workspace; renamed arbor→raz then raz→tailfin; tests green, clippy clean
- [x] git repo with initial commit; CLAUDE.md, ROADMAP.md, JOURNAL.md committed
Gate: `cargo test --workspace` → 42 passing on a clean clone.

## M1 — proxy skeleton (tailfin-proxy)
- [x] hyper server on 127.0.0.1:7171, tokio, graceful shutdown
- [x] forwards method/path/headers/body to a configurable upstream base URL
- [x] hop-by-hop headers handled correctly; everything else byte-identical
- [x] non-streaming JSON round-trip works against a stub upstream (test)
Gate: a curl of a non-streaming completion through tailfin is byte-identical to direct.

## M2 — streaming fidelity
- [x] SSE responses relayed unbuffered, eager flush
- [x] tee: response bytes cloned into an mpsc consumed by a task that only logs frame
      counts for now
- [x] kill-the-meter test: metering task aborted mid-stream, client still gets a
      complete response
- [x] REAL-AGENT GATE: a full interactive Claude Code session through tailfin —
      streaming, tool use, a subagent spawn — with no visible behaviour change
Gate: the real-agent session, plus the kill-the-meter test, both pass.

## M3 — identity wiring (tailfin-ident into the proxy)
- [x] resolve NodeRef per request: declared headers first (Claude Code, Codex)
- [x] prefix digest computed from request bodies but used in shadow mode only:
      logged, compared against declared identity, never yet authoritative
- [x] arena updated on request begin/finish; tree visible in debug logs
Gate: a session with subagents produces a correct tree (subagents attached to the
right parent), verified against the transcript on disk.

## M4 — metering (tailfin-wire into the tee)
- [x] SseDecoder + Meter consume teed frames; usage merged into the arena node
- [x] both dialects: Anthropic /v1/messages and OpenAI /chat/completions
- [x] incomplete streams marked, counted, reported
- [x] cross-check: token totals within 1% of the provider-reported usage for a real
      session (cache tiers compared separately)
Gate: the cross-check on a real session.

## M5 — ledger + report (tailfin-ledger, tailfin)
- [x] append-only JSONL ledger: one record per request finish, includes NodeRef,
      usage, confidence, incomplete flag
- [x] `tailfin run` (foreground proxy) and `tailfin report` (reads ledger)
- [x] report prints: total, main vs subagent split, fan-out multiplier, per-node
      table ranked by cost, peak concurrency, cache read:write ratio, incomplete count
- [x] rate card loaded from a TOML file; missing card = token-only report, no dollars
- [x] ledger records carry schema_version + stable task id; --capture flag
      reserved (parses, prints "capture lands in M8", stores nothing)
Gate: `tailfin report` on a real fan-out session prints the table and the numbers
reconcile with M4's cross-check.

## M6 — release engineering
- [x] GitHub Actions: build + test on push; release workflow producing macOS
      arm64/x64 and Linux x64 binaries on tag
- [x] install script (curl | sh) and Homebrew tap formula
- [x] crates.io: six crates; tag `v*` publishes GitHub Releases *and* crates.io via Actions
- [x] README rewritten around the finding, not the tool; includes the honest-ceiling
      and subscription-auth caveats verbatim from CLAUDE.md
Gate: on a machine (or clean VM/container) without Rust, install script → working
`tailfin` in under a minute.

## M7 — launch pack  `[x]`
- [x] 2-day dogfood **skipped** (human: not doing it)
- [x] launch post drafted: number in the title, comparative framing, tool in
      paragraph two; draft lives in docs/launch.md
- [x] `tailfin report --share` produces a paste-ready table with no identifying paths
Gate: docs/launch.md approved 2026-08-20. Dogfood skipped 2026-08-20.

## Post-v0.1 — the endgame ladder (docs/endgame.md). M7 gate passed 2026-08-20.

Order rationale: replay ships first because it produces the follow-up launch number
and works on declared-identity tasks alone; the fuse and vouchers share one admission
mechanism, so they build together; prefix inference widens compatibility but nothing
above depends on it.

## M8 — shadow replay (endgame L2)
- [x] `--capture` mode: full request bodies stored locally, opt-in, off by default,
      retention knob, schema versioned  *(skip if already done as part of M5)*
- [x] `tailfin replay --sample N --models a,b [--since 7d]` resubmits captured tasks via
      provider batch APIs under the user's own keys, never on the interactive path
      *(live batch uses StubBatch until keys+a week of captures exist)*
- [x] scoring: native checks first (tests pass / compiles / diff applies); judge
      model only as fallback, reported as agreement bands, never verdicts
- [x] output: task-shape × model × cost × survival table, n per row, confidence column
Gate: **skipped** 2026-08-20 (human: no live-week replay). Software path remains
stub-batch + table.

## M9 — the fuse and conservation (endgame L3)
- [x] `--max-per-task N` enforced: synthetic `end_turn` at the ceiling, then hard
      stop per the client stop-signal table in CLAUDE.md
- [x] vouchers: `--subagent-share P%` mints each subagent's allowance from the
      parent's remaining ceiling, enforced per node; the tree total is arithmetically
      incapable of exceeding the root ceiling
- [x] the one-in-flight-request-per-branch overshoot bound stated verbatim in README
- [x] synthetic-stop verified per agent actually claimed (start: Claude Code only)
      *(stub upstream + Claude Code headers; live runaway is a second Claude request)*
Gate: stub path proven. Live multi-request runaway depends on Claude issuing a
second `/v1/messages` after the overshoot.

## M10 — prefix inference authoritative
- [x] digest promoted from shadow mode to authoritative for undeclared agents
- [x] min match depth tuned against real multi-session traffic; false-merge and
      false-split rates measured and recorded in docs/
      *(synthetic corpus in docs/prefix-inference.md: 0/32 false merge, 0/32 false split)*
Gate: live 2026-08-20. **opencode** `openrouter/openai/gpt-4o-mini` via
`http://127.0.0.1:7172` → `https://openrouter.ai/api`: path
`/v1/chat/completions` (upstream prefix `/api` is joined, not replaced),
`declared=false`, prefix digest on, `conf=0.25`, task `inferred-000003`,
replied `Pong`, `in=11828 out=3 complete=true`. Earlier MiniMax/tokenrouter
turn hung incomplete on a `$0` credit limit — that is the upstream, not
tailfin. **Codex** `exec` via `http://127.0.0.1:7171` → local opencodex
`:8080`: `declared=true`, `conf=1.00`, session id as root, replied `pong`.
`/v1/responses` is metered (`Dialect::OpenAiResponses`); HTTPS SSE
`in=19431 out=1 cache_read=128 complete=true`. Codex prefers
`ws://…/v1/responses`; tailfin tunnels HTTP upgrades. opencodex returns
`426 Upgrade Required` (`Responses WebSocket transport is disabled; use HTTP`)
and Codex falls back to SSE. Not aider.

## M11 — cost stamps (endgame L4)
- [x] `tailfin stamp <ref>`: git trailer/note with tasks, cost, models, fan-out,
      incomplete count, identity confidence; opt-in per repo
- [x] `tailfin blame`: per-hunk cost rendering
- [x] stamps are one line collapsed, expandable; no stamp without `--capture`-grade
      attribution confidence
Gate: **skipped** 2026-08-20 (human: no published stamp PR).

## M12 — doctor (conflict detector)
- [x] `tailfin doctor` reads a LiteLLM/gateway config and reports: budget-fallback chains
      with no tier floor; compression ratios that evict prefixes from the persistent
      cache tier; memory-inject feeding compression-strip
- [x] every rule cites its published measurement in the output
Gate: live 2026-08-20. `tailfin doctor` on BerriAI/litellm
`proxy_server_config.yaml` (public, repo root, 251 lines,
`context_window_fallbacks` set, no `tier_floor` / `min_tier`):
`[budget-fallback-no-floor]` — 1 finding. Same rule on
`litellm/proxy/example_config_yaml/oai_misc_config.yaml`. Fixture
`testdata/gateway-litellm.yaml` still covers the three-rule combo.

## Parked — endgame L5 (actuarial layer)
Do not build. No telemetry or sharing code exists in this repository until tailfin is
established infrastructure AND an explicit opt-in design has been written, reviewed,
and approved by the human. This paragraph is the permission gate; its removal is a
human-only act.
