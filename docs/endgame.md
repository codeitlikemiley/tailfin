# tailfin: The Endgame

*The escalation nobody has built, derived from everything this project already established. Written 2026-08-19. Levels 2–5 are design, not research — treat the claims of novelty as "not found after extensive search," never as proven absence.*

---

## The frame

**The token is the piston stroke. The task is the ride.**

The entire AI industry bills in a unit no human cares about, because nobody could measure the unit humans do care about. You don't price a taxi by counting engine revolutions — but that's exactly how agent work is priced today, and the results are everywhere in this project's research: 79% of finance leaders over budget, orgs burning 3× their annual AI budget by June, an employee at a $1.4B fintech spending $80,000 in a week without anyone noticing until the invoice.

Per-token billing survives because it externalizes *all* variance onto the customer. The provider never has to know whether your task was efficient; you absorb the difference. That information asymmetry is margin, which is why no provider will ever fix it.

tailfin's boundary inference — the ability to say "these 40 requests were one task" for an agent nobody modified — is the **odometer**. And once a ride has an odometer, everything else becomes possible: fares, insurance, fleet economics, a market. Every level below is the same asset — the task boundary — worn a different way.

---

## Level 1 — the flight recorder *(already planned: v0.1 through M9)*

Observe, then enforce. The fan-out ledger and the per-task fuse. Covered by the roadmap; skipped here except to note that everything below reuses its exact data structures. Nothing in this document requires new research — only new consumers of the arena and the ledger.

---

## Level 2 — Shadow Replay: the counterfactual ledger

**The claim:** the entire model-routing category is guessing, and tailfin can replace guessing with evidence — using data only tailfin has.

The research on this was brutal and specific. Three of four production routers emit near-constant tier assignments. `Always-Mid` matches a real router *exactly* on three of four benchmarks. The one router that actually reads prompts shows no advantage over content-blind allocation at matched tier shares. RouteLLM's famous "85% savings" collapses to 14% against a random baseline on MMLU. Routing fails because it predicts *prospectively*, per-request, with no ground truth about how *your* tasks respond to cheaper models.

tailfin has what no router has: **complete recorded tasks with real boundaries.** So don't predict — *replay*.

```
tailfin replay --last-week --models haiku,gemini-flash,local-qwen --sample 20
```

Take twenty completed tasks from the ledger. Resubmit them through the providers' **batch APIs** — half price, off-peak, completely outside the interactive path (which also sidesteps the cache-invalidation objection that kills live A/B routing: batch pricing is cache-free anyway). Score the counterfactual outputs: where the task has a native check, use it (tests pass, code compiles, diff applies); elsewhere use a judge model and report *agreement bands, never verdicts*. Output:

```
  task shape                    current      cheapest equivalent     confidence
  test generation (n=31)        sonnet  $84   local-qwen  $0          high (tests pass)
  refactor w/ types (n=12)      opus   $210   sonnet     $61          high (compiles+tests)
  research synthesis (n=8)      opus   $95    — no equivalent —       judge disagreement
```

That table is **your routing policy, derived from your work, with ground truth** — the thing every router pretends to have. It's also the launch post: *"I replayed last week's agent work on three cheaper models. Here's what actually survived."* Comparative, re-runnable, number-in-title — every viral mechanic this project measured, in one artifact.

**Why nobody has it:** replay requires task boundaries plus full request capture. Only the thing that draws boundaries can replay them. **Scope:** a weekend on top of M5's ledger. **What kills it:** judge noise (mitigate with bands and native checks), batch API coverage, and storage of response bodies — keep them local, under the user's own keys, with a retention knob, or don't store them at all.

Build this immediately after v0.1. It's the feature that pays for the product with a number.

---

## Level 3 — Conservation: budgets that obey physics

**The claim:** delegation should conserve money the way physics conserves energy — and today it conserves nothing.

Right now, when an agent spawns a subagent, the budget "transfers" as prompt text: *"try to keep costs reasonable."* The honor system, addressed to a stochastic process. Anthropic's own docs concede there is no limit on total subagents per session; the 415-agents-in-6-minutes genre of complaint is what the honor system produces. Even Anthropic's new session budgets are one shared pool — a subagent can drain the whole thing.

tailfin can enforce **conservation**: a parent task's ceiling *subdivides*. Spawning a subagent mints a voucher — a slice of the parent's remaining budget. The subagent's requests spend from its voucher; exhausting the voucher stops the subagent (gracefully, per the stop table) *without touching the parent's remaining balance*. Total spend across the tree can never exceed the root ceiling, not because everyone behaved, but because the accounting is physically incapable of exceeding it.

The beautiful part: **the day-one version needs no protocol at all.** Every subagent already passes through the same proxy — the vouchers are just arena bookkeeping on data structures that exist. `--max-per-task 5.00 --subagent-share 30%` and the arena does the rest. The cross-machine version — signed vouchers traveling in headers so a task delegated to another box or another *company* carries an enforceable allowance — is the long game, and it's the control plane the emerging agent-payment rails (x402 and friends) conspicuously lack: they move money between agents; nothing bounds *authorization to spend within a task*.

**What kills it:** nothing technical intra-proxy. Cross-org needs adoption on both ends — don't lead with it, let it emerge from the intra-proxy semantics being obviously right.

---

## Level 4 — Provenance: cost travels with the work

**The claim:** the audit trail everyone failed to sell becomes valuable the moment it attaches to the *artifact* instead of a dashboard.

This project catalogued the graveyard: fourteen tamper-evident agent-audit-ledger attempts in nineteen months, thirteen of them at ≤4 points on Hacker News. They all sold the same thing — "trust me, here's a hash chain" — to a compliance buyer who wasn't in the room. The one that got traction led with a concrete buyer and scenario.

tailfin's version inverts the direction: the record travels *with the work*.

```
tailfin stamp HEAD
```

attaches a git trailer or note to the commit: what tasks produced it, total cost, models used, fan-out multiplier, incomplete-stream count, identity confidence. `tailfin blame` renders dollars-per-hunk. A PR arrives annotated: *"3 tasks, $12.40, fan-out 4.2×, one subagent burned $8 and contributed no surviving lines."*

That last clause is why this isn't a compliance feature. It's a **code-review fact** — the reviewer's honest answer to "how was this made," and the measured answer to "AI wrote this" disclosure, which today is pure self-report. The buyer is the reviewer, present in the room, every day. And it composes: Level 2's replay data means a stamp can eventually say *"this could have been produced for $0.60"* — cost review becomes part of code review.

**What kills it:** noise. If every commit gets a stamp nobody reads, it's spam. Make it opt-in per repo, one line in the PR body, expandable — and let the outlier stamps (the $340 PR) do the marketing.

---

## Level 5 — The actuarial layer: pricing work instead of fuel

**The claim — the one genuinely nobody has touched:** once tasks have boundaries, they have cost *distributions*, and anything with a measurable distribution can be **quoted, and then underwritten.**

Uber didn't invent the car; it invented the upfront fare. The fare required the odometer plus millions of recorded rides. tailfin instances, in aggregate, are recording the rides: task shape → cost distribution → outcome rate. With strictly opt-in, anonymized sharing of *distributions* (never content — shapes, token counts, outcome flags), you get the table that doesn't exist anywhere:

> *Tasks shaped like this one: $2.80 ± $0.90, 94% completion, across 40,000 observations.*

First that's a **quote** — the answer to "what will this cost?" that no developer, no CFO, and no provider can currently give. Then it's a **budget with statistical teeth**: set the fuse at p95 automatically instead of guessing a number. And ultimately it's what an **underwriter** needs: someone can sell fixed-price agent work — "this class of refactor: $4 flat" — and absorb the variance profitably, because the variance is finally measured. That's the moment agent labor stops being priced like fuel and starts being priced like work.

The provider can't build this — per-token billing *is* the strategic position, and a meter that reveals the true cost distribution of tasks erodes it. The underwriter can't be the meter — a counterparty that fixes prices needs an *independent* odometer or nobody trusts the quote. Which is the argument, in the end, for what tailfin has to be: **open source, third-party, and boring enough to trust.** The neutrality isn't a virtue; it's the product requirement.

**What kills it:** doing it early. This level needs tailfin to already be trusted infrastructure at meaningful scale, and one telemetry scandal ends the whole arc — this project's own research on the trust deficit in AI tooling is the cautionary tale. Strictly opt-in, distributions only, published methodology, and not before the flight recorder has earned its place.

---

## The order, and the through-line

Ship **L2 (replay)** right after v0.1 — a weekend of work, and it converts tailfin from "interesting meter" to "paid for itself with a table." **L3 (conservation)** follows — the intra-proxy version is arena bookkeeping, and it's the fuse's natural adult form. **L4 (stamps)** is opportunistic — a formatter over data the ledger already holds. **L5** waits for trust, deliberately.

The through-line, which is also the answer to "what are we really building": every level is the same asset. The boundary makes the task **visible** (L1), **comparable** (L2), **conserved** (L3), **attributable** (L4), and finally **priceable** (L5).

tailfin starts as a flight recorder, becomes a circuit breaker, and ends as the meter that lets agent work be priced like work instead of like fuel.

---

## What would falsify the whole arc

Stated plainly, because a vision without kill conditions is marketing:

1. **Providers ship task-level billing themselves.** Watch Anthropic's session budgets — if they extend from Managed Agents to the raw Messages API and add attribution, L1–L3 compress. (Unlikely at L5: pricing transparency cuts against their margin structure — but "unlikely" is not "impossible.")
2. **The wire closes.** The declared headers could vanish in a release; agents could pin TLS or move to opaque transports. Prefix inference is the hedge, and it must stay excellent for exactly this reason.
3. **Judge-scored replay turns out too noisy** outside code tasks. Then L2 narrows to the domains with native checks — still valuable, smaller story.
4. **Nobody actually changes behavior when shown the table.** The deepest risk, and the one this project's virality research half-confirms: people share findings and keep habits. The fuse and the vouchers are the mitigation — they don't require behavior change, they *are* the behavior change.
