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
- [ ] SSE responses relayed unbuffered, eager flush
- [ ] tee: response bytes cloned into an mpsc consumed by a task that only logs frame
      counts for now
- [ ] kill-the-meter test: metering task aborted mid-stream, client still gets a
      complete response
- [ ] REAL-AGENT GATE: a full interactive Claude Code session through raz —
      streaming, tool use, a subagent spawn — with no visible behaviour change
Gate: the real-agent session, plus the kill-the-meter test, both pass.

## M3 — identity wiring (raz-ident into the proxy)
- [ ] resolve NodeRef per request: declared headers first (Claude Code, Codex)
- [ ] prefix digest computed from request bodies but used in shadow mode only:
      logged, compared against declared identity, never yet authoritative
- [ ] arena updated on request begin/finish; tree visible in debug logs
Gate: a session with subagents produces a correct tree (subagents attached to the
right parent), verified against the transcript on disk.

## M4 — metering (raz-wire into the tee)
- [ ] SseDecoder + Meter consume teed frames; usage merged into the arena node
- [ ] both dialects: Anthropic /v1/messages and OpenAI /chat/completions
- [ ] incomplete streams marked, counted, reported
- [ ] cross-check: token totals within 1% of the provider-reported usage for a real
      session (cache tiers compared separately)
Gate: the cross-check on a real session.

## M5 — ledger + report (raz-ledger, raz-cli)
- [ ] append-only JSONL ledger: one record per request finish, includes NodeRef,
      usage, confidence, incomplete flag
- [ ] `raz run` (foreground proxy) and `raz report` (reads ledger)
- [ ] report prints: total, main vs subagent split, fan-out multiplier, per-node
      table ranked by cost, peak concurrency, cache read:write ratio, incomplete count
- [ ] rate card loaded from a TOML file; missing card = token-only report, no dollars
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

## Post-v0.1 (do not start without explicit instruction)
- M8 — prefix inference authoritative for undeclared agents (aider/Cline/Continue),
       tuned min-depth against real multi-session traffic
- M9 — the fuse: --max-per-task, synthetic end_turn + hard stop, per-client table
- M10 — `raz doctor`: config conflict detector (budget chains without tier floors,
        compression vs cache-tier eviction, memory-inject vs compression-strip)
