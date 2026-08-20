# tailfin

**The flight recorder for AI agents.**
*`tail -f` for everything your agent did while you weren't looking.*

The black box lives in the tail because that's the part most likely to survive a
crash. tailfin sits between any agent and any provider — one environment
variable, no SDK, no account — and records the whole task tree: what each job
cost, which subagent burned it, and how deep the fan-out went.

```
ANTHROPIC_BASE_URL=http://localhost:7171   # Claude Code
OPENAI_BASE_URL=http://localhost:7171/v1   # Codex, aider, opencode, Cline, local models
```

Works with the tools you already use. Everything stays on your machine. No telemetry, ever.

v0.1 observes. It does not enforce. The task is the unit.

## Install

Prebuilt binaries from [GitHub Releases](https://github.com/codeitlikemiley/tailfin/releases). You do not compile.

```sh
curl -fsSL https://github.com/codeitlikemiley/tailfin/releases/latest/download/install.sh | bash
```

Pin a tag with `TAILFIN_VERSION=v0.1.1`. Homebrew:

```sh
brew tap codeitlikemiley/tailfin https://github.com/codeitlikemiley/tailfin
brew install tailfin
```

From crates.io (compiles locally):

```sh
cargo install tailfin
```

From this repo (`cargo build -p tailfin --release`):

```sh
cargo build -p tailfin --release
./target/release/tailfin report --help
```

## Quickstart

```sh
tailfin run --upstream https://api.anthropic.com
# another terminal:
ANTHROPIC_BASE_URL=http://localhost:7171 claude
# after a session:
tailfin report
```

Commands: `run`, `report`, `replay`, `stamp`, `blame`, `doctor`.

## What it records

A real Claude Code session through tailfin (one subagent spawn):

| | tokens |
|---|---:|
| main thread | 246,864 |
| subagent you didn't watch | 227,827 |
| **total** | **474,691** |
| fan-out | **1.92×** |
| incomplete streams | 2 (counted, not zeroed) |
| cache read:write | 0.95 |

48% of that session was work off the main thread. Token counts matched the
provider usage frames. Dollars at list rates are not a bill — see
[docs/launch.md](docs/launch.md).

## Command output

Captured from the `tailfin` binary. Collapsed so this README stays a map, not a
log dump. Expand a section to see what that command prints.

<details>
<summary><code>tailfin --help</code></summary>

```
Flight recorder for AI agents

Usage: tailfin <COMMAND>

Commands:
  run     Run the proxy in the foreground
  report  Print a fan-out report from a ledger file
  replay  Replay captured tasks via a batch sink (never the interactive proxy)
  stamp   Write a one-line Tailfin-Cost git trailer/note (capture-grade identity only)
  blame   Per-node cost as hunk-shaped rows
  doctor  Report collisions in a LiteLLM/gateway config
  help    Print this message or the help of the given subcommand(s)

Options:
  -h, --help  Print help
```

</details>

<details>
<summary><code>tailfin run --help</code></summary>

```
Run the proxy in the foreground

Usage: tailfin run [OPTIONS] --upstream <UPSTREAM>

Options:
      --listen <LISTEN>
          Bind address [env: TAILFIN_LISTEN=] [default: 127.0.0.1:7171]
      --upstream <UPSTREAM>
          Provider base URL (e.g. https://api.anthropic.com) [env: TAILFIN_UPSTREAM=]
      --ledger <LEDGER>
          JSONL ledger path [env: TAILFIN_LEDGER=] [default: tailfin.jsonl]
      --capture
          Store full request bodies locally (off by default)
      --retention <RETENTION>
          How long to keep captured bodies (e.g. 7d, 24h) [default: 7d]
      --capture-dir <CAPTURE_DIR>
          Directory for captured bodies. Default: tailfin-capture next to the ledger
      --max-per-task <MAX_PER_TASK>
          Per-task ceiling in dollars. Requires `--rates`. Honest to within one in-flight request per branch
      --subagent-share <SUBAGENT_SHARE>
          Fraction of a parent's remaining ceiling minted to each new subagent (e.g. 30%)
      --rates <RATES>
          TOML rate card (µ$ per token). Without it, reports are token-only
  -h, --help
          Print help
```

</details>

<details>
<summary><code>tailfin run</code> — listening line</summary>

```
tailfin listening on http://127.0.0.1:7171 → https://api.anthropic.com/ (ledger tailfin.jsonl)
```

Point the agent at it with `ANTHROPIC_BASE_URL=http://localhost:7171`. Ctrl+C
stops the proxy; the ledger stays on disk.

</details>

<details>
<summary><code>tailfin report --help</code></summary>

```
Print a fan-out report from a ledger file

Usage: tailfin report [OPTIONS]

Options:
      --ledger <LEDGER>  JSONL ledger path [env: TAILFIN_LEDGER=] [default: tailfin.jsonl]
      --rates <RATES>    TOML rate card (µ$ per token). Without it, the report is token-only
      --share            Paste-ready table: no paths, no session or node ids
  -h, --help             Print help
```

</details>

<details>
<summary><code>tailfin report</code> — token-only fan-out table</summary>

Without `--rates`, no dollars are invented. This is the session in
[What it records](#what-it-records):

```
no rate card; token counts only (no dollars)
task 287f4ade-7829-4477-ab3c-825dd4c29171
  tokens    474691  (main 246864 / sub 227827)
  fan-out   1.92x
  peak conc 1
  incomplete 2
  cache r:w 0.95
  node                             tokens
  main 287f4ade-7829-4477-ab3c-825…  246864
  sub  a32f6f348523b0003             227827
```

`tailfin report --rates rates.toml` adds a dollar column from *your* rate card.

</details>

<details>
<summary><code>tailfin report --share</code> — paste-ready, no paths or ids</summary>

Same numbers, session UUID and node ids stripped so the table can leave the machine.

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

</details>

<details>
<summary><code>tailfin replay --help</code></summary>

```
Replay captured tasks via a batch sink (never the interactive proxy)

Usage: tailfin replay [OPTIONS]

Options:
      --sample <SAMPLE>            Max captured tasks to resubmit [default: 20]
      --models <MODELS>            Comma-separated model ids to resubmit against [default: haiku]
      --since <SINCE>              Only tasks newer than this window (e.g. 7d)
      --ledger <LEDGER>            JSONL ledger path (used to locate the default capture dir) [env: TAILFIN_LEDGER=] [default: tailfin.jsonl]
      --capture-dir <CAPTURE_DIR>  Directory of captured request bodies
      --stub                       Force the in-process stub batch (no provider, not the interactive proxy)
  -h, --help                       Print help
```

</details>

<details>
<summary><code>tailfin run --capture</code> then <code>tailfin replay --stub</code></summary>

`--capture` is off by default. When on, request bodies stay local (`--retention 7d`).
`replay` resubmits captured tasks through a batch sink — never through the
interactive proxy. Without provider keys it uses the in-process stub.

Empty capture dir (nothing recorded yet):

```
tailfin replay: no ANTHROPIC_API_KEY; stub batch (not a live week of tasks)
shape            n  model            cost      survival  confidence
(no captured tasks)
```

With a captured tool-use turn:

```
tailfin replay: no ANTHROPIC_API_KEY; stub batch (not a live week of tasks)
shape            n  model            cost      survival  confidence
tool-use          1  haiku            $0.0010   unscored  judge weak agreement
```

Native checks (compiles / tests pass / diff applies) are scored as survived/died.
A judge is an agreement *band*, never a verdict.

</details>

<details>
<summary><code>tailfin stamp --help</code> / <code>tailfin blame --help</code></summary>

```
Write a one-line Tailfin-Cost git trailer/note (capture-grade identity only)

Usage: tailfin stamp [OPTIONS] [GIT_REF]

Arguments:
  [GIT_REF]  Git ref to note (default HEAD). Printed if git notes fails [default: HEAD]

Options:
      --ledger <LEDGER>  JSONL ledger path [env: TAILFIN_LEDGER=] [default: tailfin.jsonl]
      --rates <RATES>    TOML rate card (µ$ per token). Without it, the stamp is token-only
  -h, --help             Print help
```

```
Per-node cost as hunk-shaped rows

Usage: tailfin blame [OPTIONS]

Options:
      --ledger <LEDGER>  JSONL ledger path [env: TAILFIN_LEDGER=] [default: tailfin.jsonl]
      --rates <RATES>    TOML rate card (µ$ per token). Without it, blame is token-only
  -h, --help             Print help
```

</details>

<details>
<summary><code>tailfin stamp HEAD</code> and <code>tailfin blame</code></summary>

Stamp is one collapsed line. It refuses anything below capture-grade identity
(declared headers, confidence 1.0).

```
Tailfin-Cost: tasks=1 cost=tokens-only fan-out=1.00x incomplete=0 conf=1.00 models=declared
tailfin stamp: printed only (git notes unavailable)
```

Blame is per-node cost as hunk-shaped rows:

```
hunk                                tokens      $
  a51c0897-d370-472e-8d11-73a…       97515  -
```

</details>

<details>
<summary><code>tailfin doctor --help</code></summary>

```
Report collisions in a LiteLLM/gateway config

Usage: tailfin doctor <CONFIG>

Arguments:
  <CONFIG>  Path to a LiteLLM / gateway config (YAML/TOML/JSON text)

Options:
  -h, --help  Print help
```

</details>

<details>
<summary><code>tailfin doctor testdata/gateway-litellm.yaml</code></summary>

Each finding cites a published measurement. This fixture is in-repo, not a
production LiteLLM file.

```
tailfin doctor: 3 finding(s)
- [budget-fallback-no-floor] fallback chain present without tier_floor / min_tier
  cite: LiteLLM budget fallbacks have no floor (project verification; docs/architecture.md risks)
- [compression-evicts-cache-prefix] compression ratio will evict the persistent cache prefix
  cite: cache reads price at 0.1x input; 1h writes at 2x — compacting the prefix forces re-write (CLAUDE.md invariant 3; docs/architecture.md)
- [memory-inject-feeds-compression-strip] memory injection is paired with compression/strip
  cite: memory-inject feeding compression-strip is a published collision (docs/architecture.md weeks 5–8 doctor)
```

</details>

## Honest constraints

Cost is knowable only after a response completes, so any
ceiling is hard only to within one in-flight request per branch. `Admission::Last`
encodes this. Every user-facing mention of budgets states it plainly.

`--max-per-task 5.00 --rates rates.toml` enforces that bound. `--subagent-share 30%`
mints each subagent's allowance from the parent's remaining ceiling so the tree
cannot arithmetically exceed the root.

Claude Code under subscription auth (`ANTHROPIC_BASE_URL` set, no API key): traffic
passes through tailfin but billing follows the subscription's opaque quota. We meter
tokens; we do not see their bill. Never claim otherwise.

Fan-out figures in the launch material came from one real session (48% of
tokens in a subagent, 1.92× the main thread) priced at illustrative list rates.
Ratios are real token counts; dollars are not a bill. See docs/launch.md.

Prebuilt binaries: macOS arm64/x64 and Linux x64. No Windows release.

## What it actually measures

The signal that two requests belong to one conversation is the same signal the
provider uses for prompt cache: a stable, shared, ordered prefix. Declared headers
(Claude Code, Codex) are authoritative; prefix inference is live for undeclared
agents (minimum shared depth 2). See docs/prefix-inference.md.

Cache reads price at 0.1x input; 5-minute writes at 1.25x; 1-hour writes at 2x.
Never summed into one number.

A stream that never delivered its terminal usage frame is recorded as incomplete —
never silently counted as zero.

## License

Apache-2.0. Copyright 2026 tailfin contributors. See [LICENSE](LICENSE).
