# Fireup - Agent Instructions

## Cursor Cloud specific instructions

### Overview

Fireup is a Rust CLI tool that migrates Firestore LevelDB backup data into PostgreSQL. It has three subcommands: `import`, `analyze`, and `validate`. See `README.md` for full usage details.

### Services

| Service | How to start | Notes |
|---------|-------------|-------|
| PostgreSQL 15 | `sudo docker compose up -d postgres` | Required for `import` command and integration tests. Exposed on port 5433. Wait for readiness: `sudo docker exec fireup_postgres pg_isready -U fireup -d fireup_dev` |

### Building and running

- `cargo build` to compile; `cargo run -- --help` for CLI usage.
- Copy `.env.example` to `.env` before first run if it doesn't exist.

### Testing

- **Unit tests:** `cargo test --lib` (156 tests, no external deps needed)
- **Deployment tests:** `cargo test --test deployment_tests` (5 tests, no external deps needed)
- **Integration tests (`cargo test --test integration_tests`):** have pre-existing compile errors (`parse_backup` method not found); these do not compile as of the current codebase state.
- `cargo clippy` has one pre-existing hard error (`result_large_err` on `FireupError`) plus many warnings.

### Gotchas

- Docker requires `sudo` in this environment (the ubuntu user is not in the docker group).
- The Docker daemon must be started manually: `sudo dockerd &>/tmp/dockerd.log &` — wait ~3 seconds before issuing docker commands.
- Docker uses `fuse-overlayfs` storage driver and `iptables-legacy` in the nested container environment.
- Rust stable (1.85+) is required; some transitive dependencies use edition 2024 features.
- The `tests/.firestore-data/` directory contains pre-generated sample data; the Firebase Emulator + Node.js data generator is optional.
