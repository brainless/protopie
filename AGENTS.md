# AGENTS.md — Guide for Coding Agents

This document defines how coding agents (Claude Code, Codex, opencode, or any other agent) work on the **protopie codebase itself**. It is not about the agents protopie runs internally to generate SolidJS code (that architecture is `PRD.md` §7 / `DEVELOP.md`) — it's about you, working on this Rust repo.

Read `PRD.md` and `DEVELOP.md` before touching anything.

## Before starting any task

1. Confirm which epic is active (`epics/` — lowest-numbered file without `Status: Done`). Read the full epic before touching any file. Do not rely on `README.md`/`PRD.md`/`DEVELOP.md` for what's actually built — read the epic files and the code directly.
2. If no epic covers the work you're about to do, write one first, following the format in "Epics convention" below, before writing code.
3. Cross-reference `DEVELOP.md` for crate boundaries and architectural constraints (scoped-context-only agents, deterministic scaffold, single-shot generation — do not casually add a tool-calling loop or bash access to a generator agent; that's a deliberate constraint, not an oversight).

## Local source access

The relevant dependencies and design references are cloned locally under `~/Projects/`. Prefer reading these checkouts over web searches, npm/crates.io documentation, or GitHub browsing — they are the local source of truth for internals and undocumented behavior. These are reference-only checkouts, not path dependencies (except where noted).

### This project's own lineage — read first

| Project | Local path | Read first / use it for |
|---|---|---|
| **nocodo** | `~/Projects/nocodo/` | protopie's direct architectural predecessor. Read `agents/DESIGN.md`, `agents/RUNTIME.md` (actually the praxis vocabulary design, despite the name), and `internal-docs/MILESTONE_PRAXIS_v1.md` in particular — that's the design pivot protopie starts from (single PO agent + deterministic checker, not PO/PM/EM/Epics). `internal-docs/DECISION_PROVENANCE_GRAPH.md` is the model for protopie's own SQLite/FTS/vector-search retrieval layer. |
| **akar** | `~/Projects/akar/` | protopie's own desktop GUI toolkit — `gui/` in this repo is built with it. Read `AGENTS.md`, `DEVELOP.md`, and `examples/demo-rust/` before writing any `gui` code. akar has no webview/browser-embedding capability (confirmed — no `wry`/webview dependency anywhere in its crate tree); that's why protopie opens the live preview in the system browser instead of embedding it (`PRD.md` §8). |
| **llm-sdk** | `~/Projects/llm-sdk/` | The Rust crate `agents` in this repo uses for all LLM provider calls (Claude, Gemini, Grok, GLM, Groq, OpenAI, local llama.cpp, etc.). Read `README.md` and `src/` for the trait-based `LlmClient` design before wiring a new agent call. |

### Rust dependencies used directly by this repo

| Project | Local path | Use it for |
|---|---|---|
| **tree-sitter** | `~/Projects/tree-sitter/` | Core parser library `host` uses for scoped code extraction/splicing. |
| **tree-sitter-typescript** | `~/Projects/tree-sitter-typescript/` | The grammar for locating exact function/component slices inside generated `.ts`/`.tsx` files — this is how `host` hands generators a narrow slice instead of a whole file. |
| **tree-sitter-javascript** | `~/Projects/tree-sitter-javascript/` | Same, for any plain `.js`/config files in a scaffolded project. |
| **taffy** | `~/Projects/taffy/` | Layout engine akar itself wraps; read if debugging `gui` layout issues, not a direct protopie dependency. |

### Generated-project stack references

protopie doesn't vendor these — every generated project pulls them from npm — but the following local checkouts are useful for understanding exact APIs, versions, and idioms before writing generator prompts or scaffold templates:

| Project | Local path | Use it for |
|---|---|---|
| **daisyui** | `~/Projects/daisyui/` | Component catalog, class names, and theme/token structure the `ui-design` generator should target. |
| **rustysolid** | `~/Projects/rustysolid/` | A working template combining Rust/Actix + Diesel + SolidJS + Solid Router + Tailwind + DaisyUI. The closest existing reference for how a generated project's `gui`/`admin-gui` should be laid out (Vite config, Tailwind/DaisyUI setup, Solid Router wiring) — read this before writing `scaffold`. |
| **shadcn_ui** | `~/Projects/shadcn_ui/` | Component API ergonomics reference if a generator's output needs a composition pattern beyond what DaisyUI's class-only components give you. |

There is no local clone of SolidJS or TailwindCSS themselves — treat their published docs/npm packages as source of truth for those two, and `rustysolid`/`daisyui` as the concrete "how these fit together" reference.

Do not fetch URLs for anything with a local path above. Read files locally.

## Epics convention

Work is tracked as one Markdown file per epic in `epics/`, numbered `NNN-title.md`, following the format akar uses (`~/Projects/akar/epics/001-exploration-and-architecture.md` is the canonical example):

```
# Epic NNN: Title

**Status:** Not started | In progress | Done
**Goal:** one sentence.

---

## Introduction

### Work Context

**Problem:** what gap this epic closes and why it matters now.
**Goal of this epic:** what "done" looks like, concretely.
**What we are NOT deciding:** explicitly deferred scope, so a coding agent doesn't creep into a later epic's territory.
**Output:** the concrete artifact (a crate, a working generator, a completed epic file for the next one).

---

## Reference sources

A table of local paths (from the tables above, plus anything epic-specific) relevant to this epic's work, so the next agent doesn't have to rediscover them.

---

## Tasks

Ordered, checkable task list.
```

- Epics are the durable plan. `PRD.md`/`README.md`/`DEVELOP.md` describe intent and architecture; they are not updated turn-by-turn as work progresses — epics are.
- Don't mark an epic `Done` until its stated Output actually exists and builds/passes.
- When an epic surfaces work outside its own stated scope, write a new epic for it rather than expanding the current one's scope mid-flight.

## What NOT to do

- Do not give a generator agent (§7 of `PRD.md`) file, bash, or multi-turn tool access — scoped context in, replacement code out, single LLM call. This is a deliberate constraint carried over from nocodo's `rust_engineer` discipline, not an oversight to "fix" by adding tools.
- Do not reintroduce nocodo's earlier PO→PM→EM→Epics/Tasks coordination pipeline (`~/Projects/nocodo/agents/DESIGN.md`) — that design was tried and explicitly walked back in `~/Projects/nocodo/internal-docs/MILESTONE_PRAXIS_v1.md`. protopie's PO agent is single, and task records are the only channel to generators, not a shared multi-agent chat.
- Do not add a praxis-equivalent typed vocabulary crate speculatively — `PRD.md` §2/§8 defers this until real usage shows the need, per nocodo's own "only grows a new variant when a real PRD needed it" discipline.
- Do not attempt to embed a browser/webview inside `gui`'s akar-rendered window without first opening a scoped epic for it — v1 deliberately opens the preview in the system browser (`PRD.md` §8); embedding is real, separate scope, not a drive-by addition.
- Do not bake mock fixture data directly into generated Solid components (`import` of a JSON file). All generated components fetch from the internal mock-data proxy — this is what keeps the generated app swappable to a real backend later.

## Style

- No emojis in source or docs unless explicitly requested.
- No comments unless the WHY is non-obvious. Code should be self-documenting.
- Match the doc style already set in `PRD.md`/`DEVELOP.md` — plain prose, tables for reference data, no filler.
