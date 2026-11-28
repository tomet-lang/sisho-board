<!-- Generated from tmtroot/agents.tmt. Edit that, then `tomet export .`. -->

# Sisho Board

## Core Philosophy & Principles

### Precedence of Instructions

The rules in this document serve as the project baseline and defaults. Explicit instructions given by the user in a conversation always take precedence over these defaults.

### User-First Design over Developer Convenience

Always prioritize the end-user experience, syntax ergonomics, and API design.
If there is tension between developer or implementation convenience and user simplicity, user simplicity must always win. Never compromise user experience just because an implementation is easier.

### Resist Path-of-Least-Resistance & Legacy Bias

The existing implementation is never a justification or reason for new design choices.

- Do not justify decisions with "it is already implemented this way, so let's stick with it."
- Prioritize architectural beauty, elegance, and long-term correctness over the path of least resistance.
- Understand the existing context thoroughly, but never take shortcuts or settle for lazy patches.

### Transparent Execution

Always state what you intend to do before doing it. Never execute non-trivial actions or commands silently.

## Single Source of Truth & Documentation

### Never edit `.md` directly

All Markdown files (`README.md`, `AGENTS.md`, `CONTRIBUTING.md`) are generated build artifacts, not source files. `CLAUDE.md` is the one exception: it holds only the line `@AGENTS.md`.

- Never edit a generated `.md` file directly.
- Always edit the corresponding `.tmt` file under `tmtroot/`.
- Run `just docs` to regenerate the Markdown artifacts, and verify with `just docs-check`.
- Direct edits to generated `.md` files will be overwritten and will fail lint checks.

## What this is

`sisho-board` is the interactive terminal operating dashboard for the Tomet ecosystem, and provides `sisho-board-sdk` for extensions that emit custom board panes (`--sisho-board-pane`). It runs standalone as `sisho-board` or via `sisho` as `sisho board`.

The rules that follow:

- `sisho-board` depends on no internal `sisho` crate (only `sisho-sdk` for `--sisho-extension-info` and `jobserver-gate`). It communicates with `tomet`, `twrit`, `catasto`, and allowed `sisho-<word>` extensions via process boundaries.
- `sisho-board` never executes any discovered `sisho-<word>` extension unless its trust state in `~/.local/share/sisho/trust.json` (queried via `sisho --list --json`) is `Allowed`.
- The workspace enforces a 4-layer dependency hierarchy (see [.writ.tmt](.writ.tmt)): Layer 0 (`sisho-board-config`, `sisho-board-sdk`, `sisho-board-run`), Layer 1 (`sisho-board-core`, `sisho-board-health`), Layer 2 (`sisho-board-tui`), Layer 3 (`app`). Only `sisho-board-run` may use `std::process`, and only `sisho-board-tui` may use `ratatui` or `crossterm`.

Read `CONTRIBUTING.md` for the contribution workflow, commit conventions, and verification steps.

## Language

Write code comments and documentation in English. Do not use Japanese in code.

## Environment & Tooling

All commands, builds, and tests must run within the Nix development environment (via `direnv` or `nix develop`). Assume this environment is active and rely on the tools provided by the flake, rather than assuming or relying on global host toolchains.

## Commits & Version Control (jj)

The repository uses Jujutsu (`jj`) in colocated mode with Git.

- Favor a branchless workflow. Do not create named Git branches for regular agent tasks.
- All commit messages must follow the convention in `CONTRIBUTING.md`: `type(scope): description`.
- Use `jj describe -m "..."` to set commit messages, and `jj new` to advance to subsequent revisions.
- Always pass `--no-pager` to `jj` subcommands.
- Do not put `Claude-Session:` or `Co-Authored-By:` trailers in commit messages.

