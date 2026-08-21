# Epic 001: LLM Recording & Assertion Harness

**Status:** Done
**Goal:** Build the foundation for a test suite that records real LLM interactions once and asserts against the stored response many times, proven end to end with a handful of exploratory recordings and at least one recording that has passing assertions against it.

---

## Introduction

### Work Context

**Problem:** Every layer of the intended pipeline in `PRD.md` §7 — brain-dump handling, task splitting, task-dependency resolution, per-task code generation — needs its own kind of test, and all of them share the same shape: send something to the LLM once, capture what comes back, then run many cheap, repeatable assertions against that capture instead of hitting the LLM again per assertion or per CI run. Building each layer's tests ad hoc would duplicate that recording/replay machinery N times and leave no common place to keep it consistent. The repo needs the harness itself before it needs the first real test.

**Goal of this epic:** Stand up the minimal harness — a gitignored folder of named sample SolidJS apps to test against, and an `evals record` command that takes a prompt straight from the command line, calls a live local LLM, and persists the prompt + raw response as a checked-in recording. Recording must not require pre-authoring anything — the point is to be able to freely try out prompts against the system and have every one of them land in git for later inspection. Assertions are added *after the fact*, as a separate step, by anyone (maintainer or coding agent) reading an already-recorded response — never predefined before a recording exists. Prove the whole loop by recording several exploratory prompts against a sample app, then writing and passing assertions against at least one of them via `evals test`, deterministically and with no LLM reachable.

This harness is written and used collaboratively by the maintainer and coding agents working on later epics — so the CLI and file formats must be simple enough for either to record a new prompt, re-record an existing one, or add a new assertion, without needing to touch the harness's own Rust code.

**What we are NOT deciding:** the PO contract, generator labels or boundaries, tree-sitter versus another editing mechanism, the task serialization syntax, task-dependency-resolution schema, code-generation file-op schema, mock-data proxy design, automatic repair, browser automation, SQLite retrieval, the production crate layout, or the `gui`. This epic does not implement task splitting, dependency resolution, or code generation as test *subjects* — it only needs to prove the recording/replay mechanism works for one kind of LLM call (a brain-dump-shaped prompt) so the same mechanism can be reused, unchanged, for those subjects in later epics.

**Output:** A working `evals` crate with `record` and `test` subcommands, a gitignored `fixtures/` folder containing at least one named sample SolidJS app, several checked-in recordings under `evals/recordings/` from real ad-hoc prompts, at least one recording with passing assertions, and a documented authoring workflow. `epics/002-*.md` scoped to the next test type (prompt-to-task splitting) from what this epic's recordings actually revealed.

---

## Reference sources

| System | Local path | Why it matters |
|---|---|---|
| **protopie intent** | `PRD.md`, `DEVELOP.md` | Defines the product constraints the harness must preserve without assuming the final implementation. |
| **nocodo** | `~/Projects/nocodo/agents/DESIGN.md` | RustEngineer's prompt/raw/extracted-output capture is the reference for transparent LLM experiments. |
| **llm-sdk** | `~/Projects/llm-sdk/` | The `record` path calls the local OpenAI-compatible llama.cpp endpoint through this crate. Read `README.md` and `src/` before implementing the adapter. |
| **rustysolid** | `~/Projects/rustysolid/` | Reference for the smallest credible SolidJS + Vite + TypeScript sample app to seed `fixtures/`. |

---

## Design

### Where things live

```
fixtures/                     # gitignored (repo root) — named sample SolidJS apps
  blank/                      #   e.g. a minimal scaffolded app, no brain-dumps applied yet
  ...                         #   more named apps added over time for other stages/edge cases

evals/                        # crate: harness code + checked-in recordings
  src/                        #   record/test CLI, llm-sdk adapter, assertion runner
  recordings/                 #   checked in — one folder per recording
    2026-08-21-add-work-items-screen/
      prompt.toml             #     fixture name, prompt text, prompt version, model/settings, timestamp
      response.json           #     raw response + call metadata (latency, token counts when available)
      assertions.toml         #     optional, added after the fact — absent until someone writes tests for it
```

`fixtures/` holds real npm projects (`node_modules/`, build output) and is local-only by design — it is not evaluation evidence itself, it's the input state a recording points at by name. `evals/recordings/` holds small text/JSON only and is what makes `evals test` reproducible on a clean clone without a live model.

### Recording format (v0)

A recording is a directory (name chosen at record time, e.g. date-prefixed + a short slug) containing:
- `prompt.toml` — which fixture it ran against (by name, resolved under `fixtures/`), the exact prompt text, a prompt version string, and the model/settings used,
- `response.json` — the raw LLM response plus call metadata (latency, token counts when available),
- `assertions.toml` — absent by default. Recording a prompt never creates this file; it's added later, separately, once someone has actually read the response and decided what should be true of it.

Assertion vocabulary starts minimal: substring/keyword presence and absence. Do not build schema or tool-call assertion machinery speculatively — add it in the epic that first needs it (task-splitting or code-gen recordings), since the shape of a task or a file-op record is explicitly undecided here.

### CLI

- `evals record <name> --fixture <fixture> --prompt "<text>"` (or `--prompt-file <path>` for longer brain-dumps) — the primary, low-friction entry point: writes `prompt.toml` and calls the live local model via `llm-sdk`, writing `response.json`, in one step. No pre-existing files required — this is how you try out any prompt against the system. Fails if `<name>` already has a recording unless `--force` is passed (re-recording after a prompt or model change). Requires a reachable local llama.cpp endpoint; never runs automatically as part of `evals test` or CI.
- `evals test [<name>]` — for the given recording (or all recordings that have an `assertions.toml`), loads `response.json` and runs the declared assertions, no network or LLM access, exits non-zero on any failure. Recordings without `assertions.toml` are reported as untested, not as errors. This is the command CI and a coding agent run by default.

---

## Tasks

- [x] Add `fixtures/` to `.gitignore` at the repo root; document (in `evals/README.md` or `DEVELOP.md`) how a named fixture app is created — for now, a documented manual `npm create vite` + Tailwind/DaisyUI setup is fine; do not build the `scaffold` crate to unblock this epic.
- [x] Add one fixture (`fixtures/blank/`) — a minimal running SolidJS + Vite + TypeScript app with no brain-dumps applied.
- [x] Create the `evals` crate skeleton in the workspace with `record` and `test` subcommands (clap or similar), wired into the workspace `Cargo.toml`.
- [x] Define the v0 recording format (`prompt.toml` / `response.json` / optional `assertions.toml`) exactly as described above.
- [x] Implement the `record` path: `evals record <name> --fixture <fixture> --prompt "..."` writes `prompt.toml`, calls `llm-sdk` against the local llama.cpp endpoint, and writes `response.json` with model id/settings/prompt version/latency/token counts (when available) — no other file required beforehand.
- [x] Implement the `test` path: for a recording with `assertions.toml`, load `response.json` and run its declared keyword/content assertions, report pass/fail per assertion, no network or LLM call; recordings without `assertions.toml` report as untested rather than erroring.
- [x] Use `evals record` to actually try out a handful of real prompts against `fixtures/blank/` (an initial brain-dump, and at least one or two variants/edge cases) and check the recordings into git as-is — `top-navigation`, `layout-fresh-project`, `one-pager-saas-fresh-project`.
- [x] Pick one recording and write its first `assertions.toml` by hand, from reading the recorded response — this is the "test against stored response" step, done after the recording exists, not before.
- [x] Verify `evals test` passes repeatably from the checked-in recording alone (no model reachable) and fails clearly when an assertion is violated.
- [x] Document the record/re-record/add-assertions workflow so both the maintainer and a coding agent can do all three without reading the `evals` crate's source first.
- [x] Write `epics/002-*.md` scoped to the next test type from the map in this epic's Introduction (prompt-to-task splitting is the natural next one, since it's the next layer PRD §7.2 depends on) — informed by whatever this epic's recordings actually revealed about the format, not by re-deciding the PO or task-schema questions this epic deferred.
