# tailfin — architecture and week-one scope

*A Rust proxy that infers task structure from the wire. Solo developer day 1, enterprise later, same binary.*

*Status 2026-08-20:* M0–M12 software is ticked on ROADMAP.md. Remaining work is
the 2-day dogfood and the calendar gates, not this week-one plan. The day-by-day
section below is the original plan, kept as the record of intent.

> **tailfin** — the flight recorder lives in the tail because that's what survives
> a crash. `tail -f` for everything your agent did while you weren't looking.

---

## The one-sentence claim

**Everything that enforces a per-task budget today requires the caller to declare the task boundary.** An SDK context manager, a registered agent object, a trace-id header, a hosted harness, a CI workflow. Not one of them can draw a boundary around an unmodified coding agent that opens an HTTPS connection and starts talking.

tailfin draws it anyway. Everything else in this document is downstream of that.

That is also why the ladder works: once you can name a task, you can meter it, cap it, classify it, and arbitrate objectives for it. Without it, none of those are possible, which is exactly why nobody has built them.

---

## Why Rust, concretely

Not preference — four properties the product actually needs.

1. **It sits in the hot path of every request.** A garbage-collection pause inside a streaming SSE relay is a stall the client can see. Claude Code aborts a stream that goes silent for 300 seconds and counts every relayed byte.
2. **The install must be one file.** The clearest finding in the adoption research: `npx <tool>` wins, `cargo install --git` dies at 53 stars. Rust gives you a static binary — but **ship prebuilt binaries via install script, Homebrew and Scoop. Never make a user compile.** That distinction is the whole lesson.
3. **It parses untrusted network input** — SSE frames and provider JSON, on every byte of every response. Memory safety is not decoration here.
4. **The same core compiles three ways.** Binary today, `cdylib` for embedding later, `wasm32` for an edge deployment. The enterprise path doesn't need a rewrite.

---

## Crate layout

```
tailfin/
├─ crates/
│  ├─ tailfin-ident      task identity: header parsers + prefix digest
│  ├─ tailfin-wire       SSE decode + provider-agnostic usage extraction
│  ├─ tailfin-tree       task arena, cost roll-up, admission decisions
│  ├─ tailfin-ledger     append-only JSONL, capture/replay/stamp/doctor
│  ├─ tailfin-proxy      hyper service, relay + tee
│  └─ tailfin            run | report | replay | stamp | blame | doctor
```

The three crates that carry the hard logic have no HTTP dependency, no async
runtime, and no I/O — which is why they were testable before a single byte moved
over a socket. `cargo test --workspace` is 115 passing. Classes/floors
(`tailfin-policy`) were never a crate; they stay a later rung on the ladder.

---

## The architectural constraint everything else obeys

**Passthrough by default. Observe by tee, never by parse-then-reserialize.**

Anthropic's gateway guidance is explicit that a gateway which wraps or rewrites upstream payloads breaks the client's error-recovery path and the header/body pairing beta features depend on. Stripping half of a header/body pair produces hard `400`s. And a gateway that buffers a response to inspect it stalls a stream the client is timing.

So the relay never owns the body:

```rust
// NEVER — this is the mistake that breaks agents:
let body = hyper::body::to_bytes(upstream.into_body()).await?;
let parsed: Value = serde_json::from_slice(&body)?;

// ALWAYS — bytes flow through; frames are cloned to a metering task
// that is allowed to fail without touching the relay:
let (tx, rx) = mpsc::channel(64);
let teed = TeeBody::new(upstream.into_body(), tx);
tokio::spawn(meter_task(rx, node_ref));
Ok(Response::from_parts(parts, teed))
```

There are exactly **two** places tailfin generates bytes rather than relaying them: the synthetic stop, and the deny response. Everywhere else it is a wire.

---

## Task identity — the part nobody has

Two signals, in precedence order.

**1. Declared.** Anthropic publishes `x-claude-code-session-id`, `x-claude-code-agent-id` (subagent requests only) and `x-claude-code-parent-agent-id` (nested only) *specifically so a gateway can attribute cost to parallel agents*. Codex ships `x-codex-turn-metadata` carrying `root_turn_id` — the task-tree root, the single best field for this in the ecosystem. When these are present, attribution is exact.

**2. Inferred.** For aider, Cline, Continue, and anything custom, tailfin computes a **rolling prefix digest** over the message array — cumulative hashes at depths 1, 2, 4, 8, 16, 32. Two requests belong to the same task when they share a deep prefix.

The elegant part: *the signal that tells you two requests are one conversation is the same signal the provider uses to decide whether its prompt cache hits* — a stable, shared, ordered prefix. Prefix matching isn't a heuristic bolted on the side; it is the wire's own notion of continuity.

Guardrails that are already implemented and tested:

- A shared system prompt alone must not merge two sessions — the minimum match depth is 2, not 1.
- A continuation matches its own past at the depth they share, so turn 9 joins the session that turn 5 opened.
- Two branches that diverge at message 3 share exactly 2, not 8.
- Every identity carries a `confidence`, and the ledger records it, so a report can disclose how much of its own attribution it trusts.

---

## Request lifecycle

```
  client                tailfin                            upstream
    │                     │                                │
    │──── request ───────▶│                                │
    │                     ├─ classify: headers → declared  │
    │                     │            else prefix digest  │
    │                     ├─ resolve NodeRef {root,node,parent}
    │                     ├─ admit(task) ──┐               │
    │                     │                │ Allow / Last / Deny
    │                     │◀───────────────┘               │
    │                     │                                │
    │        [Deny] ◀─────┤  synthetic end_turn, then      │
    │                     │  429+x-should-retry:false (Anthropic)
    │                     │  or 402 (AI-SDK clients)       │
    │                     │                                │
    │                     ├──── relay, headers intact ────▶│
    │                     │                                │
    │◀═══ streamed bytes ═╪═══════ SSE frames ═════════════│
    │                     │                                │
    │                     └──tee──▶ SseDecoder → Meter → Ledger
    │                                             │
    │                                       task.finish()
```

The tee is off the critical path. If the meter panics, the user's request still completes — an invariant worth a test.

---

## Stopping an agent without corrupting it

Layered, because each half alone is wrong.

| Client | Signal | Behaviour |
|---|---|---|
| Claude Code | `429` + **`x-should-retry: false`** | Stops immediately. Without the header it retries **up to 10 times** with backoff |
| opencode, Cline, Continue (Vercel AI SDK) | `402` or `400` | Immediate clean stop. `429` is retried twice first |
| aider | `400` | Clean stop. Careful: aider **retries** on budget errors, which is what LiteLLM returns |
| Codex | typed quota error | Unrecognized statuses fall into `UnexpectedStatus` and get retried |

**The order is: synthetic `end_turn` at the ceiling, then a hard status for every subsequent request in that task.** A synthetic response ends the current turn cleanly and leaves a readable summary in the transcript; without the hard status, spending simply resumes next turn. Without the synthetic, in-progress work is thrown away.

The synthetic-response technique has **no prior art anywhere** — the one proposal for it inside an agent framework was closed as not planned. That is either the opportunity or the warning; test it against every agent you claim to support before shipping it. Two caveats to design around: it puts words in the model's mouth that persist in history and prompt cache, so mark them unmistakably; and it must be a valid SSE sequence, since a buffering gateway stalls the client.

**And say this in the README, in these words:** a ceiling is only hard to within one in-flight request per branch. Cost is knowable only after a response completes. Cloudflare states this outright; Vercel calls its budget "a soft cap, not a hard limit"; Anthropic's own session budgets carry the same bound. A tool claiming otherwise is not measuring.

---

## The ladder

Each rung is useful alone, and each one is only reachable because of the one below.

| | Capability | Unlocked by | Buyer |
|---|---|---|---|
| **0** | **Observe** — fan-out ledger, cache-thrash ratio, per-subagent cost | task identity | individual dev |
| **1** | **Enforce** — per-task ceiling, graceful stop | roll-up + admission | individual dev, small team |
| **2** | **Classify** — request classes, `critical` never below tier X | identity + policy | platform team |
| **3** | **Arbitrate** — floors, lexicographic ranking, explicit unsatisfiable case | classes | enterprise |
| **4** | **Distribute** — shared policy, identity, audit export | all of it | enterprise |

Rung 0 is the whole of week one. Rungs 3 and 4 are where the money is, and they are unreachable without rung 0 — which is precisely why the category is empty.

**Same binary at every rung.** No config → works alone. `--config policy.toml` → team. `--ledger sqlite://…` → shared. `--control-plane https://…` → fleet. That escalation is the Grafana and Vector playbook, and it is how one tool serves a solo developer and an enterprise without a rewrite.

---

## Week one, day by day

Realistic for one person at 40–60 focused hours. **Rung 0 only. No enforcement.**

| Day | Deliverable | Done when |
|---|---|---|
| **1–2** | Transparent streaming proxy | Claude Code runs a full session through it with no behaviour change. Header fidelity, correct hop-by-hop handling, zero buffering. This is harder than it looks and it is the whole foundation |
| **3** | Wire in `tailfin-ident` + `tailfin-tree` | A real session produces a correct task tree with subagents attached to parents |
| **4** | Wire in `tailfin-wire` metering | Token counts match what the provider reports, cache tiers kept separate |
| **5** | JSONL ledger + `tailfin report` | The fan-out table prints from a real session |
| **6** | Release engineering | Prebuilt binaries for macOS arm64/x64 + Linux x64 via GitHub Actions. Install script. Homebrew tap. README |
| **7** | Buffer, then launch | |

### Cut list — everything below is explicitly *not* in week one

TLS termination · config files · **all enforcement** · prefix-inferred sessions (declared headers only) · SQLite · any UI · Codex `wire_api = "chat"` · Gemini and Bedrock dialects · Windows binaries.

Prefix inference is the crown jewel and it is *still* cut from week one. It is week 2–3. Week one proves the proxy doesn't break anything and the fan-out number is real.

### The launch post

Not "Show HN: my proxy." Every dead launch in this category pitched a tool; every successful one pitched a finding, with the number in the title. Yours already exists — this conversation measured it:

> **82% of what my coding agent spends is work I never see**

That figure was the research session that set the shape of the post. v0.1 launch
numbers are in docs/launch.md. Do not reuse 82% / 11.6×.

Comparative framing travels (706 points for "Claude Code sends 33k tokens before reading the prompt; OpenCode sends 7k"). Introspective framing does not (4 points for a *more* dramatic stat the same week). So lead with the comparison — your fan-out vs. your main thread, or agent A vs. agent B — and put the tool in the second paragraph.

---

## After week one

**Weeks 2–3 — prefix inference.** Turn on the digest for undeclared agents. This is what makes the claim "works with any agent" true rather than aspirational, and it is the defensible asset.

**Week 4 — the fuse.** `--max-per-task 5.00`, synthetic stop, hard status. Ship the overshoot caveat in the README.

**Weeks 5–8 — the conflict detector.** A `doctor` subcommand that reads an existing LiteLLM or gateway config and reports collisions: budget fallback chains with no tier floor, compression ratios that will evict prefixes from the persistent cache tier, memory plugins feeding compression plugins set to strip. Provably absent in every product I checked, each rule backed by a published measurement, and it needs no adoption of your runtime at all.

**Month 3+ — classes and floors.** Rung 2 and 3. Only if rung 0 found people who care.

---

## Risks, priced

1. **Platform absorption is live.** Anthropic shipped hard per-session budgets on 2026-08-07 — Managed Agents API only, not Claude Code CLI, but the direction is unmistakable. LiteLLM's recent releases show explicit work on Codex and Cursor agent support. **The ledger survives either; the fuse might not.** That is the argument for shipping observation first.
2. **Subscription auth doesn't route billing through you.** With `ANTHROPIC_BASE_URL` set but no API credential, the claude.ai login stays active — traffic passes through, subscription quota applies. You can meter and stop, but you are estimating against an opaque quota, not dollars. Don't overclaim the "one prompt ended my Pro plan" story.
3. **Cursor is a partial citizen.** Desktop chat can take a custom base URL; background and headless `cursor-agent` cannot. Keep it out of the headline compatibility list.
4. **The fidelity tax never ends.** Claude Code gains beta headers and body fields every release. Inspect, don't modify — and scope the synthetic response tightly, because it is the one place a wire change breaks you silently.
5. **Nobody pays for the limiter.** LiteLLM gives budgets away free and monetizes SSO, audit logs, RBAC and support. Plan the enterprise tier around identity and audit, never around the cap.

---

## Uncertain, flagged honestly

1. **Codex's `x-codex-turn-metadata` was verified on the Responses API path only.** If a user configures `wire_api = "chat"`, I could not confirm the headers still ship. Test before designing around it.
2. **The synthetic `end_turn` is untested in the wild.** It should work mechanically — Claude Code ends a turn on `stop_reason: end_turn` with no tool_use block — but no prior art exists, so verify per agent.
3. **Prefix inference is unproven against real traffic.** The tests cover the logic; they do not tell you what the right minimum match depth is when fifty sessions share a 4,000-token system prompt. Expect to tune `with_min_level`, and expect that tuning to be the hardest empirical work in the project.
4. **The 82% / 11.6× figures come from one session** — this one — priced at illustrative Opus-class list rates. The ratios are real token counts; the dollars are not a bill. Measure your own before you publish anyone else's.
5. **Rate cards must be config, never constants.** Anthropic's own session budgets price at public list rates, so negotiated discounts make a cap fire early. Inherit that flaw knowingly or avoid it deliberately.
