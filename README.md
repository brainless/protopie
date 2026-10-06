# protopie

Local project chat and live browser preview, built with [akar](https://github.com/brainless/akar).

| Crate | Role |
|---|---|
| `protopie-api` | Wire types shared by server and clients |
| `protopie-server` | Axum HTTP server for projects, chat edits, and Vite preview |
| `protopie-gui` | akar/winit desktop UI; starts `protopie-server` as a child process |

Run `./run.sh` to build and start the GUI and its server. See [MVP guide](docs/mvp.md) for requirements, the project and preview workflow, examples, limits, and verification commands.
