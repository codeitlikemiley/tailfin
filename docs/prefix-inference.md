# Prefix inference — measured false-merge / false-split

Measured 2026-08-20 against the in-repo synthetic corpus in
`raz-ident::false_merge_and_split_rates_on_synthetic_traffic`. Minimum prefix
match depth is 2 (`SessionIndex` `min_level_idx = 1` → `LEVELS[1] = 2`).

| experiment | n | errors | rate |
|---|---|---|---|
| false merge (shared system prompt, then diverge) | 32 | 0 | 0.00 |
| false split (continuation adds a turn at depth ≥ 2) | 32 | 0 | 0.00 |

Live `raz-proxy` repeats the same rules on undeclared bodies: a continuation
joins; a shared first message alone does not. Declared headers (Claude Code,
Codex) still take precedence.

BLOCKED: an aider or opencode session through raz with zero declared headers
has not been run in this environment. Tune `with_min_level` against that
traffic before treating these rates as field measurements.
