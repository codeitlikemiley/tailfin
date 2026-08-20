# tailfin journal

Append-only log. **Current state is [STATUS.md](STATUS.md).** Do not pick a
"next" from this file.

Historical entries used the old name **raz** (arbor→raz at M0). The project
renamed raz→tailfin on 2026-08-20.

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
2026-08-20 00:43 — M7 — protocol: every milestone rebuilds raz, restarts the proxy, and runs a real Claude session. Rebuilt /tmp/raz; claude -p → pong; ledger in=2 out=4 cache_1h=97509 complete; report --share → task 1, no ids — next: human approval of docs/launch.md
2026-08-20 00:50 — M7 — docs/launch.md approved; gate closed. 2-day dogfood still open — next: M8 shadow replay when you say go
2026-08-20 01:05 — M8 — opt-in --capture (request tee, schema v1, retention) + raz replay stub-batch table (native/judge bands, never interactive). Rebuilt /tmp/raz-m8; claude -p → pong complete in=2 out=4 capture_id set; report --share redacted. BLOCKED: live provider batch + one week of captured tasks — next: M9 fuse
2026-08-20 01:12 — M9 — --max-per-task + --subagent-share vouchers; synthetic end_turn then 429 x-should-retry:false. Rebuilt /tmp/raz-m9; claude -p pong (Last overshoot); next req synthetic, then 429. spend 146593 vs ceiling 0 (one in-flight). README bound verbatim — next: M10 prefix inference
2026-08-20 01:18 — M10 — prefix digest live for undeclared agents; min depth 2 stays. docs/prefix-inference.md 0/32 false-merge, 0/32 false-split. Rebuilt /tmp/raz-m10; claude -p pong complete. BLOCKED: aider/opencode live — next: M11 stamp/blame
2026-08-20 01:24 — M11 — raz stamp/blame; capture-grade (conf=1.0) gate. Rebuilt /tmp/raz-m11; claude -p pong; stamp one-line Raz-Cost declared. BLOCKED: real PR stamp — next: M12 doctor
2026-08-20 01:30 — M12 — raz doctor three rules with citations; fixture testdata/gateway-litellm.yaml. Rebuilt /tmp/raz-m12; claude -p pong complete. BLOCKED: production public LiteLLM config. Parked L5 untouched — next: none (ladder software shipped)
2026-08-20 06:10 — ops — CI/release for hexuria/raz: fmt+clippy+test+install smoke; tag v* publishes tarballs+SHA256SUMS+install.sh on GitHub Releases — next: push and cut a tag
2026-08-20 06:40 — rename — raz→tailfin everywhere (crates, bins, env, docs, Formula, CI); GitHub codeitlikemiley/tailfin. `tail -f` for the agent's flight. — next: tests + first tag on the new repo
2026-08-20 07:15 — rename — 115 tests, clippy -D warnings clean; README command output captured from the binary (collapsed); LICENSE Apache-2.0 tailfin contributors — next: create codeitlikemiley/tailfin, push, tag v0.1.0
2026-08-20 09:30 — crates.io — path+version workspace deps so the six crates can publish; cargo install tailfin — next: publish 0.1.0
2026-08-20 09:50 — crates.io — publish is CI: tag v* or workflow_dispatch runs scripts/publish-crates.sh; already-published versions skipped — next: 2-day dogfood
2026-08-20 10:00 — release — bump 0.1.0 → 0.1.1 (CI publish + formula placeholders) — next: tag v0.1.1
2026-08-20 10:10 — release — v0.1.1 tagged; GitHub Release + crates.io via Actions (all six crates 0.1.1); formula sha256s in #5 — next: 2-day dogfood
2026-08-20 10:20 — docs — catch stale "M9 will ship" / prefix-is-shadow / week-one-as-todo; historical JOURNAL and week-one plan left as record — next: 2-day dogfood
2026-08-20 10:25 — skip — dogfood, M8 live replay, M11 stamp PR, L5 telemetry — next: opencode+Codex through rebuilt tailfin, M12 public LiteLLM
2026-08-20 10:40 — M10 — Codex exec pong via :7171→:8080 declared conf=1.00; WS /v1/responses 404 then HTTPS fallback; usage not metered (Responses not a dialect). opencode MiniMax via :7172→tokenrouter declared=false digest conf=0.25 inferred-000001; stream incomplete. — next: M12 doctor
2026-08-20 11:05 — proxy — Codex `/v1/responses` is a dialect (SSE + WS text frames); `SseDecoder::finish` flushes leftover JSON; `rewrite_uri` joins an upstream path prefix (OpenRouter `/api`); HTTP WebSocket tunneled on raw TCP because hyper-util Client does not upgrade. Live: Codex exec pong via :7171→opencodex :8080, HTTPS SSE `in=19431 out=1 complete=true` (WS 426 is opencodex disabling the transport, then HTTP fallback). opencode `openrouter/openai/gpt-4o-mini` via :7172→`https://openrouter.ai/api`, `inferred-000003` `in=11828 out=3 complete=true`. MiniMax/tokenrouter still needs credits. — next: none queued
2026-08-20 10:41 — M12 — doctor on BerriAI/litellm proxy_server_config.yaml → budget-fallback-no-floor (context_window_fallbacks, no floor). Same on oai_misc_config.yaml. — next: none queued
2026-08-20 11:20 — docs — STATUS.md is the session handoff (queue, live agents, how to release); ROADMAP.md is the M0–M12 archive; JOURNAL append-only; architecture/endgame/launch point at STATUS. — next: cut v0.1.2

