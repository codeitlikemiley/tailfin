# Prefix inference — measured false-merge / false-split

Field facts belong here; the queue is [STATUS.md](../STATUS.md).

Measured 2026-08-20 against the in-repo synthetic corpus in
`tailfin-ident::false_merge_and_split_rates_on_synthetic_traffic`. Minimum prefix
match depth is 2 (`SessionIndex` `min_level_idx = 1` → `LEVELS[1] = 2`).

| experiment | n | errors | rate |
|---|---|---|---|
| false merge (shared system prompt, then diverge) | 32 | 0 | 0.00 |
| false split (continuation adds a turn at depth ≥ 2) | 32 | 0 | 0.00 |

Live `tailfin-proxy` repeats the same rules on undeclared bodies: a continuation
joins; a shared first message alone does not. Declared headers (Claude Code,
Codex) still take precedence.

Live 2026-08-20: **opencode** (no Claude/Codex headers) hit
`/v1/chat/completions` through tailfin. First request of the session:
`declared=false`, prefix digest compared, `conf=0.25`, root `inferred-000003`.
That is the undeclared path. The turn completed (`in=11828 out=3`). A
continuation at depth ≥ 2 was not observed on this one-shot. Synthetic 0/32
rates still stand; field `with_min_level` tuning still wants a second turn.

**Codex** is the *declared* path, not this table: `conf=1.00` on
`/v1/responses`. It is not evidence for prefix inference.
