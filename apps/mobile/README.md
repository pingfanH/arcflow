# ArcFlow Mobile

ArcFlow mobile targets use Tauri 2.

The first mobile target should be initialized from the shared Tauri app in `apps/desktop`:

```sh
pnpm --filter @arcflow/desktop tauri android init
pnpm --filter @arcflow/desktop tauri ios init
```

Keep platform-specific mobile files thin. Shared UI belongs in React packages, and all Bluetooth, protocol, wave, storage, and plugin logic belongs in Rust crates.
