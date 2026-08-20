# protopie

A Rust desktop app for prototyping business workflow apps by typing what you want.

You brain-dump what you need — no chat, no back-and-forth — and protopie generates and maintains a real SolidJS + TailwindCSS + DaisyUI single-page app, running live in your browser, that you keep shaping over days and weeks.

## What it does

- You describe a screen, flow, or feature in plain language.
- protopie turns that into a set of granular tasks, each handled by a dedicated, constrained code generator (state/context, routing, component hierarchy, UI styling, mock data).
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

## Status

Early design phase. See [`PRD.md`](PRD.md) for the product plan, [`DEVELOP.md`](DEVELOP.md) for architecture and how to work on this repo, and [`AGENTS.md`](AGENTS.md) if you're a coding agent working on protopie itself.

## Lineage

protopie is architecturally informed by [nocodo](../nocodo) (an earlier, adjacent agents-driven full-stack builder) and reuses [akar](../akar) and [llm-sdk](../llm-sdk) from the same author. See `PRD.md` §2 for what protopie deliberately keeps and deliberately drops from nocodo's own design history.
