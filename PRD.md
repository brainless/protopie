# PRD — protopie

## 1. What this is

protopie is a Rust desktop app that lets a non-technical user brain-dump what they want, and turns that into a running, editable SolidJS + TailwindCSS + DaisyUI single-page app — in seconds, not after a long planning phase. The user never writes code and never has a multi-turn chat with an LLM. They type what they want, watch the app update in their browser, and keep typing. The central product bet is not one-shot generation; it is that the same app can remain coherent while successive brain-dumps add, contradict, rename, delete, and structurally replace earlier work.

The target output is **business workflow SPAs**: auth screens, list views, detail views, filter bars, navigation (top/bottom/side). Content-heavy sites, marketing pages, and CMS-style apps are explicitly out of scope for now.

## 2. Why (and what we're deliberately not repeating)

protopie is the second project born out of the same lineage as [nocodo](../nocodo) (a Rust + agents platform for building full-stack apps) and reuses [akar](../akar) (a native, GPU-rendered Rust UI toolkit, no web engine) for its own desktop GUI, and [llm-sdk](../llm-sdk) for talking to LLM providers.

nocodo's own design history is the most important input to this PRD. It went through three phases, documented in `nocodo/internal-docs/`:

1. A full **PO → PM → Epics/Tasks → specialist agents** pipeline (`MIXTURE_OF_AGENTS.md`, `agents/DESIGN.md`) — a coordination layer modeled on a human dev team.
2. An attempt to make that pipeline emit a typed, provenance-carrying spec crate (**praxis**, `MILESTONE_PRAXIS.md`) — real value, but bolted onto a heavy pipeline.
3. A deliberate walk-back (`MILESTONE_PRAXIS_v1.md`): *"many coordination agent roles add overhead without proportional value for an LLM-native product"* and *"code is verifiable, a task status is not."* The replacement is a single conversational agent driving a code generator, checked by a deterministic checker, with generated code (not task/epic status) as the only durable artifact.

protopie starts from lesson 3, not lesson 1. Concretely:

- **At most one planning agent**, not PO+PM+EM. A PO-style planning pass is a candidate for turning a brain-dump plus selected context into granular changes, but it is introduced only if cumulative evolution tests show that direct scoped generation needs it. If present, it remains single-shot and does not negotiate with the user.
- **No agent-to-agent chat.** If a planning pass and multiple generators prove useful, their only channel is a typed, validated change/task record — the same "chats are private, tasks are shared" principle nocodo already validated.
- **No file-level tools for code generators.** A generator is handed host-selected context and returns schema-validated operations within an explicit scope — never bash, arbitrary file access, or an LLM-controlled edit loop. The first output contract may be whole-file replacement inside a small allowlist; narrower AST operations are introduced when evidence supports them. This keeps nocodo's small-model `rust_engineer` discipline: single-shot, narrow scope, example-driven prompt, deterministic post-processing, low temperature.
- **Errors go back into the prompt, not into a retry loop the LLM controls.** Rust-side tooling (`npm run build` / `tsc` / future headless-browser checks) runs deterministically after every generation; failures become structured context appended to a fresh single-shot prompt, not a multi-turn conversation.
- **Praxis is optional and deferred.** nocodo's praxis crate (typed vocabulary + `Unresolved<T>` + provenance) is a strong pattern, but it was built for backend auth/state-machine domains. protopie's domain is UI composition (routes, components, state/context, data bindings), which doesn't map 1:1 onto `Role`/`Permission`/`Entity`. We are not committing to a protopie-specific praxis crate in v1. If, after a few iterations, the same "declare it as a typed static + let a deterministic checker find gaps" shape proves valuable for routes/components/state slices, we introduce a small vocabulary crate then — grown from real need, per nocodo's own rule ("only grows a new variant when a real PRD needed it").
- **Architecture follows evaluation evidence.** PO responsibilities, generator boundaries, scope resolution, and editing mechanisms are hypotheses. They are changed in response to cumulative evolution scenarios, not preserved because they resemble a human software team or looked plausible in a design document.

## 3. Product principles

- **Iterate in the browser, every few seconds.** No "wait for the big plan." Every brain-dump should produce visible, running changes fast. If a generation step can't complete in one deterministic pass, it should fail loudly and specifically, not silently degrade.
- **The user never sees or manages tasks, code, or chat turns.** They see: a prompt box, a running preview, and (later) a way to see what changed. All task/label/generator machinery is internal plumbing, same as nocodo's "the task list *is* the chat list, there is no separate session concept exposed to users."
- **Single-turn, not conversational.** Every LLM call — PO or generator — is one prompt in, one structured response out. Multi-turn tool-calling loops are avoided by design, not just by convention, because they're the main source of drift and cost in the systems we're moving away from.
- **Deterministic scaffold, LLM fills gaps.** Project scaffolding (SolidJS + Vite + Tailwind + DaisyUI project structure, router setup, mock API proxy, base layout) is generated by Rust code, not by an LLM. LLMs only ever fill in or modify well-defined slices inside that scaffold.
- **Mock data first, real backend later.** Every prototype is backed by a local mock data source served over an internal HTTP proxy as plain JSON — never in-memory-only fixtures baked into components. This keeps the generated app honestly RESTful from day one, so pointing it at a real backend later is a config change, not a rewrite.
- **Test evolution, not isolated generations.** Evaluation scenarios apply ordered brain-dumps to the output of earlier steps. Assertions describe observable application behavior, preserved requirements, and explicitly superseded behavior rather than requiring a particular agent topology or source layout.
- **Live discovery, deterministic replay.** Live local-model runs capture the complete prompt, response, parsed operations, project changes, and validation results. Recorded responses are replayed without an LLM so orchestration and editing regressions can be tested independently from model variability.

## 4. Users

Non-technical people prototyping a business workflow app. Assumed to be capable of: downloading a desktop app, installing Node.js/npm once. Assumed to have no interest in, or tolerance for, reading code, resolving merge conflicts, or understanding TypeScript errors — those are protopie's job to prevent or silently fix.

## 5. Scope (v1)

**In scope**
- New project creation and project switching (a project selector).
- Brain-dump prompt input (single text box, no chat thread UI).
- Generated SPA patterns: login/auth screen, list view (table/cards), detail view, filter bar, navigation (top nav, bottom nav, side nav — user's choice or PO's inference).
- SolidJS state/context management, Solid Router routing, component hierarchy, and Tailwind/DaisyUI-based UI styling, produced through constrained generation strategies whose final boundaries are determined through evaluation (see §7).
- A local mock data layer: JSON fixtures per entity, served over an internal HTTP proxy so the generated app always does `fetch()` calls, never imports fixtures directly.
- Automatic `npm install` / `npm run dev` process management by the Rust host; TypeScript/build error detection and automatic re-generation to fix errors.
- Opening the running project's dev server in the user's system browser from protopie's main screen.

**Out of scope for v1**
- Content-heavy sites (blogs, marketing pages, CMS-driven layouts).
- Real backend integration (beyond making the mock layer swap-compatible with a real REST API later).
- Multi-user collaboration, deployment/hosting, version control UI.
- A praxis-equivalent typed vocabulary crate (see §2) — revisit after real usage.
- Embedding the live preview inside protopie's own akar-rendered window (see §8, open question).

## 6. Non-technical user experience

1. User opens protopie, is dropped into a project selector (top nav) plus a main screen.
2. User creates a new project or picks an existing one.
3. Main screen: one prompt box. User types a brain-dump ("I need a CRM with a list of customers, a detail page per customer, and filters by status").
4. protopie stores the brain-dump, selects related project context, proposes a bounded change set, and immediately starts the relevant generation work. Whether that proposal needs a distinct PO pass is determined by the evolution suite rather than exposed to the user.
5. protopie runs `npm install` (first time) and `npm run dev` for the project, watching its output.
6. As soon as the dev server is up and the first pass of generated code builds clean, the main screen shows a link/button to open the app in the system browser.
7. User keeps typing follow-up brain-dumps over the following days/weeks. Each one is treated as new context added to the same project — never a chat reply to a previous turn.
8. If a generation step produces a TypeScript/build error, protopie detects it automatically (via `npm run build` / `tsc`) and re-invokes the relevant generator with the error appended, without asking the user to do anything, before showing the result.

## 7. Agent & code-generation architecture

The pipeline below is the current production hypothesis, not a contract that Epic 001 must confirm. The evaluation harness starts with the simplest single-shot strategy that can attempt a bounded change. Planning roles, specialist labels, tree-sitter edits, and retry ownership are introduced or revised only when cumulative scenarios expose a specific need.

### 7.1 Pipeline

```
User brain-dump
   → stored in SQLite (per project), FTS + vector search indexed
   → PO agent: (brain-dump + retrieved context) → structured task list, each task labeled
       with a generator type + scope (file/slice/new-symbol) + instructions
   → Rust task runner dispatches each task to its generator agent
   → generator agent: single-shot LLM call, narrow scope (one function/component/slice),
       given existing code context (via scoped file reads done by Rust, not by the agent)
   → Rust applies the returned code to the file
   → deterministic check: npm run build / tsc / (later) headless browser check
   → on failure: re-invoke the same generator, single-shot, with the error appended
       (not a chat continuation — a fresh prompt with more context)
   → on success: dev server hot-reloads, user sees it live
```

No agent in this pipeline has bash, arbitrary file, or multi-turn chat access. The Rust host is the only thing that touches the filesystem, runs processes, or assembles prompt context.

### 7.2 Candidate PO agent

A candidate single agent with one responsibility: turn `(raw brain-dump, selected project context)` into a flat list of granular tasks, each tagged with:
- a **generator label** (see §7.3),
- a **target scope** (existing file + symbol, or "new function/component" with a required signature),
- **instructions** distilled from the brain-dump and retrieved context.

If evaluation shows this pass adds value, it remains one LLM call per brain-dump. No back-and-forth with the user occurs inside this call — if the brain-dump is ambiguous, the system produces its best effort; ambiguity is resolved by a later brain-dump. If direct bounded generation performs as well without a distinct planner, the PO remains absent rather than becoming architecture for its own sake.

### 7.3 Candidate code-generation boundaries

The initial candidate is a set of strict, narrow, concern-oriented prompts mirroring nocodo's `rust_engineer` discipline (example-driven prompts, low temperature, deterministic post-processing, single artifact per call). The table is a starting taxonomy to test, not a requirement that every concern become a separate long-lived agent:

| Generator label | Owns |
|---|---|
| `solid-state` | Solid signals/stores and Context providers — one store or one context per call |
| `routing` | Solid Router route definitions and route-level layout wiring |
| `component-hierarchy` | New components and how they compose (props, slots, children) — one component per call |
| `ui-design` | Tailwind/DaisyUI markup and classes inside a given component — style-only edits, never logic |
| `mock-data` | Mock JSON fixtures + the internal proxy route(s) that serve them for an entity |
| *(more added iteratively — auth flow, filter-bar wiring, nav shell — as real prompts surface the need)* |

LLMs are asked for explicit, schema-validated operations over host-selected context — never "make the change you think is right somewhere in this project." Function/component replacement and `tree-sitter-typescript` scope resolution are the first candidates to evaluate. Structural changes such as imports, renames, file creation/deletion, and route rewrites may require different deterministic host operations. The evolution suite decides where each mechanism is reliable.

### 7.4 Error feedback loop

`npm run build` (and `tsc --noEmit` for faster feedback) run after every batch of generator writes. On failure, the Rust host extracts the relevant error(s) and re-invokes the specific generator that owns the failing file/slice with the compiler error appended to a fresh single-shot prompt. No generator ever sees a raw multi-file error dump — only what's relevant to its own scope, same principle as nocodo's praxis-v1 static checker feeding scoped gaps back to the PO rather than the whole spec.

### 7.5 Mock data layer

Every entity the user describes gets: a JSON fixture file, and a route on an internal local HTTP proxy that serves it (with basic filter/query param support). Generated Solid components always `fetch()` from that proxy — never `import` a fixture directly. This is what keeps the generated app "RESTful by construction" and makes swapping in a real backend later a base-URL change, not a rewrite.

### 7.6 Evaluation-driven development

Architecture work is driven by ordered evolution scenarios, beginning with the harness in `epics/001-evolution-replay-harness.md`. A scenario starts from a fixed scaffold and applies each brain-dump to the project produced by the preceding step. Each step declares new observable requirements, requirements that must survive, and behavior explicitly superseded by the new request.

The development loop is:

1. **Red:** the next scenario step fails an assertion or regresses an earlier one.
2. **Green:** change the prompt, selected context, output contract, host operation, or generation boundary until the step passes.
3. **Refactor:** simplify the strategy and rerun the entire ordered sequence from a clean scaffold.

Live evaluation calls a pinned local model through `llm-sdk` and records model settings, prompt version, assembled prompt, raw response, parsed operations, file changes, validation output, latency, and token counts when available. Deterministic replay starts from the same scaffold and feeds recorded raw responses through the same parser, operation validator, application path, and checks without an LLM or network access. Replay tests validate deterministic machinery; live runs evaluate model and prompt quality. Tests assert outcomes rather than internal agent roles so competing strategies can be compared against the same scenario.

## 8. Open questions / risks to resolve early

- **Live preview surface.** akar is a native GPU immediate-mode renderer with no browser/webview embedding of any kind (confirmed: no `wry`/webview dependency anywhere in the akar crate tree). v1 resolves this by having protopie **open the running dev server in the user's system default browser** rather than embedding a preview inside the akar window. Embedding (via `wry`, the same crate Tauri itself wraps) remains a possible v2 direction if a truly single-window experience turns out to matter — but it is not assumed solved by picking akar, and should be treated as separate, explicitly scoped work if pursued.
- **Context retrieval quality.** The FTS + vector search step over stored brain-dumps needs to actually surface the *right* prior context per new prompt, or the PO agent will generate tasks that ignore or contradict earlier decisions. Needs early, real testing with multi-session prompt sequences, not just single-shot demos.
- **When errors can't be fixed by re-prompting the same generator once or twice.** Need a defined ceiling (e.g. 2 retries) and a clear, non-technical way to surface "this part didn't work" to the user without showing them a stack trace.
- **Generator boundary creep.** As real prompts arrive, some changes will span candidate scopes (e.g. a new component needs both a route and a store). If the strategy splits those changes, its planning and ordering must prevent generators from colliding in the same file. Expect the labels in §7.3 to be merged, removed, or re-split in response to evaluation.
- **Praxis-equivalent for UI.** Revisit after a few weeks of real dogfooding: does a typed vocabulary crate for routes/components/state (in the shape of nocodo's `Unresolved<T>` / provenance pattern) pay for itself here, or does the deterministic build-error checker already cover what we need?
- **Cumulative evolution.** A generator that succeeds on a clean scaffold may still lose earlier intent during rename, deletion, contradiction, or structural replacement. Ordered scenarios, rather than isolated prompt samples, are the primary evidence for the architecture.
- **Atomic publication and recovery.** Writing directly into a Vite-watched project can expose half-applied or broken changes before validation finishes. The host needs a tested staging and last-known-good publication strategy before protopie can promise that users never see a broken preview.
- **Generated-code ownership.** v1 must decide whether protopie exclusively owns generated files or preserves arbitrary external edits. The latter materially changes scope resolution, conflict handling, and rollback requirements.
- **Evaluation overfitting.** Fixed prompts can be made to pass by encoding scenario-specific answers. Prompt examples should use different domains, traces should expose prompt growth, and later epics should add held-out evolution sequences before treating a strategy as general.

## 9. Success criteria for v1

- A non-technical user can go from a brain-dump to a running, browser-visible SPA change in under a minute, repeatedly, across a multi-week project.
- Generated code always builds clean before being shown as "done" — the user should never see a broken preview.
- No step in the pipeline requires the user to read or write code, resolve a merge/diff, or have a multi-turn conversation with an LLM.
- A clean run of the cumulative evolution suite preserves all non-superseded assertions across successive additive, contradictory, renaming, deletion, and structural changes; deterministic replay reproduces the same project and validation results without an LLM.
