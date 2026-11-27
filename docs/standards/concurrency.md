# Concurrency Standards

## 1. Runtime model

| Piece | Where | Notes |
| :--- | :--- | :--- |
| UI thread | eframe/egui event loop (`src/gui.rs`), ratatui loop (`src/tui.rs`) | Renders and sends commands. Must never block (INV-3) |
| Worker | OS thread `zalo-worker` running a Tokio multi-thread runtime with **2 worker threads** (`spawn_session_worker`, `src/gui/worker.rs`) | Owns network, disk, SQLite, crypto, processes |
| UI → worker | `tokio::sync::mpsc::unbounded_channel::<WorkerCmd>()`, sender stored in `WORKER_TX` | One variant per user intent |
| Worker → UI | `std::sync::mpsc` of `GuiEvent`, wrapped in `GuiTx` | `GuiTx::send` also calls `request_repaint()` |
| Shared session | `Arc<tokio::sync::Mutex<Session>>` inside the worker | The UI receives data through events, never handles |
| Blocking work | `tokio::task::spawn_blocking` (SQLite, decode, unzip, hashing, FFmpeg, PDF helper) | |

The repaint model is event-driven: the UI repaints when an event arrives. Idle CPU work keeps the heartbeat at
3 s when the window is focused, and sort keys are cached. Don't add per-frame work that scales with the number
of threads or messages.

## 2. Rules

- **Nothing that can block runs on the UI thread:** network, file or SQLite access, image decode of large
  files, process spawn, clipboard reads that can stall. Send a `WorkerCmd` and handle the resulting `GuiEvent`.
- **One command, one intent.** Don't add a generic `Do(String)` escape hatch. Every fallible command emits a
  terminal event (success or error). Long tasks emit progress events.
- **Never hold a lock across `.await`.** Keep `tokio::sync::Mutex<Session>` critical sections short, and
  never do network I/O while holding it unless the request needs the session in the same step.
- **No `block_on` inside async code.** Move CPU-heavy or blocking work to `spawn_blocking`.
- **Sends handle a closed channel.** A closed receiver means the app is shutting down: log it and return,
  never panic.
- **Flooding producers rate-limit at the source:** polling cadence, coalescing, debounce (for example the
  145 ms debounce on sticker suggestions). Don't rely on unbounded buffering.
- **Every spawned task has an owner and an exit path.** Tie it to the worker lifetime or a cancellation flag
  (`Arc<AtomicBool>`, as in voice recording). Polling loops must not overlap themselves.
- **Timeouts on every network await.** No unbounded waits.

## 3. Failure policy

| Operation | Policy |
| :--- | :--- |
| UI refresh | Fail open: a stale view beats a frozen window |
| Message send | Fail closed: never show "sent" when the server rejected it. No automatic resend of non-idempotent sends (voice, files) |
| History write | Fail open: losing a cache row must not drop delivery |
| Session/auth | Fail closed: never continue with an expired credential |
| Call signalling | Fail closed: hang up cleanly rather than leave a half-open call |

## 4. Known debt

- The TUI does not use the GUI worker: it opens its own `MessageStore` and spawns its own polling task
  (`tui.rs`). Keep its I/O off the draw loop all the same.
- `src/gui/workspace.rs` still opens SQLite directly in a few context-menu paths. Some use
  `std::thread::spawn`; the "delete conversation" path runs on the UI thread. Route new code through the
  worker and move these paths when you are working in that area anyway.
- The worker runtime is built with `.unwrap()` at startup. That is acceptable (no external data), but do not
  copy the pattern elsewhere.

## 5. Review checklist

- [ ] No blocking call added to UI code. New work goes through `WorkerCmd` → `GuiEvent`.
- [ ] No lock held across `.await`, and no `block_on` in async code.
- [ ] Every new task has an owner, a timeout and an exit path.
- [ ] Every new fallible command has a terminal event.
