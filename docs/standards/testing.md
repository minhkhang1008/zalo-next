# Testing Standards

## 1. Layout

| Kind | Location | Example |
| :--- | :--- | :--- |
| Unit | `#[cfg(test)] mod tests` at the bottom of the module | `picker.rs`, `productivity.rs`, `data_dir.rs` |
| Integration (several modules) | `tests/<area>_*.rs` | `tests/message_identity_integration.rs`, `tests/updater_regressions.rs` |
| Packaging / scripts | `tests/*.py` (Python `unittest`) | `tests/native_packaging.py`, `tests/release_notes_test.py` |
| Native helpers | `native/macos/test-*.py` | Emoji/media/history-codec helpers |

Commands:
- `cargo test --locked` runs the Rust suite (~470 tests) without network. The single live call test is
  `#[ignore]`d and must stay that way.
- `cargo test --locked --lib <name>` runs one area.
- `python3 -m unittest tests/release_notes_test.py` runs a script test.

## 2. Rules

- **No network.** Zalo responses are synthetic or redacted fixtures, never live calls. HTTP behaviour
  (Assistant, updater) is tested against a local fake server on `127.0.0.1`.
- **Never touch real user data.** No real `session.json`, no repo-root or Application Support
  `zalo_history.db`. Use `:memory:`, `PrivateTempDir`, or an explicit temp path passed into the function under
  test. Functions that do I/O take a path parameter so tests can redirect them.
- **No real credentials or identifiers** in fixtures. Use obviously fake ids (`user-1`, `thread-1`,
  `msg-0001`).
- **One behaviour per test**, named for that behaviour: `fn <behaviour>_when_<condition>()`. Tag invariant
  checks with `INV-N` in the name or as an assertion comment.
- **Bug fixes start with a failing test** when the bug can be reproduced offline. When it can't (live server,
  GUI visuals, phone sync, calls), say so in the worklog and describe the manual check.
- **Crypto and codec tests are known-answer tests:** a fixed key and input give a fixed output.
- **Migrations** are tested from both a fresh DB and an existing (legacy-shaped) DB.
- **Flaky tests are bugs.** Fix them or quarantine them with a tracked `TODO(<issue>)`.

## 3. What unit tests cannot prove

The worklog must keep these separate from "tests pass":
- the server accepting a request (needs an `error_code 0` from a live run);
- what the phone or another client renders;
- GUI layout and visuals;
- audio and video devices;
- phone sync with a real device;
- Windows and Linux behaviour (CI builds them, but nobody exercises them).

## 4. Performance checks

Changes to polling cadence, repaint triggers, history pagination or media caches should come with a
before/after measurement (hardware, dataset size, numbers). If none was taken, say "not measured".
