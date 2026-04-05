# Contributing to PureMic Noise Reducer

Thank you for your interest in contributing. This document covers how to set up your environment, submit changes, and follow the project's coding standards.

---

## Table of Contents

- [Getting Started](#getting-started)
- [Development Workflow](#development-workflow)
- [Branching Strategy](#branching-strategy)
- [Commit Messages](#commit-messages)
- [Pull Request Process](#pull-request-process)
- [Code Standards](#code-standards)
- [Reporting Issues](#reporting-issues)

---

## Getting Started

1. Fork the repository on GitHub.
2. Clone your fork:

```bash
git clone https://github.com/<your-username>/PureMic-Noise-Reducer.git
cd PureMic-Noise-Reducer
```

3. Add the upstream remote:

```bash
git remote add upstream https://github.com/cankaytaz/PureMic-Noise-Reducer.git
```

4. Install dependencies and verify the project builds:

```bash
npm install
npm run tauri dev
```

Refer to the [README](README.md) for full platform-specific setup instructions including the macOS audio driver build.

---

## Development Workflow

1. Sync your fork with upstream before starting work:

```bash
git fetch upstream
git checkout main
git merge upstream/main
```

2. Create a feature branch from `main` (see naming conventions below).
3. Make your changes in small, focused commits.
4. Test your changes locally by running the application.
5. Push your branch and open a Pull Request against `upstream/main`.

---

## Branching Strategy

| Prefix | Purpose | Example |
|---|---|---|
| `feature/` | New features or enhancements | `feature/auto-device-detection` |
| `fix/` | Bug fixes | `fix/eq-slider-reset` |
| `refactor/` | Code restructuring without behavior change | `refactor/pipeline-buffer-management` |
| `docs/` | Documentation changes | `docs/update-contributing-guide` |

---

## Commit Messages

```
<type>: <short description>

[optional body]
```

**Types:** `feat`, `fix`, `refactor`, `docs`, `style`, `chore`

**Examples:**

```
feat: add device refresh button to settings modal
fix: prevent accumulator overflow in audio pipeline
docs: add Windows setup instructions to README
```

---

## Pull Request Process

1. **One concern per PR.** Keep pull requests focused on a single change.
2. **Fill out the PR description.** Explain what changed, why, and how you tested it.
3. **Rebase on main** before requesting review:

```bash
git fetch upstream
git rebase upstream/main
```

4. **Ensure the project builds without errors:**

```bash
npm run build
cd src-tauri && cargo build
```

5. **Respond to review feedback** promptly. Do not force-push during review unless asked.
6. **Squash on merge.** PRs are merged via squash merge to keep the history clean.

---

## Code Standards

### TypeScript / React

- Use functional components with hooks. No class components.
- Avoid `any` types. Enable strict mode.
- One component per file. Use named exports.
- Follow the existing directory structure:
  - Reusable UI primitives → `src/components/ui/`
  - Feature components → `src/components/`
  - Custom hooks → `src/hooks/`
  - Utilities and types → `src/lib/`
- Style with Tailwind utility classes. Use `cn()` from `src/lib/utils.ts` for conditional classes.

### Rust

- Run `cargo fmt` before committing.
- Run `cargo clippy` and address all warnings.
- Use `anyhow::Result` for error handling in command functions.
- Use `tracing` macros for logging — no `println!` or `eprintln!`.
- Audio processing code belongs in `src-tauri/src/audio/`.
- Tauri command handlers belong in `src-tauri/src/commands/`.
- Use `AtomicU32` / `AtomicBool` for real-time audio parameters. Never hold a Mutex lock inside an audio callback.

### General

- Do not commit `.env` files, `node_modules/`, `dist/`, or `target/`.
- Do not add new dependencies without discussing them in the PR description.
- Remove unused imports and dead code before submitting.

---

## Reporting Issues

Open an issue on GitHub with the following information:

- **Bug reports:** Steps to reproduce, expected vs. actual behavior, operating system, and audio device details.
- **Feature requests:** A clear description of the desired functionality and the problem it solves.

---

Thank you for helping make PureMic better.
