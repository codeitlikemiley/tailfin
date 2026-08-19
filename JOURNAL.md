# raz journal

2026-08-19 20:35 — M0 — extracted arbor workspace, renamed arbor→raz (crates, packages, identifiers, docs); 42 tests passing, clippy clean — next: M1 proxy skeleton
2026-08-19 21:01 — M1 — raz-proxy: hyper/tokio relay on 127.0.0.1:7171, hop-by-hop strip, Host rewrite, graceful shutdown; JSON body byte-identical through vs direct (55 tests) — next: M2 streaming fidelity + tee
2026-08-19 21:12 — M2 — SSE unbuffered + TeeBody (try_send, never backpressures) + kill-the-meter; HTTPS via rustls for Anthropic; 61 tests — next: REAL-AGENT GATE (needs you)
2026-08-19 21:14 — docs — filed docs/endgame.md + docs/positioning.md; CLAUDE.md north star; post-v0.1 re-sequenced M8–M12; M5 capture-schema checkbox reserved — next: M2 REAL-AGENT GATE (unchanged)
2026-08-19 21:27 — M2 — real-agent gate: Claude Code 2.1.235 through 127.0.0.1:7171; streaming + tools + Explore subagent (Haiku 4.5) on docs/endgame.md; no visible behaviour change — next: M3 identity wiring
2026-08-19 21:32 — M3 — raz-ident+arena wired; live path is declared headers or anonymous; prefix digest shadow-only (does not merge); 67 tests — next: restart proxy, one subagent session, compare `raz: tree` logs to the transcript
2026-08-19 21:42 — M3 — gate: transcript d9b82c5f-70f0-4527-882f-098d7b136723.jsonl agentId ad949528c5419e788; raz tree [session:0 agent:1] parent=session — next: M4 metering
2026-08-19 21:50 — M4 — SseDecoder+Meter on the tee; Anthropic+OpenAI usage into arena; incomplete marked; 70 tests — next: restart proxy, one real session, cross-check usage vs provider (cache tiers separate)
2026-08-19 22:12 — M4 — cross-check red: session a5ab73a5 teed complete=false in=0; transcript had cache_read=140758 out=55. Cause: dialect None (path miss) so meter never ran. Default Anthropic + strip accept-encoding; 72 tests — next: rebuild proxy, same one-sentence prompt, compare in/out/cache_*
2026-08-19 22:20 — M4 — live stream did meter: in=2 cache_read=140758 cache_1h=18935 out=32 complete=true (cache_read exact vs prior transcript; 1h within 0.05%). Freeze: /api/hello was forwarded + stderr blocked the relay; Connection refused = proxy not listening. Local hello + async logs — next: run proxy from /tmp, retry Claude
2026-08-19 23:05 — M4 — TUI freeze after a reply: stale Content-Length on reboxed bodies + HTTP/1 keep-alive reuse after SSE. Strip CL; server keep-alive off — next: restart /tmp proxy, fresh Claude session
2026-08-19 23:15 — M4 — I ran claude -p through raz twice: usage exact vs transcript (in=2 out=4 cache_1h=102426 / 107293, cache_read=0). Subagent tree [session:0 ac618ae74d66c5cca:1]. Gate closed — next: M5 ledger + report
2026-08-19 23:45 — M5 — JSONL ledger schema v1 + raz run/report; live fan-out 1.92x main 246864 / sub 227827; first row in=2 out=230 cache_1h=111401 matches transcript exactly; --capture reserved; 79 tests — next: M6 release engineering
2026-08-19 23:55 — M6 — CI + tagged release (macOS arm64/x64, Linux x64); install.sh; Formula/raz.rb; README around the finding. Gate: debian:bookworm-slim with no rustc, RAZ_TARBALL install, `raz report --help` — next: M7 launch pack
2026-08-20 00:15 — M7 — `raz report --share` redacts paths and ids; docs/launch.md drafted from the measured 1.92× / 48% session, not 82%; 2-day dogfood still open — next: human approval of docs/launch.md
2026-08-20 00:22 — M7 — committed testdata/share-ledger.jsonl; share-mode tests read the fixture through Ledger::read_all+render; 2-day dogfood still open — next: human approval of docs/launch.md
