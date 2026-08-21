# DEVELOP.md

Architecture and working notes for building protopie itself. Read [`PRD.md`](PRD.md) first for the product intent; this file is about how the Rust codebase should be structured and how to work on it day to day. See [`AGENTS.md`](AGENTS.md) for where to find reference source for every dependency this repo touches.

## 1. Repo layout (planned)

protopie is a single Cargo workspace. Proposed crates, following nocodo's separation of concerns (business logic never lives in the app-shell crate):

```
protopie/
├── Cargo.toml                  # workspace
├── gui/                        # akar-based desktop shell: project selector, prompt screen, process status
├── host/                       # orchestrator: project lifecycle, npm process management (install/dev/build),
│                                #   file I/O, validated scope resolution and project edits
├── agents/                     # candidate planning/generation prompt strategies, llm-sdk calls,
│                                #   single-shot generation, deterministic response processing
├── store/                      # SQLite: projects, brain-dumps, task records, FTS + vector search
├── scaffold/                   # deterministic project scaffolder: emits the initial SolidJS + Vite +
│                                #   Tailwind + DaisyUI + Solid Router project structure, mock-data proxy
├── evals/                      # record/test harness, checked-in recordings (prompt + raw response + assertions)
└── epics/                      # one file per epic — see AGENTS.md for the convention

fixtures/                       # gitignored, repo root — named sample SolidJS apps evals cases run against
```

Nothing here is final — expect this to shift as ordered evolution scenarios make parts of the candidate pipeline load-bearing or unnecessary.

## 2. Crate responsibilities

- **`gui`** — the only crate that touches akar. A project selector (top nav) and a main screen (prompt box + generation status + "open in browser" link/button once the dev server is healthy). Owns the winit event loop and wgpu surface per akar's own model (akar owns neither the window nor the event loop — see `~/Projects/akar/DEVELOP.md`). No business logic here; it calls into `host`.
- **`host`** — owns everything that isn't an LLM call: spawning and watching `npm install` / `npm run dev` / `npm run build` per project (as child processes, capturing stdout/stderr), selecting bounded context, validating proposed file operations, staging changes, and publishing only validated project states. `tree-sitter-typescript` extraction and splicing is an initial candidate for symbol-local edits, not the assumed answer for rename, deletion, imports, file creation, or other structural work. This is the only crate allowed to touch generated-project files or spawn processes. Agents never get raw file or shell access — they receive exactly the context `host` selects and return only operations permitted by the active output contract.
- **`agents`** — single-shot prompt strategies and deterministic response processing. A PO planner and the candidate labels (`solid-state`, `routing`, `component-hierarchy`, `ui-design`, `mock-data`, …, per `PRD.md` §7.3) are introduced only when evolution scenarios demonstrate that the separation improves outcomes. Every strategy builds a prompt from host-selected scope and context, calls `llm-sdk`, strips transport noise such as fences/`<think>` blocks, validates the response schema, and returns it to `host`. No agent here holds a multi-turn conversation or a tool-calling loop.
- **`store`** — SQLite, one DB per user (in the OS data dir), scoped per project. Tables for raw brain-dumps, generated task records (label, scope, instructions, status, resulting file+range, provenance back to the brain-dump), and retrieval indexes: FTS5 for keyword search, `sqlite-vec` for embeddings (see nocodo's `internal-docs/DECISION_PROVENANCE_GRAPH.md` for the schema/pipeline shape this is modeled on — chunking, `nodes`/`edges`, `llm_cache` to avoid re-processing unchanged input).
- **`scaffold`** — the deterministic (no LLM) initial project generator: `npm create vite` equivalent wiring for SolidJS + TypeScript, Tailwind + DaisyUI config, Solid Router base setup, and the internal mock-data HTTP proxy skeleton. Every new protopie project starts from this scaffold; generators only ever edit inside it.
- **`evals`** — experimental infrastructure, not product business logic: a `record`/`test` CLI plus checked-in recordings (prompt, prompt version, raw LLM response, call metadata, assertions) under `evals/recordings/`. Each recording is a case that names a sample app under the gitignored root-level `fixtures/`. `record` calls the live local model and writes a recording; `test` runs a case's assertions against its stored recording with no network or LLM access. Assertions describe application outcomes and behavior, not which agent or editing mechanism produced them.

## 3. The candidate generation loop

This is the current production hypothesis. Epic 001 begins with a simpler direct single-shot strategy and adds planning or specialist boundaries only in response to observed failures.

1. User submits a brain-dump in `gui`.
2. `host` hands `(brain-dump, project_id)` to `store` to persist, then asks `store` for retrieved related context (FTS + vector search over this project's prior brain-dumps/tasks).
3. If the tested strategy uses a PO pass, `host` calls it with `(brain-dump, selected context)` and validates the returned bounded change plan. Direct generation may omit this step.
4. For each planned change, `host` resolves the scope, selects the permitted output contract, and calls the matching single-shot prompt strategy.
5. `host` applies validated operations to a staged project state, never directly to the last-known-good preview.
6. `host` runs `npm run build` (or `tsc --noEmit` for a fast first pass) and later runtime checks against the staged state.
   - Clean → `host` publishes the staged state; hot-reload picks it up and `gui` shows the project as up to date.
   - Errors → the initial repair candidate maps each error to the most relevant changed scope and re-invokes that prompt strategy once more with the error appended to a fresh prompt. Evaluation determines whether this ownership mapping is reliable. Cap retries (start at 2) before surfacing a plain-language failure in `gui`.
7. `gui` shows a link to the running `npm run dev` server once healthy; opening it launches the user's system browser (see `PRD.md` §8 on why this is v1's answer to "live preview," not an embedded webview).

## 4. Local dev setup

- Requires a recent Rust toolchain and recent Node/npm (the same requirement protopie will state to its own end users).
- `cargo build` at the workspace root builds every crate.
- Generated protopie *projects* are plain npm projects — `cd` into a project's directory and run `npm install`/`npm run dev` manually while developing `host`'s process-management code, to see exactly what protopie itself will be shelling out to.
- No API keys are committed; LLM provider credentials are read from environment variables via `llm-sdk` the same way its own examples do (see `~/Projects/llm-sdk/README.md` Quick Start).

## 5. Evaluation-driven development

The primary architecture test is cumulative evolution, not isolated prompt quality. An ordered scenario begins with a fixed scaffold and applies each brain-dump to the exact project produced by the prior step. Every step declares:

- new observable requirements,
- earlier requirements that must remain true,
- behavior or terminology explicitly superseded by the new request,
- deterministic validation commands and assertions.

The working loop is red/green/refactor at scenario level: expose the next failing change, make the smallest improvement to prompt/context/output contract/host operation, then rerun the complete sequence from a clean scaffold. Tests must not assert that a named agent ran or that a particular source layout was produced; otherwise the harness would protect the proposed architecture instead of evaluating it.

There are two execution layers:

- **Live evaluation** — calls a pinned local model through `llm-sdk`. Capture the model identifier and relevant llama.cpp settings, prompt version, complete assembled prompt, raw response, parsed operations, applied changes, validation output, latency, token counts when available, and retries. Live runs measure model and prompt behavior and are opt-in when a local endpoint is required.
- **Deterministic replay** — starts from the same scaffold and feeds recorded raw responses through the same parser, operation validator, application path, and checks without an LLM or network access. It must reproduce the same terminal source tree and validation outcomes, including safe rejection of malformed responses. Replay tests isolate deterministic machinery and are suitable for ordinary automated testing.

Fixed scenarios invite overfitting. Prompt examples should use domains different from the evaluated app, prompt growth must remain visible in traces, and later epics should add held-out evolution sequences before a strategy is treated as general.

Component-level tests still matter inside that larger loop:

- **Scaffold** — deterministic, so test it deterministically: generate a project, assert the exact file tree and that `npm install && npm run build` succeeds, in CI.
- **Prompt strategies** — test response validation and host application with recorded responses. Evaluate prompt quality only in live runs; do not make ordinary unit tests depend on stochastic output.
- **Error-feedback loop** — inject a known-bad generation, assert `host` maps the resulting `tsc`/build error to the right generator scope and that the retry prompt contains the error text.
- **`store` retrieval** — assert that FTS/vector queries actually surface the intended prior brain-dump for a handful of hand-built multi-turn prompt sequences; this is the part most likely to silently degrade as it's the least visible to the user.

## 6. Conventions carried over from nocodo/akar worth keeping

- **Deterministic scaffolding, LLM only fills defined gaps** (nocodo's schema-codegen pattern).
- **Agents get scoped context, never raw file/bash access** — the host assembles context; the agent never goes looking for it.
- **Single-shot, low-temperature, example-driven prompts for code generation** — no multi-turn tool loops for anything that writes code.
- **Errors as fresh scoped prompts, not conversation continuations** — matches nocodo's praxis-v1 lesson that scoped, structured gap-feedback beats dumping full context back at an agent.
- **Architecture assertions stay out of outcome tests** — PO roles, generator labels, and tree-sitter are candidates evaluated by the same cumulative scenarios.
- **Every live result is replayable** — prompt and response transparency is required evidence, not optional debugging output.
- **One epic file per unit of work in `epics/`**, following akar's format (`Status`, `Goal`, `Introduction` with Problem/Goal/Out-of-scope/Output, and a reference-sources table with local paths) — see `AGENTS.md`.
