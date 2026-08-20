# Epic 001: Exploration & Architecture

**Status:** Not started
**Goal:** Validate protopie's riskiest architectural bets with small throwaway spikes before committing to the full crate layout, and produce a concrete Epic 002 task list. No production code is expected to survive this epic.

---

## Introduction

### Work Context

**Problem:** `PRD.md` and `DEVELOP.md` lay out an architecture (single PO agent → labeled tasks → scoped single-shot generator agents → deterministic build-check → retry loop) built by analogy to nocodo's own design history, but none of it has been run against real code yet. Several pieces carry real risk that's cheaper to de-risk now than after `host`/`agents`/`scaffold` are built around wrong assumptions:

- Can `tree-sitter-typescript` reliably extract and splice back a single function/component body inside a real SolidJS+TSX file, well enough for `host` to hand a generator a narrow slice and apply its response? (`PRD.md` §7.3, `DEVELOP.md` §2 `host`)
- Does a single-shot, example-driven prompt (no tool loop) actually produce usable SolidJS state/context, routing, and component code from `llm-sdk`, closely mirroring nocodo's `rust_engineer` discipline but for a different language/framework?
- What does a scaffolded SolidJS + Vite + TailwindCSS + DaisyUI + Solid Router project need to contain at minimum, and does `rustysolid`'s layout transfer directly or need adjustment for a protopie-generated (not hand-written) project?
- What does the mock-data proxy need to look like concretely (a tiny local HTTP server serving JSON fixtures with basic filter/query support), and can a scaffolded project's `fetch()` calls target it cleanly?

**Goal of this epic:** Produce short, throwaway spikes (not durable crates) answering each question above, plus a set of Architecture Decision Records capturing what was learned, and a completed `epics/002-*.md` that can be handed to a coding agent to start real implementation.

**What we are NOT deciding:** the `gui`/akar UI itself, the SQLite/FTS/vector-search retrieval layer, the exact set of generator labels beyond the four named in `PRD.md` §7.3, or any praxis-equivalent typed vocabulary. These are deferred to later epics.

**Output:** A short `ADR-*.md` (or a single `DECISIONS.md`) capturing what was learned from each spike, and `epics/002-*.md` written and ready to hand off.

---

## Reference sources

| System | Local path | Why it matters |
|---|---|---|
| **nocodo** | `~/Projects/nocodo/` | `agents/DESIGN.md`, `internal-docs/MILESTONE_PRAXIS_v1.md` — the single-agent, deterministic-checker pipeline protopie's architecture is modeled on. `agents/src/code_extractor/` — nocodo's own tree-sitter-based scoped extraction, the closest existing reference for the `host` spike. |
| **llm-sdk** | `~/Projects/llm-sdk/` | The SDK all agent spikes call through. Read `README.md` Quick Start and `examples/` before writing a spike prompt harness. |
| **tree-sitter-typescript** | `~/Projects/tree-sitter-typescript/` | Grammar for the TSX extraction/splicing spike. |
| **rustysolid** | `~/Projects/rustysolid/` | Reference SolidJS + Solid Router + Tailwind + DaisyUI project layout for the scaffold spike. |
| **daisyui** | `~/Projects/daisyui/` | Component/class catalog the `ui-design` generator spike should target. |
| **akar** | `~/Projects/akar/` | Not needed for this epic's spikes (no GUI work here), but read `epics/001-exploration-and-architecture.md` there as the format this file follows. |

---

## Tasks

- [ ] Spike: given a real `.tsx` file, use `tree-sitter-typescript` to extract a single function/component's exact source range, hand it (plus surrounding imports) to an LLM call via `llm-sdk` as "replace this body," and splice the response back in. Confirm round-trip correctness on at least 3 real SolidJS component shapes (a signal-based component, a Context provider, a Solid Router route).
- [ ] Spike: single-shot prompt harness for each of the four v1 generator labels (`solid-state`, `routing`, `component-hierarchy`, `ui-design`) against a small set of hand-written test scopes; capture prompt + raw response + extracted code for each, following nocodo's transparent-output convention.
- [ ] Spike: scaffold a minimal SolidJS + Vite + TypeScript + Tailwind + DaisyUI + Solid Router project by hand (informed by `rustysolid`), confirm `npm install && npm run build && npm run dev` all work cleanly, and note exactly what config files/boilerplate a deterministic `scaffold` crate would need to emit.
- [ ] Spike: a minimal local HTTP proxy serving one JSON fixture with basic query-param filtering, and a scaffolded component's `fetch()` call hitting it successfully.
- [ ] Spike: feed a deliberately broken generation through `tsc`/`npm run build`, confirm the resulting error can be mapped back to the specific generator/scope that owns it, and that a re-prompt with the error appended actually fixes it in a single retry for at least one real case.
- [ ] Write up findings as ADRs (one per spike, pass/fail + what changed from the PRD's assumption).
- [ ] Write `epics/002-*.md` scoping the first real implementation epic (likely: `scaffold` crate + one working generator end-to-end, per `DEVELOP.md`'s "prove the loop with one thing first" lesson from nocodo's own praxis-v1 migration discipline).
