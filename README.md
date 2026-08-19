# raz

**The flight recorder for AI agents.**

Your coding agent spawns subagents you never see, and they spend most of your money.
raz sits between any agent and any provider — one environment variable, no SDK, no
account — and shows you the whole task tree: what each job cost, which subagent burned
it, and how deep the fan-out went. Soon: stop any task at a ceiling you set.

```
ANTHROPIC_BASE_URL=http://localhost:7171   # Claude Code
OPENAI_BASE_URL=http://localhost:7171/v1   # Codex, aider, opencode, Cline, local models
```

Works with the tools you already use. Everything stays on your machine. No telemetry, ever.

v0.1 observes. It does not enforce. The task is the unit.

## Install

Prebuilt binaries from [GitHub Releases](https://github.com/hexuria/raz/releases). You do not compile.

```sh
curl -fsSL https://github.com/hexuria/raz/releases/latest/download/install.sh | bash
```

Pin a tag with `RAZ_VERSION=v0.1.0`. Homebrew:

```sh
brew tap hexuria/raz https://github.com/hexuria/raz
brew install raz
```

Build from this repo with **`-p raz-cli`** (`-p raz` is the wrong package):

```sh
cargo build -p raz-cli --release
./target/release/raz-cli report
./target/release/raz-cli run --upstream https://api.anthropic.com
```

`report`, `replay`, `stamp`, `blame`, and `doctor` live on this binary. A different program named `raz` may already be on PATH; this repo's CLI is `./target/release/raz-cli` (same code as `./target/release/raz`).

`raz report --rates rates.toml` prints dollars. Without a rate card it prints tokens only.
`raz report --share` prints the same table with session ids and paths stripped,
so it can be pasted.

`raz run --capture` stores request bodies locally (off by default; `--retention 7d`).
`raz replay --sample 20 --models haiku,sonnet` resubmits those tasks through a batch
sink — never through the interactive proxy.

## Honest constraints

Cost is knowable only after a response completes, so any
ceiling is hard only to within one in-flight request per branch. `Admission::Last`
encodes this. Every user-facing mention of budgets states it plainly.

`--max-per-task 5.00 --rates rates.toml` enforces that bound. `--subagent-share 30%`
mints each subagent's allowance from the parent's remaining ceiling so the tree
cannot arithmetically exceed the root.

Claude Code under subscription auth (`ANTHROPIC_BASE_URL` set, no API key): traffic
passes through raz but billing follows the subscription's opaque quota. We meter
tokens; we do not see their bill. Never claim otherwise.

Fan-out figures in the launch material came from one real session (48% of
tokens in a subagent, 1.92× the main thread) priced at illustrative list rates.
Ratios are real token counts; dollars are not a bill. See docs/launch.md.

## What it actually measures

The signal that two requests belong to one conversation is the same signal the
provider uses for prompt cache: a stable, shared, ordered prefix. Declared headers
(Claude Code, Codex) are authoritative; prefix inference is shadow-mode until M8.

Cache reads price at 0.1x input; 5-minute writes at 1.25x; 1-hour writes at 2x.
Never summed into one number.

A stream that never delivered its terminal usage frame is recorded as incomplete —
never silently counted as zero.

## License

Apache-2.0
