# Plugin WebSocket Bridge

ArcFlow exposes a local WebSocket bridge under the plugin domain for trusted
external software. The bridge is local-first and capability-gated: clients can
only call methods for capabilities granted during the initial hello.

## Connection

The desktop app starts the bridge through the `start_external_control` Tauri
command. The default bind is local-only:

```text
127.0.0.1:0
```

Port `0` means the operating system picks an available port. Read
`external_control_status` to discover the actual bound address.

## Hello

The first WebSocket text message must be a hello object:

```json
{
  "clientName": "OBS ArcFlow Bridge",
  "protocolVersion": 1,
  "requestedCapabilities": ["device.read"]
}
```

ArcFlow replies with an accepted session:

```json
{
  "protocolVersion": 1,
  "clientName": "OBS ArcFlow Bridge",
  "grantedCapabilities": ["device.read"]
}
```

If any requested capability is not allowed by the active policy, the session is
rejected before JSON-RPC traffic starts.

## Capabilities

Current plugin capability strings:

```text
device.read
wave.generate
wave.control
script.run
script.manage
storage.private
ui.panel
plugin.manage
events.subscribe
external.ws
```

The default local policy is intentionally conservative. Read-only bridge mode
grants `external.ws`, `device.read`, and `events.subscribe`. `external.ws`
identifies the local plugin bridge connection and does not grant host methods by
itself. The UI can explicitly start the bridge in control mode, which
additionally allows `wave.control`, `script.run`, `script.manage`, and
`plugin.manage`. A running bridge keeps the policy it was started with; stop and
restart it to switch modes.

Plugin registry mutations are persisted in SQLite and synchronized into the
Core-owned sandboxed plugin runtime. Plugins still cannot access Bluetooth
directly; enabled plugins are loaded behind the Plugin API boundary.
External clients can also invoke enabled plugin hooks through the same bridge;
any host actions returned by the plugin are executed only through the Plugin API
and still require the plugin manifest capabilities.

Clients granted `events.subscribe` receive pushed WebSocket event envelopes for
runtime events. Polling through `runtime.events` remains available for clients
that only need snapshots. Plugin bridge lifecycle changes emit
`plugin.bridge.started` and `plugin.bridge.stopped` events.

Preview playback uses the same Rust-owned backend session as the shared
desktop/mobile UI. External software does not stream Bluetooth frames directly:
it starts or stops a Core preview session through the bridge, and Core continues
to own sequence allocation, safety limits, and BLE writes.

## JSON-RPC

After hello, requests use JSON-RPC 2.0 envelopes:

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "device.status",
  "params": {
    "deviceId": "coyote-v3"
  }
}
```

Successful responses include `result`; failed responses include `error`.

## Methods

### `device.scan`

Required capability: `device.read`

Refreshes the Rust-owned BLE scan and synchronizes the Coyote V3 output-device
set. The response has the same shape as `scan_devices` in the Tauri IPC
surface, including platform BLE diagnostics when the provider exposes them.

```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "device.scan"
}
```

Example result:

```json
{
  "adapterStatus": "ready",
  "devices": [
    {
      "id": "coyote-v3",
      "model": "coyoteV3",
      "batteryPercent": null,
      "connected": false
    }
  ],
  "diagnostics": {
    "discoveredPeripherals": 1,
    "inspectedPeripherals": 1,
    "matchedAdvertisements": 1,
    "skippedMissingProperties": 0,
    "skippedUnknownPeripherals": 0,
    "matchedSamples": [
      {
        "localName": "47L121000",
        "serviceUuids": ["0x180C"]
      }
    ],
    "skippedUnknownSamples": [],
    "message": "native BLE scan saw 1 peripherals, inspected 1, matched 1, skipped unknown 0, missing properties 0; matched: 47L121000 [0x180C]"
  }
}
```

### `device.status`

Required capability: `device.read`

```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "device.status",
  "params": {
    "deviceId": "coyote-v3"
  }
}
```

### `device.connect`

Required capability: `wave.control`

Connects a discovered BLE device through the Tauri platform provider, reads
battery when available, refreshes the Rust device scan, and synchronizes the
Coyote V3 output-device set. The response has the same shape as `scan_devices`
in the Tauri IPC surface, including platform BLE diagnostics when available.

```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "device.connect",
  "params": {
    "deviceId": "coyote-v3"
  }
}
```

### `device.disconnect`

Required capability: `wave.control`

Stops active output for the device, removes it from the output-device set,
disconnects the BLE session through the Tauri platform provider, and refreshes
the Rust device scan.

```json
{
  "jsonrpc": "2.0",
  "id": 4,
  "method": "device.disconnect",
  "params": {
    "deviceId": "coyote-v3"
  }
}
```

### `wave.stop`

Required capability: `wave.control`

```json
{
  "jsonrpc": "2.0",
  "id": 5,
  "method": "wave.stop",
  "params": {
    "deviceId": "coyote-v3"
  }
}
```

### `device.refreshBattery`

Required capability: `device.read`

Reads the shared Coyote battery characteristic for an already connected device
through the Rust-owned platform BLE provider, then returns the same shape as
`scan_devices` in the Tauri IPC surface. The method fails if the device is not
currently connected.

```json
{
  "jsonrpc": "2.0",
  "id": 5,
  "method": "device.refreshBattery",
  "params": {
    "deviceId": "coyote-v3"
  }
}
```

### `wave.previewStatus`

Required capability: `device.read`

Returns whether the Rust-owned preview playback session is running.

```json
{
  "jsonrpc": "2.0",
  "id": 6,
  "method": "wave.previewStatus"
}
```

### `wave.startPreview`

Required capability: `wave.control`

Starts repeating conservative Coyote V3 preview windows for an active output
device. Activate the device with `device.activateOutput` before starting
preview playback.

```json
{
  "jsonrpc": "2.0",
  "id": 7,
  "method": "wave.startPreview",
  "params": {
    "deviceId": "coyote-v3",
    "channelAStrength": 12,
    "channelBStrength": 0
  }
}
```

### `wave.stopPreview`

Required capability: `wave.control`

Stops the preview session and sends a stop-output command through Rust Core.

```json
{
  "jsonrpc": "2.0",
  "id": 8,
  "method": "wave.stopPreview"
}
```

### `device.activateOutput`

Required capability: `wave.control`

Marks a device as eligible for wave output writes.

```json
{
  "jsonrpc": "2.0",
  "id": 9,
  "method": "device.activateOutput",
  "params": {
    "deviceId": "coyote-v3"
  }
}
```

Successful responses include the current `activeOutputDevices` list.

### `device.deactivateOutput`

Required capability: `wave.control`

Removes a device from wave output writes.

```json
{
  "jsonrpc": "2.0",
  "id": 10,
  "method": "device.deactivateOutput",
  "params": {
    "deviceId": "coyote-v3"
  }
}
```

Successful responses include the current `activeOutputDevices` list.

### `wave.submitWindow`

Required capability: `wave.control`

Submits one Coyote V3 100ms B0 window. `channelA` and `channelB` are either
four points or `null`/omitted to disable that channel.

```json
{
  "jsonrpc": "2.0",
  "id": 11,
  "method": "wave.submitWindow",
  "params": {
    "deviceId": "coyote-v3",
    "sequence": 4,
    "strengthModes": {
      "a": "absolute",
      "b": "unchanged"
    },
    "aStrength": 8,
    "bStrength": 0,
    "channelA": [
      { "periodMs": 10, "strength": 0 },
      { "periodMs": 10, "strength": 10 },
      { "periodMs": 10, "strength": 20 },
      { "periodMs": 10, "strength": 30 }
    ],
    "channelB": null
  }
}
```

Supported strength modes are `unchanged`, `increase`, `decrease`, and
`absolute`.

### `script.run`

Required capability: `script.run`

```json
{
  "jsonrpc": "2.0",
  "id": 9,
  "method": "script.run",
  "params": {
    "scriptId": "script.demo"
  }
}
```

### `script.list`

Required capability: `script.manage`

```json
{
  "jsonrpc": "2.0",
  "id": 10,
  "method": "script.list"
}
```

### `script.upsert`

Required capability: `script.manage`

The document is validated by the Rust script compiler before it is stored.

```json
{
  "jsonrpc": "2.0",
  "id": 11,
  "method": "script.upsert",
  "params": {
    "scriptId": "script.demo",
    "documentJson": "{\"id\":\"script.demo\",\"version\":1,\"steps\":[{\"type\":\"wait\",\"durationMs\":250}]}"
  }
}
```

### `script.delete`

Required capability: `script.manage`

```json
{
  "jsonrpc": "2.0",
  "id": 12,
  "method": "script.delete",
  "params": {
    "scriptId": "script.demo"
  }
}
```

### `runtime.status`

Required capability: `device.read`

Returns active output device ids, BLE output worker counters, and the number of
plugins currently loaded into the sandboxed runtime.

```json
{
  "jsonrpc": "2.0",
  "id": 10,
  "method": "runtime.status"
}
```

### `runtime.events`

Required capability: `events.subscribe`

Returns the recent in-memory runtime event log for script and BLE output worker
events.

```json
{
  "jsonrpc": "2.0",
  "id": 11,
  "method": "runtime.events"
}
```

Subscribed clients also receive pushed event envelopes:

```json
{
  "method": "event",
  "params": {
    "sequence": 1,
    "kind": "script.completed",
    "message": "script `script.demo` completed 2 steps"
  }
}
```

### `runtime.plugins`

Required capability: `plugin.manage`

Returns the plugins currently loaded into the sandboxed WASM/JavaScript runtime.

```json
{
  "jsonrpc": "2.0",
  "id": 10,
  "method": "runtime.plugins"
}
```

Example result:

```json
{
  "plugins": [
    {
      "pluginId": "dev.arcflow.example",
      "runtime": "wasm",
      "entry": "dist/plugin.wasm",
      "bundleRoot": "/Users/me/ArcFlow/plugins/dev.arcflow.example"
    }
  ]
}
```

### `plugin.invokeHook`

Required capability: `plugin.manage`

Invokes one hook on an enabled WASM/JavaScript plugin. The external client
provides the hook payload, but any returned host actions are executed through
the Rust-owned Plugin API, so plugin manifest capabilities still apply.

```json
{
  "jsonrpc": "2.0",
  "id": 12,
  "method": "plugin.invokeHook",
  "params": {
    "pluginId": "dev.arcflow.example",
    "hook": "external.connected",
    "payload": {
      "clientName": "OBS ArcFlow Bridge"
    }
  }
}
```

Example result:

```json
{
  "pluginId": "dev.arcflow.example",
  "hook": "external.connected",
  "actionCount": 1,
  "results": [
    {
      "key": "lastExternalPayload",
      "stored": true
    }
  ]
}
```

### `plugin.registry`

Required capability: `plugin.manage`

```json
{
  "jsonrpc": "2.0",
  "id": 11,
  "method": "plugin.registry"
}
```

### `plugin.installManifest`

Required capability: `plugin.manage`

```json
{
  "jsonrpc": "2.0",
  "id": 12,
  "method": "plugin.installManifest",
  "params": {
    "manifestJson": "{\"id\":\"dev.arcflow.example\",\"name\":\"Example\",\"version\":\"0.1.0\",\"runtime\":\"wasm\",\"entry\":\"dist/plugin.wasm\",\"apiVersion\":\"1\",\"capabilities\":[\"device.read\"]}"
  }
}
```

### `plugin.installBundle`

Required capability: `plugin.manage`

Loads a plugin bundle directory containing `manifest.json` and the manifest
entry file, then persists the bundle root for runtime loading.

```json
{
  "jsonrpc": "2.0",
  "id": 13,
  "method": "plugin.installBundle",
  "params": {
    "bundlePath": "/Users/me/ArcFlow/plugins/dev.arcflow.example"
  }
}
```

### `plugin.setEnabled`

Required capability: `plugin.manage`

```json
{
  "jsonrpc": "2.0",
  "id": 14,
  "method": "plugin.setEnabled",
  "params": {
    "pluginId": "dev.arcflow.example",
    "enabled": true
  }
}
```

### `plugin.delete`

Required capability: `plugin.manage`

Deletes a persisted plugin and unloads it from the sandboxed runtime if it was
enabled.

```json
{
  "jsonrpc": "2.0",
  "id": 15,
  "method": "plugin.delete",
  "params": {
    "pluginId": "dev.arcflow.example"
  }
}
```

## Minimal Client Flow

External software should treat the bridge as a plugin-domain control surface,
not as a Bluetooth socket. The client asks for capabilities during hello, then
sends JSON-RPC requests. Rust Core keeps ownership of device state, safety
limits, sequence allocation, plugin capability checks, and BLE writes.

This example uses the WebSocket implementation available in modern browser
runtimes and recent Node.js releases. Replace the URL with the address returned
by the Tauri `external_control_status` command after starting the bridge.

```js
const socket = new WebSocket("ws://127.0.0.1:49152");
let nextId = 1;
const pending = new Map();

function request(method, params) {
  const id = nextId++;
  socket.send(
    JSON.stringify({
      jsonrpc: "2.0",
      id,
      method,
      params,
    }),
  );

  return new Promise((resolve, reject) => {
    pending.set(id, { resolve, reject });
  });
}

socket.addEventListener("message", (event) => {
  const message = JSON.parse(event.data);

  if (message.method === "event") {
    console.log("runtime event", message.params);
    return;
  }

  const waiter = pending.get(message.id);
  if (!waiter) {
    return;
  }
  pending.delete(message.id);

  if (message.error) {
    waiter.reject(new Error(message.error.message));
  } else {
    waiter.resolve(message.result);
  }
});

socket.addEventListener("open", async () => {
  socket.send(
    JSON.stringify({
      clientName: "Example Plugin Bridge Client",
      protocolVersion: 1,
      requestedCapabilities: [
        "device.read",
        "wave.control",
        "events.subscribe",
        "plugin.manage",
      ],
    }),
  );
});

socket.addEventListener("message", async function onHello(event) {
  const hello = JSON.parse(event.data);
  if (!hello.grantedCapabilities) {
    return;
  }

  socket.removeEventListener("message", onHello);

  console.log("granted", hello.grantedCapabilities);

  const scan = await request("device.scan");
  console.log("scan", scan);

  const status = await request("device.status", { deviceId: "coyote-v3" });
  console.log("device", status);

  await request("device.connect", { deviceId: "coyote-v3" });
  await request("device.activateOutput", { deviceId: "coyote-v3" });
  await request("wave.startPreview", {
    deviceId: "coyote-v3",
    channelAStrength: 8,
    channelBStrength: 0,
  });

  await request("plugin.invokeHook", {
    pluginId: "dev.arcflow.example",
    hook: "external.connected",
    payload: { source: "example-client" },
  });

  setTimeout(() => {
    request("wave.stopPreview")
      .then(() => request("device.disconnect", { deviceId: "coyote-v3" }))
      .catch(console.error);
  }, 1000);
});
```

If the bridge was started in read-only mode, the hello above is rejected because
`wave.control` and `plugin.manage` are not granted by that policy. In that mode,
request only `external.ws`, `device.read`, and `events.subscribe`.
