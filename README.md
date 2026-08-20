# protopie

A Rust desktop app for prototyping business workflow apps by typing what you want.

You brain-dump what you need — no chat, no back-and-forth — and protopie generates and maintains a real SolidJS + TailwindCSS + DaisyUI single-page app, running live in your browser, that you keep shaping over days and weeks.

## What it does

- You describe a screen, flow, or feature in plain language.
- protopie turns that into a bounded set of generated changes. The eventual split between planning, routing, state, component, styling, and mock-data prompts is determined by cumulative evolution tests rather than fixed in advance.
- Generated code is checked (`npm run build` / `tsc`) before you ever see it; build errors are fixed automatically by re-generating the specific part that broke.
- Your app runs via `npm run dev`; protopie gives you a link to open it in your browser, and changes show up as you keep prompting.
- All data in a prototype comes from a local mock JSON data source served over an internal proxy — so the app is RESTful from day one and easy to point at a real backend later.

## What it's for (and not for)

Built for: auth screens, list views, detail views, filter bars, and navigation (top/bottom/side) — the shape of most internal business tools and workflow apps.

Not (yet) built for: content-heavy sites, marketing pages, CMS-style layouts.

## Stack

- **Host app**: Rust, using [akar](../akar) for its own native GUI (no Electron/Tauri/webview — a project selector and a prompt screen, rendered with a GPU immediate-mode renderer).
- **Generated projects**: TypeScript, [SolidJS](https://www.solidjs.com/), Solid Router, TailwindCSS, DaisyUI, Vite.
- **LLM access**: [llm-sdk](../llm-sdk) (multi-provider Rust SDK — Claude, Gemini, Grok, GLM, Groq, OpenAI, local llama.cpp, etc.).
- **Storage**: SQLite (per-project brain-dumps, task history, FTS + vector search for retrieving prior context).

## Development approach

protopie is being designed through ordered evolution scenarios rather than isolated code-generation demos. One scaffolded app receives successive brain-dumps that add, rename, contradict, delete, and structurally replace earlier work. Tests assert observable outcomes and preservation of non-superseded behavior, leaving the PO role, generator boundaries, and editing mechanism free to change as failures reveal what structure is actually needed.

Live local-model runs capture their complete prompts, raw responses, parsed operations, changes, and validation results. A deterministic replay suite feeds those recorded responses through the same deterministic machinery from the same scaffold without an LLM, separating host/editing regressions from model and prompt quality. See [`epics/001-evolution-replay-harness.md`](epics/001-evolution-replay-harness.md) for the deliberately small first experiment.

## Status

Early design and evaluation-harness phase. See [`PRD.md`](PRD.md) for the product plan, [`DEVELOP.md`](DEVELOP.md) for architecture and how to work on this repo, and [`AGENTS.md`](AGENTS.md) if you're a coding agent working on protopie itself.

## Lineage

protopie is architecturally informed by [nocodo](../nocodo) (an earlier, adjacent agents-driven full-stack builder) and reuses [akar](../akar) and [llm-sdk](../llm-sdk) from the same author. See `PRD.md` §2 for what protopie deliberately keeps and deliberately drops from nocodo's own design history.
