# RustStrom v2 data plane: status

Short status of the core workstream (Pingora 0.9.1). Evidence comes from `cargo test`,
`cargo clippy --all-targets -- -D warnings` and `scripts/smoke-v2.sh` on roberta.

## M1 HTTP proxy: done

- Pingora `Server` bootstrap with own CLI (`-c/--config`, `--check`, `--version`, `--upgrade`,
  `--daemon`); v1.1 TOML plus the optional v2 keys of DESIGN.md.
- v1 matcher language, pre-indexed router (exact host map + ordered fallback, `:authority` for h2).
- Strategies RoundRobin, Random, IPHash (consistent hash ring), LeastConnection (P2C on in-flight
  requests), StickyCookie (v1.1 opaque id, `Path=/`).
- Forwarding headers like v1.1, Prometheus metrics with the v1.1 names.
