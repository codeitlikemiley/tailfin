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

Prebuilt binaries. You do not compile.

```sh
curl -fsSL https://raw.githubusercontent.com/goldcoders/raz/main/install.sh | bash
```

Homebrew:

```sh
brew tap goldcoders/raz https://github.com/goldcoders/raz
brew install raz
```

```sh
raz run --upstream https://api.anthropic.com
# another terminal:
ANTHROPIC_BASE_URL=http://localhost:7171 claude
# after a session:
raz report
```

`raz report --rates rates.toml` prints dollars. Without a rate card it prints tokens only.
`raz report --share` prints the same table with session ids and paths stripped,
so it can be pasted.

## Honest constraints

Cost is knowable only after a response completes, so any
ceiling is hard only to within one in-flight request per branch. `Admission::Last`
encodes this. Every user-facing mention of budgets states it plainly.

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
