# raz journal

2026-08-19 20:35 — M0 — extracted arbor workspace, renamed arbor→raz (crates, packages, identifiers, docs); 42 tests passing, clippy clean — next: M1 proxy skeleton
2026-08-19 21:01 — M1 — raz-proxy: hyper/tokio relay on 127.0.0.1:7171, hop-by-hop strip, Host rewrite, graceful shutdown; JSON body byte-identical through vs direct (55 tests) — next: M2 streaming fidelity + tee
2026-08-19 21:12 — M2 — SSE unbuffered + TeeBody (try_send, never backpressures) + kill-the-meter; HTTPS via rustls for Anthropic; 61 tests — next: REAL-AGENT GATE (needs you)
2026-08-19 21:14 — docs — filed docs/endgame.md + docs/positioning.md; CLAUDE.md north star; post-v0.1 re-sequenced M8–M12; M5 capture-schema checkbox reserved — next: M2 REAL-AGENT GATE (unchanged)
2026-08-19 21:27 — M2 — real-agent gate: Claude Code 2.1.235 through 127.0.0.1:7171; streaming + tools + Explore subagent (Haiku 4.5) on docs/endgame.md; no visible behaviour change — next: M3 identity wiring
