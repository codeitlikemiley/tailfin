# 48% of this coding-agent session was work I never saw

The main thread used 246,864 tokens. A subagent I didn't watch used 227,827 —
almost as many, from one spawn. Together that's 474,691 tokens: a 1.92× fan-out
on a single Claude Code session.

tailfin is a local proxy. One environment variable points the agent at it; it draws
the task tree from traffic the agent already sends. No SDK, no account, no
telemetry. Everything stays on the machine. `tailfin report --share` prints the
paste-ready table:

```
no rate card; token counts only (no dollars)
task 1
  tokens    474691  (main 246864, sub 227827)
  fan-out   1.92x
  peak conc 1
  incomplete 2
  cache r:w 0.95
  role                             tokens
  main                               246864
  sub 1                              227827
```


A transcript parser locked to `~/.claude/` never sees those 227,827 tokens. It
reads the parent log, which contains the subagent's compact result, so fan-out
looks like cheap parallelism. The wire sees the whole tree.

```
curl -fsSL https://github.com/codeitlikemiley/tailfin/releases/latest/download/install.sh | bash
tailfin run --upstream https://api.anthropic.com
ANTHROPIC_BASE_URL=http://localhost:7171 claude
tailfin report --share
```

v0.1 defaults to observation. `--max-per-task` is opt-in; a ceiling is hard only
to within one in-flight request per branch.

## What these numbers are

- One real Claude Code session, 2026-08-19, through tailfin. Declared identity
  (session + agent headers). One subagent. Peak concurrency 1.
- Token counts are from the provider's usage frames on the wire, cache tiers
  kept separate. They matched the on-disk transcript on the metered turns.
- Two streams never delivered a terminal usage frame. They are counted
  incomplete, not silently zeroed.
- This is not two days of dogfood. It is not the 82% / 11.6× figure from earlier
  research. Do not quote those.

## What they are not

Claude Code under subscription auth (`ANTHROPIC_BASE_URL` set, no API key):
traffic passes through tailfin but billing follows the subscription's opaque quota.
We meter tokens. We do not see the bill.

At illustrative Opus-class list rates ($15 / M input, $75 / M output, cache
writes 1.25× / 2×, reads 0.1×) the same session is about $6.42, of which $2.35
(37%) is the subagent. Dollar share is not token share: the main thread paid
for 1-hour cache writes, which price at 2× input. Summing cache tiers into one
number is how tools report savings while the bill goes up. Those dollars are
not a bill.

Cursor is not in the compatibility list. Desktop chat can take a custom base
URL; background `cursor-agent` cannot.

Identity is exact for Claude Code and Codex (declared headers). Prefix inference
is live for undeclared agents (minimum shared depth 2) and carries a confidence
figure. Field measurement on aider/opencode is still open — see
docs/prefix-inference.md.

## Status

Approved 2026-08-20. The 2+ day dogfood checkbox on ROADMAP.md is still open.
