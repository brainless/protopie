# protopie

Single-column agent chat (sidebar width) built on [akar](https://github.com/brainless/akar).

| Crate | Role |
|---|---|
| `protopie-api` | Wire types shared by server and clients |
| `protopie-server` | Axum HTTP server; `POST /chat` echoes the prompt |
| `protopie-gui` | akar/winit chat UI; spawns `protopie-server` as a child process |

Run: `./run.sh` (builds both crates, then starts the GUI, which launches the server).
