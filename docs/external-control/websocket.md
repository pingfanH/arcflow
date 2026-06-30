# External WebSocket Control

ArcFlow exposes a local WebSocket gateway for trusted external software. The
gateway is local-first and capability-gated: clients can only call methods for
capabilities granted during the initial hello.

## Connection

The desktop app starts the gateway through the `start_external_control` Tauri
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

Current capability strings:

```text
device.read
wave.generate
wave.control
script.run
storage.private
ui.panel
plugin.manage
events.subscribe
external.ws
```

The default local policy is intentionally conservative. It grants
`device.read` and `events.subscribe`; broader capabilities such as
`wave.control` and `plugin.manage` require a future explicit approval surface.

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

### `device.status`

Required capability: `device.read`

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

### `wave.stop`

Required capability: `wave.control`

```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "wave.stop",
  "params": {
    "deviceId": "coyote-v3"
  }
}
```

### `wave.submitWindow`

Required capability: `wave.control`

Submits one Coyote V3 100ms B0 window. `channelA` and `channelB` are either
four points or `null`/omitted to disable that channel.

```json
{
  "jsonrpc": "2.0",
  "id": 3,
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
  "id": 4,
  "method": "script.run",
  "params": {
    "scriptId": "script.demo"
  }
}
```

### `plugin.registry`

Required capability: `plugin.manage`

```json
{
  "jsonrpc": "2.0",
  "id": 5,
  "method": "plugin.registry"
}
```

### `plugin.installManifest`

Required capability: `plugin.manage`

```json
{
  "jsonrpc": "2.0",
  "id": 6,
  "method": "plugin.installManifest",
  "params": {
    "manifestJson": "{\"id\":\"dev.arcflow.example\",\"name\":\"Example\",\"version\":\"0.1.0\",\"runtime\":\"wasm\",\"entry\":\"dist/plugin.wasm\",\"apiVersion\":\"1\",\"capabilities\":[\"device.read\"]}"
  }
}
```

### `plugin.setEnabled`

Required capability: `plugin.manage`

```json
{
  "jsonrpc": "2.0",
  "id": 7,
  "method": "plugin.setEnabled",
  "params": {
    "pluginId": "dev.arcflow.example",
    "enabled": true
  }
}
```
