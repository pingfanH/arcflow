# BLE Smoke Test

This checklist verifies the first desktop BLE slice against a real Coyote V3
device. It is intentionally manual because operating-system Bluetooth
permissions, radio state, and physical device behavior cannot be proven by unit
tests.

## Scope

Covered:

- Desktop Tauri 2 shell.
- Coyote V3 discovery through the native BLE provider.
- Device connection from the shared React device page.
- Battery percentage read after connection.
- Output activation and conservative A/B channel strength preview.
- Stop output.

Not covered yet:

- Mobile BLE backend.
- Long-running playback stability.
- Reconnect recovery after the device is powered off mid-session.
- Real notification stream persistence beyond subscription setup.

## Preconditions

- Use the desktop Tauri app, not a plain browser tab.
- Keep the Coyote V3 device nearby and powered on.
- Enable Bluetooth in the operating system.
- Grant Bluetooth permission to the ArcFlow desktop app if prompted.
- Keep channel strengths low for the first test.
- ArcFlow desktop is configured as a single-instance app. Starting it again
  should focus the existing window instead of opening another BLE session.

Start the desktop dev shell:

```bash
pnpm dev:desktop
```

The app should open a desktop window and show the Device workspace.

## Test Steps

1. Click `Scan`.

   Expected:

   - Adapter status changes away from the unsupported fallback.
   - A Coyote V3 row appears if the device advertises service `0x180C`.
   - The row is initially `Offline` if ArcFlow has discovered but not connected
     to it yet.

2. Click the device row action button to connect.

   Expected:

   - The device row changes to `Ready`.
   - Battery text changes from `Battery --` to `Battery N%` when the battery
     characteristic can be read.
   - Runtime events include `device.connected`.

3. Activate output if the device is not already marked `Output`.

   Expected:

   - The device row changes to `Output`.
   - The runtime output counter shows one queued/written BF safety-limit write.
   - Runtime events include `device.output.activated` and a BLE write event.

4. Set low preview strengths.

   Recommended first values:

   - Channel A: `1` to `3`.
   - Channel B: `0`.

5. Click `Start` in the preview controls.

   Expected:

   - Preview status shows the selected device id.
   - BLE output counters increase while preview windows are queued/written.
   - Runtime events include `wave.preview.started`.

6. Click `Stop`.

   Expected:

   - Preview status returns to stopped.
   - Runtime events include stop/output write activity.
   - The device should stop output promptly.

## Failure Notes

Record these details when a step fails:

- Operating system and version.
- Whether the app received a Bluetooth permission prompt.
- Adapter status shown after `Scan`.
- Device id shown in the row.
- Battery text shown after connection.
- Runtime event messages around the failed action.
- Output counters in the `Output` status line.
- Terminal logs from `pnpm dev:desktop`.

Useful first checks:

- If scan never finds the device, restart the device and run `Scan` again.
- If connect fails, confirm the device is not already connected to another app.
- If battery stays unknown, continue testing output; battery read failure should
  not block Coyote V3 output writes.
- If output counters show queued but not written, inspect the runtime event log
  and terminal transport error.
