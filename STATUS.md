# STATUS — read this first

This file is the session handoff. A later agent (or human) should be able to
continue from here without the previous chat.

| file | job |
|---|---|
| **STATUS.md** | queue, last facts, how to release |
| [CLAUDE.md](CLAUDE.md) | invariants; do not infer progress from it |
| [ROADMAP.md](ROADMAP.md) | M0–M12 archive (closed) |
| [JOURNAL.md](JOURNAL.md) | dated log; append only |
| [docs/](docs/) | design, launch numbers, measurements |

Do not reopen skipped gates unless the human says so. Do not invent the next
milestone. If the queue is empty, stop and ask.

---

## Now

- **Repo:** `codeitlikemiley/tailfin` (this checkout: `/Volumes/goldcoders/Projects/tailfin`)
- **Version:** `0.1.2` on GitHub Releases and crates.io (all six crates)
- **Queue:** empty
- **L5 telemetry:** parked. Human-only to unpark.

### How to cut the next tag

1. Bump `workspace.package.version` and the `tailfin-*` versions in `[workspace.dependencies]`.
2. Formula `version`, sha256 placeholders; README pin `TAILFIN_VERSION`.
3. PR to protected `main`. CI green. Merge.
4. `git tag vX.Y.Z && git push origin vX.Y.Z`.
5. Wait for workflows `release` and `crates-io`.
6. Fill `Formula/tailfin.rb` sha256s from the Release `SHA256SUMS`. Follow-up PR.
7. Never `cargo publish` locally. Update this file's **Version** / **Queue**.

---

## Shipped

Software M0–M12 is on `main`. Live gates that actually ran:

| what | evidence |
|---|---|
| Claude Code | streaming + tools + subagent through `:7171`; fan-out 1.92× (main 246,864 / sub 227,827) in [docs/launch.md](docs/launch.md) |
| Codex `exec` 0.147 | `:7171` → local opencodex `:8080`; `/v1/responses`; `declared=true conf=1.00`; `pong`; SSE `in=19431 out=1 cache_read=128 complete=true` |
| opencode 1.18.11 | `:7172` → `https://openrouter.ai/api`; `/v1/chat/completions`; `declared=false conf=0.25`; `inferred-000003`; `Pong`; `in=11828 out=3 complete=true` |
| doctor | BerriAI/litellm `proxy_server_config.yaml` → `[budget-fallback-no-floor]` |

Identity: Claude Code + Codex = declared headers. opencode = prefix digest (min depth 2). See [docs/prefix-inference.md](docs/prefix-inference.md).

Wire dialects: Anthropic `/v1/messages`, OpenAI `/v1/chat/completions`, OpenAI `/v1/responses`. `rewrite_uri` **joins** an upstream path prefix (`https://openrouter.ai/api` + `/v1/chat/completions`). HTTP WebSocket is tunneled on raw TCP (`crates/tailfin-proxy/src/ws.rs`); hyper-util's Client does not upgrade.

---

## Skipped — leave closed

| item | human |
|---|---|
| 2-day dogfood | not doing it |
| M8 live-week replay | no week of `--capture` + live batch keys |
| M11 published stamp PR | no stamp on a public PR |
| L5 telemetry / sharing | parked; this paragraph is the permission gate |

---

## Not queued (optional later)

Only pick these up if the human names one.

| item | why it is still open |
|---|---|
| **aider live** | M10 named it; we ran OpenCode and Codex instead |
| **opencode second turn** | one-shot only; prefix join at depth ≥ 2 unmeasured in the field |
| **HTTPS WebSocket** | HTTP WS works; `https://` upstreams still 426 on the hyper-util hop. Codex then uses SSE, which we meter |
| **M9 live runaway** | fuse proven on stub + Claude headers; no real second `/v1/messages` after overshoot |
| **Codex `wire_api = "chat"`** | headers verified on `/v1/responses` only |
| MiniMax / tokenrouter | OpenCode's default model; `$0` credit limit. Not a tailfin bug |

v0.1 never claimed: Gemini, Bedrock, Windows binaries, TLS termination, SQLite, a UI, Cursor background `cursor-agent`.

---

## How a later session should work

1. Read this file. If the queue is empty, ask the human. Do not scan JOURNAL.md for a "next" and start it.
2. Proxy or wire changes: tests first; `cargo clippy --workspace --all-targets --locked -- -D warnings`; live-test the agent the change affects (Claude, Codex, or OpenCode). Rebuild the binary, restart the proxy, do not reuse an old process.
3. After a block: update **this file** (queue + last facts), append one line to JOURNAL.md, PR to `main`.
4. Releases: the list under **How to cut the next tag**. Then set **Version** / **Queue** here.

### This machine

- Build with `CARGO_TARGET_DIR=/tmp/raz-target`. Compiling on `/Volumes/...` hangs in dyld.
- Ad-hoc sign test/release bins: `codesign --force --sign - <bin>`. `xattr -c` if a bin sits in `_dyld_start`.
- `127.0.0.1:8080` is the user's opencodex; `:8787` is Headroom. Do not kill them.
- Do not edit `~/.codex/config.toml` (its `openai_base_url` is `:8080`). Override per-exec: `codex exec -c 'openai_base_url="http://127.0.0.1:7171/v1"' …`
- opencodex **disables** Responses WebSocket (`426 Upgrade Required`; body says use HTTP). Codex logs the 426, then SSE. That 426 is not a tailfin failure.
- Live OpenCode used `/tmp/tf-opencode/opencode.json` with `baseURL: http://127.0.0.1:7172/v1` and `-m openrouter/openai/gpt-4o-mini`. Global default is `tokenrouter/MiniMax-M3`.
- OpenRouter upstream **must** be `https://openrouter.ai/api` (the `/api` prefix). `https://openrouter.ai` 404s.
