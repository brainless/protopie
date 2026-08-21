# evals

Recording/replay harness for LLM interactions used across protopie's pipeline. See [`../epics/001-evolution-replay-harness.md`](../epics/001-evolution-replay-harness.md) for why this exists and the full recording-format spec; this file is the day-to-day usage doc for the maintainer and coding agents, so neither has to read `src/` first.

## Fixtures

A fixture is a real, runnable npm project under the gitignored root-level `fixtures/` (not this crate) that a recording points at by name. There is no `scaffold` crate yet, so fixtures are created manually:

1. `npm create vite@latest <name> -- --template solid-ts` inside `fixtures/`.
2. Add Tailwind + DaisyUI following `~/Projects/rustysolid/` as the reference for `vite.config.ts` / `tailwind` wiring.
3. `cd fixtures/<name> && npm install` to confirm it builds.

`fixtures/blank/` is the current minimal fixture — a scaffolded SolidJS + Vite + TypeScript + Tailwind + DaisyUI app with no brain-dumps applied.

## Recording a prompt

Requires a local llama.cpp (or OpenAI-compatible) endpoint reachable at `http://localhost:8080` (override with `--base-url`).

```
cargo run -p evals -- record <name> --fixture <fixture> --prompt "<text>"
```

- `<name>` is the recording's directory name under `evals/recordings/` — pick a short slug (e.g. `top-navigation`), a date prefix is optional, not required by the tool.
- `--fixture <fixture>` must name an existing directory under `fixtures/`.
- `--prompt "<text>"` for short prompts, or `--prompt-file <path>` for longer brain-dumps.
- `--prompt-version`, `--model`, `--temperature`, `--max-tokens` are optional and stored alongside the recording for reproducibility.
- Fails if `<name>` already has a recording — pass `--force` to re-record after changing the prompt or model.

`record` also sends a system prompt: it calls `agents::generate_system_prompt` (a pure, deterministic function — see `agents/src/system_prompt.rs`) with the fixed v1 `TechStack` and, for now, an empty recent-changes history (no real history source exists yet — see `epics/002-system-prompt-generator.md`). This is what makes output SolidJS/Tailwind/DaisyUI-shaped instead of vanilla HTML/CSS/JS.

This writes two files into `evals/recordings/<name>/`:

- `prompt.toml` — fixture name, prompt text, prompt version, the generated system prompt, model/settings, timestamp.
- `response.json` — the raw response plus `latency_ms`.

Both are checked into git as-is; that's the point — every prompt you try lands in git for later inspection, no pre-authoring required.

## Adding assertions

By hand, after reading a recorded `response.json`, add `assertions.toml` next to it. The vocabulary is minimal — substring presence and absence, checked against the response's message content:

```toml
contains = ["navbar", "nav-links"]
not_contains = ["createSignal"]
```

Both keys are optional and default to an empty list.

## Running assertions

```
cargo run -p evals -- test [<name>]
```

- With a `<name>`, runs just that recording's `assertions.toml`.
- With no `<name>`, runs every recording under `evals/recordings/` that has an `assertions.toml`.
- Loads `response.json` directly — no network or LLM access.
- Recordings without `assertions.toml` are reported as untested, not as errors.
- Exits non-zero if any assertion fails (or if a named recording doesn't exist); exits 0 otherwise, including when nothing had assertions to run.
