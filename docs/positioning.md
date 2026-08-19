# raz — positioning

*Drop into the repo as docs/positioning.md. The hero block at the bottom is paste-ready for the README. Written 2026-08-19; the stats cited are from this project's own research and one measured session — refresh them with your own numbers before launch.*

---

## Tagline

> **raz — the flight recorder for AI agents.**

Durable, instantly graspable, and honest about v0.1 (it observes; it doesn't yet enforce). When M9 ships, the natural extension is already loaded: *"…and the circuit breaker."* A flight recorder is also exactly the right trust posture — neutral, always-on, and nobody argues with what it recorded.

Runners-up, kept for different surfaces:

- *"The task is the unit."* — the manifesto version; use it in the docs intro, not the hero.
- *"One env var. Every agent. The whole tree."* — the install pitch; use it on the quickstart.
- *"Agents bill by the token. You work in tasks. raz is the translation."* — the conference-talk opener.
- *"See what your agent spends when you're not looking."* — the ad-copy version; slightly too cute for the README.

The launch post is a different artifact from the tagline and follows the measured rule — number in the title, comparative framing, tool in paragraph two. The live draft is docs/launch.md:

> **"48% of this coding-agent session was work I never saw."**

---

## The USP, at three depths

**One line.** raz is the only tool that can draw a boundary — and soon a ceiling — around a *task* inside an agent nobody modified.

**One paragraph.** Every cost tool on the market is either a file parser locked to one vendor (ccusage, codeburn read `~/.claude/` and see nothing else) or a gateway that budgets by API key, user, or calendar month (LiteLLM, Portkey, Cloudflare, Anthropic's own console). None of them knows what a *task* is, so none can answer the only questions that matter: *what did this job cost, which subagent burned it, and how do I stop the next one at $5?* raz sits on the wire, infers the task tree from traffic alone — declared headers where agents publish them, prefix inference where they don't — and answers all three. One environment variable to install. Any agent, any provider, any local model. Everything stays on your machine.

**One page — why each audience says yes.**

*The solo developer* has watched a single prompt spawn subagents and eat a week of quota, and their current tool reports a dollar total with no tree. raz shows how much of the session was work they never saw, names the subagent, and (from M9) refuses to let it happen again. No account, no SDK, no database, no telemetry — one env var and it works, including against Ollama where no other cost tool even applies.

*The team lead* runs a mixed shop — some people on Claude Code, some on Cursor, one holdout on aider — which makes every vendor-locked parser useless and every key-scoped gateway blind to what actually happened inside a key. raz attributes by task across the whole toolchain, with a confidence figure attached to every attribution, and the ledger is a local file the team can actually query.

*The platform/finance owner* lives the statistic: 79% of finance leaders hit AI cost overruns last year, and the overruns arrive as *tasks* — a runaway job, a fan-out, an agent loop — while every budget they can buy is scoped to a key, a user, or a month. raz is the missing unit of account. It's also the only one whose failure mode is honest: the README states the one-in-flight-request overshoot bound instead of pretending "hard cap" means zero.

---

## Against each alternative, one sentence each

- **ccusage / codeburn:** parse one vendor's local files, don't read subagent transcripts, see nothing outside Claude Code — raz reads the wire, so it sees every agent and the whole tree.
- **LiteLLM / gateways:** budget by key/user/team/month, need Postgres, and their own docs admit budget fallbacks have no floor — raz budgets the task, in one binary, with no database.
- **Anthropic session budgets:** real, but Managed-Agents-API only — not your CLI, not your other models, priced at list rates.
- **AgentBudget and every SDK limiter:** requires instrumenting code you own — structurally unable to reach the agents people actually run.
- **Merge / MDM-style governance:** a two-week enterprise rollout to route people onto approved models — raz is thirty seconds and doesn't need your IT department.

---

## Anti-claims — things raz must never say

These come from this project's own verification work; violating them burns the trust the whole endgame depends on.

1. Never "hard cap" without the bound: *hard to within one in-flight request per branch.*
2. Never put Cursor in the headline list — desktop chat works, background/headless can't take a base URL.
3. Under Claude subscription auth, raz meters tokens against an opaque quota — it does not see the bill. Don't sell "control your Anthropic bill" to Max-plan users.
4. "Works with any agent" means: exact for Claude Code and Codex (declared headers), inferred-with-disclosed-confidence for the rest. Say both halves.
5. Do not cite 82% / 11.6×. Those were an earlier research session. Launch numbers live in docs/launch.md (one measured session: 48% of tokens in a subagent, 1.92× fan-out). Ratios are real token counts; dollars are not a bill.

---

## README hero block (paste-ready)

```markdown
# raz

**The flight recorder for AI agents.**

Your coding agent spawns subagents you never see, and they spend most of your money.
raz sits between any agent and any provider — one environment variable, no SDK, no
account — and shows you the whole task tree: what each job cost, which subagent burned
it, and how deep the fan-out went. Soon: stop any task at a ceiling you set.

    ANTHROPIC_BASE_URL=http://localhost:7171   # Claude Code
    OPENAI_BASE_URL=http://localhost:7171/v1   # Codex, aider, opencode, Cline, local models

Works with the tools you already use. Everything stays on your machine. No telemetry, ever.
```
