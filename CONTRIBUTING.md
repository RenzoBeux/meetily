# Contributing to Murmur

Thanks for your interest in contributing! This repository is a personal fork of [Zackriya-Solutions/meeting-minutes](https://github.com/Zackriya-Solutions/meeting-minutes) with a restructured, backend-free architecture — issues and pull requests are welcome.

## Development Workflow

### Branch Strategy

- `main` is the single long-lived branch.
- Create short-lived branches from `main` using the `feat/*` or `fix/*` prefix, and open pull requests back into `main`.

### Getting Started

1. Fork the repository and clone your fork:
   ```bash
   git clone https://github.com/YOUR_USERNAME/murmur.git
   ```
2. Create a branch:
   ```bash
   git checkout -b feat/your-feature-name
   ```
3. Set up the toolchain and build — see the [Building guide](docs/BUILDING.md).

### Before Opening a PR

- `cargo check --workspace` passes (run from the repo root).
- `pnpm exec tsc --noEmit` passes (run from `frontend/`).
- The app builds and the affected flow works — for audio/transcription changes, do a real recording smoke test.
- Follow the existing conventions (see [CLAUDE.md](CLAUDE.md)): `anyhow::Result` in Rust, no index/barrel files in TypeScript, audio devices are named "microphone"/"system".

### Continuous Integration

Two GitHub Actions checks run automatically on every pull request into `main` (defined in [`.github/workflows/ci.yml`](.github/workflows/ci.yml)):

- **`Rust tests + clippy`** — `cargo test --workspace` (the enforced gate) plus an informational `cargo clippy` pass.
- **`Frontend typecheck + lint + tests`** — `tsc --noEmit` and `bun test` (the enforced gates) plus `pnpm run lint` (informational).

Both should be green before a PR is merged.

**Maintainers:** enable branch protection on `main` and mark these two check **names** — `Rust tests + clippy` and `Frontend typecheck + lint + tests` — as required status checks, so a red build blocks merges. `cargo test` and `tsc --noEmit` are the enforced gates today; clippy and eslint currently run in `continue-on-error` mode (informational). Before promoting either to a hard gate, clear its existing warning backlog so the switch doesn't immediately break CI (see the notes in `ci.yml`).

### Commit Message Format

```
<type>(<scope>): <subject>
```

Types match the git history: `feat`, `fix`, `docs`, `refactor`, `chore`, `test` — scope is optional, e.g. `feat(diarization): ...`.

## Reporting Issues

Open a GitHub issue with:

- Your OS and GPU (and which build features you used, e.g. CUDA/Vulkan)
- Steps to reproduce and expected behavior
- Relevant logs (run the app via `clean_run_windows.bat` / `./clean_run.sh` to get terminal logs)
- Screenshots if applicable

## License

By contributing, you agree that your contributions will be licensed under the project's MIT License.
