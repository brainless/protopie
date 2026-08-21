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

The prompt sent is exactly the `--prompt`/`--prompt-file` text — `record` does not add a system prompt or any stack-specific framing. Don't expect SolidJS/Tailwind/DaisyUI-aware output unless the prompt itself asks for it.

This writes two files into `evals/recordings/<name>/`:

- `prompt.toml` — fixture name, prompt text, prompt version, model/settings, timestamp.
- `response.json` — the raw response plus `latency_ms`.

Both are checked into git as-is; that's the point — every prompt you try lands in git for later inspection, no pre-authoring required.

## Adding assertions

Not automated yet. By hand, after reading a recorded `response.json`, add `assertions.toml` next to it (vocabulary starts minimal: substring/keyword presence and absence — see the epic for the exact shape once `evals test` implements it).

## Running assertions

`evals test [<name>]` is **not implemented yet** (see `epics/001-evolution-replay-harness.md` task list). It will load a recording's `response.json` and run its `assertions.toml` with no network or LLM access; recordings without `assertions.toml` will report as untested rather than erroring.
