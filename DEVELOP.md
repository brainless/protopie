# DEVELOP.md

Architecture and working notes for building protopie itself. Read [`PRD.md`](PRD.md) first for the product intent; this file is about how the Rust codebase should be structured and how to work on it day to day. See [`AGENTS.md`](AGENTS.md) for where to find reference source for every dependency this repo touches.

## 1. Repo layout (planned)

protopie is a single Cargo workspace. Proposed crates, following nocodo's separation of concerns (business logic never lives in the app-shell crate):

```
protopie/
├── Cargo.toml                  # workspace
├── gui/                        # akar-based desktop shell: project selector, prompt screen, process status
├── host/                       # orchestrator: project lifecycle, npm process management (install/dev/build),
│                                #   file I/O, tree-sitter-based scoped code extraction/splicing
├── agents/                     # PO agent + generator agents: prompt construction, llm-sdk calls,
│                                #   single-shot generation, deterministic post-processing
├── store/                      # SQLite: projects, brain-dumps, task records, FTS + vector search
├── scaffold/                   # deterministic project scaffolder: emits the initial SolidJS + Vite +
│                                #   Tailwind + DaisyUI + Solid Router project structure, mock-data proxy
└── epics/                      # one file per epic — see AGENTS.md for the convention
```

Nothing here is final — expect this to shift once the PO/generator pipeline is actually implemented and load-bearing.

## 2. Crate responsibilities

- **`gui`** — the only crate that touches akar. A project selector (top nav) and a main screen (prompt box + generation status + "open in browser" link/button once the dev server is healthy). Owns the winit event loop and wgpu surface per akar's own model (akar owns neither the window nor the event loop — see `~/Projects/akar/DEVELOP.md`). No business logic here; it calls into `host`.
- **`host`** — owns everything that isn't an LLM call: spawning and watching `npm install` / `npm run dev` / `npm run build` per project (as child processes, capturing stdout/stderr), scoped code reads (extracting exactly the function/component/slice a generator needs, via `tree-sitter-typescript` — see `~/Projects/tree-sitter-typescript`), and splicing a generator's returned code back into the right place. This is the only crate allowed to touch the filesystem or spawn processes. Agents never get raw file or shell access — they get exactly the slice `host` decides to hand them, and return exactly the replacement `host` decides to write.
- **`agents`** — the PO agent and each generator agent (`solid-state`, `routing`, `component-hierarchy`, `ui-design`, `mock-data`, …, per `PRD.md` §7.3). Each is single-shot: build a prompt from the scope `host` provided plus retrieved context from `store`, call `llm-sdk`, deterministically post-process the response (strip fences/`<think>` blocks, validate it matches the expected shape — a function body, a component, a JSON fixture), return it to `host`. No agent here holds a multi-turn conversation or a tool-calling loop.
- **`store`** — SQLite, one DB per user (in the OS data dir), scoped per project. Tables for raw brain-dumps, generated task records (label, scope, instructions, status, resulting file+range, provenance back to the brain-dump), and retrieval indexes: FTS5 for keyword search, `sqlite-vec` for embeddings (see nocodo's `internal-docs/DECISION_PROVENANCE_GRAPH.md` for the schema/pipeline shape this is modeled on — chunking, `nodes`/`edges`, `llm_cache` to avoid re-processing unchanged input).
- **`scaffold`** — the deterministic (no LLM) initial project generator: `npm create vite` equivalent wiring for SolidJS + TypeScript, Tailwind + DaisyUI config, Solid Router base setup, and the internal mock-data HTTP proxy skeleton. Every new protopie project starts from this scaffold; generators only ever edit inside it.

## 3. The generation loop, concretely

1. User submits a brain-dump in `gui`.
2. `host` hands `(brain-dump, project_id)` to `store` to persist, then asks `store` for retrieved related context (FTS + vector search over this project's prior brain-dumps/tasks).
3. `host` calls the PO agent in `agents` with `(brain-dump, retrieved context)` → gets back a flat task list, each labeled with a generator + scope + instructions.
4. For each task, `host` resolves the scope into an actual code slice (existing function/component to replace, or a signature to implement fresh) and calls the matching generator agent.
5. `host` splices the returned code into the project files.
6. `host` runs `npm run build` (or `tsc --noEmit` for a fast first pass) for the project.
   - Clean → hot-reload picks it up, `gui` shows the project as up to date.
   - Errors → `host` maps each error to the owning generator's scope and re-invokes that generator once more with the error appended to a fresh prompt. Cap retries (start at 2) before surfacing a plain-language failure in `gui`.
7. `gui` shows a link to the running `npm run dev` server once healthy; opening it launches the user's system browser (see `PRD.md` §8 on why this is v1's answer to "live preview," not an embedded webview).

## 4. Local dev setup

- Requires a recent Rust toolchain and recent Node/npm (the same requirement protopie will state to its own end users).
- `cargo build` at the workspace root builds every crate.
- Generated protopie *projects* are plain npm projects — `cd` into a project's directory and run `npm install`/`npm run dev` manually while developing `host`'s process-management code, to see exactly what protopie itself will be shelling out to.
- No API keys are committed; LLM provider credentials are read from environment variables via `llm-sdk` the same way its own examples do (see `~/Projects/llm-sdk/README.md` Quick Start).

## 5. Testing philosophy

- **Scaffold** — deterministic, so test it deterministically: generate a project, assert the exact file tree and that `npm install && npm run build` succeeds, in CI.
- **Generators** — test the post-processing/splicing logic (deterministic) with fixed LLM responses as test input; don't assert on live LLM output in unit tests. Separately, keep a small set of real end-to-end prompts run manually (or in a slow/opt-in CI job) against a live provider to catch prompt-quality regressions.
- **Error-feedback loop** — inject a known-bad generation, assert `host` maps the resulting `tsc`/build error to the right generator scope and that the retry prompt contains the error text.
- **`store` retrieval** — assert that FTS/vector queries actually surface the intended prior brain-dump for a handful of hand-built multi-turn prompt sequences; this is the part most likely to silently degrade as it's the least visible to the user.

## 6. Conventions carried over from nocodo/akar worth keeping

- **Deterministic scaffolding, LLM only fills defined gaps** (nocodo's schema-codegen pattern).
- **Agents get scoped context, never raw file/bash access** — the host assembles context; the agent never goes looking for it.
- **Single-shot, low-temperature, example-driven prompts for code generation** — no multi-turn tool loops for anything that writes code.
- **Errors as fresh scoped prompts, not conversation continuations** — matches nocodo's praxis-v1 lesson that scoped, structured gap-feedback beats dumping full context back at an agent.
- **One epic file per unit of work in `epics/`**, following akar's format (`Status`, `Goal`, `Introduction` with Problem/Goal/Out-of-scope/Output, and a reference-sources table with local paths) — see `AGENTS.md`.
