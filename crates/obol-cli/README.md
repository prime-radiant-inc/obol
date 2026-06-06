# obol-cli

Command-line tool to estimate the USD cost of an AI-agent transcript (Claude Code, Codex, Pi).

```bash
cargo install obol-cli      # installs the `obol` binary

obol estimate session.jsonl            # auto-detects the dialect
obol estimate session.jsonl --json     # machine-readable
obol refresh                           # update the on-disk pricing snapshot
```

Built on [`obol-core`](https://crates.io/crates/obol-core). Part of
[obol](https://github.com/prime-radiant-inc/obol). Apache-2.0.
