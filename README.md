# raz

A proxy that infers **task structure** from LLM wire traffic, so a per-task budget
becomes possible for agents you didn't write.

Every product that enforces a per-task ceiling today makes the caller declare the
task boundary — an SDK context manager, a registered agent, a trace-id header, a
hosted harness. None can draw one around an unmodified coding agent that opens an
HTTPS connection and starts talking. This draws it anyway.

## Status

Three crates, no HTTP dependency, no async runtime, no I/O — testable before a
single byte moves over a socket.

| crate | what | tests |
|---|---|---|
| `raz-ident` | task identity: header parsers + rolling prefix digest | 15 |
| `raz-wire`  | incremental SSE decode + provider-agnostic usage extraction | 15 |
| `raz-tree`  | task arena, cost roll-up, admission decisions | 12 |

```
cargo test --workspace     # 42 passing
cargo clippy --workspace --all-targets
```

`raz-proxy`, `raz-ledger`, `raz-cli` are week one. `raz-policy` is month three.

## The two ideas worth reading the code for

**Prefix digest** (`raz-ident`) — the signal that tells you two requests belong to
one conversation is the same signal the provider uses to decide whether its prompt
cache hits: a stable, shared, ordered prefix. So prefix matching isn't a heuristic
bolted on the side; it's the wire's own notion of continuity.

**Cache tiers stay separate** (`raz-wire`) — reads price at 0.1x input, 5-minute
writes at 1.25x, 1-hour writes at 2x. Summing them is how tools end up reporting
savings while the bill goes up.

## Honest constraints, encoded in the tests

- A ceiling is hard only to within **one in-flight request per branch**. Cost is
  knowable only after a response completes. `Admission::Last` names this.
- A stream that never delivers its terminal usage frame is marked
  `is_complete() == false`, not silently recorded as zero.
- A shared system prompt alone must never merge two sessions.

## License

Apache-2.0
