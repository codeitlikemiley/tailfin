# raz roadmap — v0.1 = observe only

Gate = the sentence that must be true before the milestone is ticked.

## M0 — bootstrap  `[x]`
- [x] extracted, renamed arbor→raz, 42 tests green, clippy clean
- [x] git repo with initial commit; CLAUDE.md, ROADMAP.md, JOURNAL.md committed
Gate: `cargo test --workspace` → 42 passing on a clean clone.

## M1 — proxy skeleton (raz-proxy)
- [x] hyper server on 127.0.0.1:7171, tokio, graceful shutdown
- [x] forwards method/path/headers/body to a configurable upstream base URL
- [x] hop-by-hop headers handled correctly; everything else byte-identical
- [x] non-streaming JSON round-trip works against a stub upstream (test)
Gate: a curl of a non-streaming completion through raz is byte-identical to direct.

## M2 — streaming fidelity
- [x] SSE responses relayed unbuffered, eager flush
- [x] tee: response bytes cloned into an mpsc consumed by a task that only logs frame
      counts for now
- [x] kill-the-meter test: metering task aborted mid-stream, client still gets a
      complete response
- [x] REAL-AGENT GATE: a full interactive Claude Code session through raz —
      streaming, tool use, a subagent spawn — with no visible behaviour change
Gate: the real-agent session, plus the kill-the-meter test, both pass.

## M3 — identity wiring (raz-ident into the proxy)
- [x] resolve NodeRef per request: declared headers first (Claude Code, Codex)
- [x] prefix digest computed from request bodies but used in shadow mode only:
      logged, compared against declared identity, never yet authoritative
- [x] arena updated on request begin/finish; tree visible in debug logs
Gate: a session with subagents produces a correct tree (subagents attached to the
right parent), verified against the transcript on disk.

## M4 — metering (raz-wire into the tee)
- [x] SseDecoder + Meter consume teed frames; usage merged into the arena node
- [x] both dialects: Anthropic /v1/messages and OpenAI /chat/completions
- [x] incomplete streams marked, counted, reported
- [x] cross-check: token totals within 1% of the provider-reported usage for a real
      session (cache tiers compared separately)
Gate: the cross-check on a real session.

## M5 — ledger + report (raz-ledger, raz-cli)
- [ ] append-only JSONL ledger: one record per request finish, includes NodeRef,
      usage, confidence, incomplete flag
- [ ] `raz run` (foreground proxy) and `raz report` (reads ledger)
- [ ] report prints: total, main vs subagent split, fan-out multiplier, per-node
      table ranked by cost, peak concurrency, cache read:write ratio, incomplete count
- [ ] rate card loaded from a TOML file; missing card = token-only report, no dollars
- [ ] ledger records carry schema_version + stable task id; --capture flag
      reserved (parses, prints "capture lands in M8", stores nothing)
Gate: `raz report` on a real fan-out session prints the table and the numbers
reconcile with M4's cross-check.

## M6 — release engineering
- [ ] GitHub Actions: build + test on push; release workflow producing macOS
      arm64/x64 and Linux x64 binaries on tag
- [ ] install script (curl | sh) and Homebrew tap formula
- [ ] README rewritten around the finding, not the tool; includes the honest-ceiling
      and subscription-auth caveats verbatim from CLAUDE.md
Gate: on a machine (or clean VM/container) without Rust, install script → working
`raz` in under a minute.

## M7 — launch pack
- [ ] run raz on your own real work for 2+ days; capture your own fan-out numbers
- [ ] launch post drafted: number in the title, comparative framing, tool in
      paragraph two; draft lives in docs/launch.md
- [ ] `raz report --share` produces a paste-ready table with no identifying paths
Gate: you (the human) approve docs/launch.md. STOP and ask for this review.

## Post-v0.1 — the endgame ladder (docs/endgame.md). Do not start before the M7 gate.

Order rationale: replay ships first because it produces the follow-up launch number
and works on declared-identity tasks alone; the fuse and vouchers share one admission
mechanism, so they build together; prefix inference widens compatibility but nothing
above depends on it.

## M8 — shadow replay (endgame L2)
- [ ] `--capture` mode: full request bodies stored locally, opt-in, off by default,
      retention knob, schema versioned  *(skip if already done as part of M5)*
- [ ] `raz replay --sample N --models a,b [--since 7d]` resubmits captured tasks via
      provider batch APIs under the user's own keys, never on the interactive path
- [ ] scoring: native checks first (tests pass / compiles / diff applies); judge
      model only as fallback, reported as agreement bands, never verdicts
- [ ] output: task-shape × model × cost × survival table, n per row, confidence column
Gate: one real week of your own tasks replayed against 2+ cheaper models produces a
table you would publish as-is.

## M9 — the fuse and conservation (endgame L3)
- [ ] `--max-per-task N` enforced: synthetic `end_turn` at the ceiling, then hard
      stop per the client stop-signal table in CLAUDE.md
- [ ] vouchers: `--subagent-share P%` mints each subagent's allowance from the
      parent's remaining ceiling, enforced per node; the tree total is arithmetically
      incapable of exceeding the root ceiling
- [ ] the one-in-flight-request-per-branch overshoot bound stated verbatim in README
- [ ] synthetic-stop verified per agent actually claimed (start: Claude Code only)
Gate: a deliberately provoked runaway fan-out is stopped; the parent survives with a
readable summary in its transcript; spend never exceeds ceiling + bound.

## M10 — prefix inference authoritative
- [ ] digest promoted from shadow mode to authoritative for undeclared agents
- [ ] min match depth tuned against real multi-session traffic; false-merge and
      false-split rates measured and recorded in docs/
Gate: an aider or opencode session is attributed correctly with zero declared headers.

## M11 — cost stamps (endgame L4)
- [ ] `raz stamp <ref>`: git trailer/note with tasks, cost, models, fan-out,
      incomplete count, identity confidence; opt-in per repo
- [ ] `raz blame`: per-hunk cost rendering
- [ ] stamps are one line collapsed, expandable; no stamp without `--capture`-grade
      attribution confidence
Gate: a real PR carries a stamp a reviewer understands without explanation.

## M12 — doctor (conflict detector)
- [ ] `raz doctor` reads a LiteLLM/gateway config and reports: budget-fallback chains
      with no tier floor; compression ratios that evict prefixes from the persistent
      cache tier; memory-inject feeding compression-strip
- [ ] every rule cites its published measurement in the output
Gate: run against a real config from a public repo; findings are true on inspection.

## Parked — endgame L5 (actuarial layer)
Do not build. No telemetry or sharing code exists in this repository until raz is
established infrastructure AND an explicit opt-in design has been written, reviewed,
and approved by the human. This paragraph is the permission gate; its removal is a
human-only act.
