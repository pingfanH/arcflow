# ArcFlow Mobile

ArcFlow mobile targets use Tauri 2 and live in `apps/mobile/src-tauri`.

The mobile shell is intentionally thin:

- `apps/mobile/src-tauri` owns the mobile Tauri config and platform entrypoint.
- `crates/tauri-app` owns the shared Tauri commands, state wiring, and Rust Core integration.
- The React UI is currently built from `apps/desktop` and reused by the mobile shell.

Keep platform-specific mobile files thin. Shared UI belongs in React packages,
and all Bluetooth, protocol, wave, storage, and plugin logic belongs in Rust
crates.
