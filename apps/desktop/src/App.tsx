import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Activity,
  Bluetooth,
  Cable,
  Database,
  FileCode,
  Gauge,
  Pause,
  Play,
  Power,
  Puzzle,
  Radio,
  RefreshCw,
  Save,
  ShieldCheck,
  Square,
  ToggleLeft,
  ToggleRight,
  Trash2,
  Wifi,
  Zap,
} from "lucide-react";
import "./App.css";

type AppStatus = {
  protocolCrate: string;
  pluginRuntimes: string[];
  externalControlBind: string;
  maxChannelStrength: number;
  maxWaveStrength: number;
};

type ExternalControlStatus = {
  running: boolean;
  bindAddress: string | null;
  acceptedSessions: number;
  activeSessions: number;
};

type DeviceSummary = {
  id: string;
  model: string;
  batteryPercent: number | null;
  connected: boolean;
};

type DeviceScanResponse = {
  adapterStatus: string;
  devices: DeviceSummary[];
};

type StopOutputResponse = {
  stoppedDevices: string[];
};

type StorageStatus = {
  schemaVersion: number;
  databasePath: string;
};

type PluginRegistryEntry = {
  id: string;
  name: string;
  version: string;
  runtime: string;
  apiVersion: string;
  capabilities: string[];
  enabled: boolean;
};

type PluginRegistryResponse = {
  plugins: PluginRegistryEntry[];
};

type ScriptDocumentRecord = {
  scriptId: string;
  documentJson: string;
};

type ScriptsResponse = {
  scripts: ScriptDocumentRecord[];
};

type ScriptRunResponse = {
  scriptId: string;
  queued: boolean;
};

const navItems = [
  { id: "device", label: "Device", icon: Bluetooth },
  { id: "wave", label: "Wave", icon: Activity },
  { id: "scripts", label: "Scripts", icon: FileCode },
  { id: "plugins", label: "Plugins", icon: Puzzle },
  { id: "external", label: "External", icon: Wifi },
] as const;

const waveBars = [28, 44, 62, 76, 68, 52, 34, 48, 72, 84, 58, 38];

const stoppedExternalControl: ExternalControlStatus = {
  running: false,
  bindAddress: null,
  acceptedSessions: 0,
  activeSessions: 0,
};

const emptyDeviceScan: DeviceScanResponse = {
  adapterStatus: "unsupported",
  devices: [],
};

const fallbackStorageStatus: StorageStatus = {
  schemaVersion: 1,
  databasePath: "",
};

const emptyPluginRegistry: PluginRegistryResponse = {
  plugins: [],
};

const emptyScripts: ScriptsResponse = {
  scripts: [],
};

const starterPluginManifest = {
  id: "dev.arcflow.pulse-tools",
  name: "Pulse Tools",
  version: "0.1.0",
  runtime: "wasm",
  entry: "dist/plugin.wasm",
  apiVersion: "1",
  capabilities: ["device.read", "storage.private"],
};

const starterScriptDocument = {
  id: "script.demo.wait",
  version: 1,
  steps: [
    { type: "wait", durationMs: 250 },
    { type: "deviceStatus", deviceId: "coyote-v3" },
  ],
};

function App() {
  const [activeTab, setActiveTab] = useState<(typeof navItems)[number]["id"]>("device");
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [externalStatus, setExternalStatus] =
    useState<ExternalControlStatus>(stoppedExternalControl);
  const [externalBusy, setExternalBusy] = useState(false);
  const [lastScan, setLastScan] = useState<DeviceScanResponse | null>(null);
  const [storageStatus, setStorageStatus] = useState<StorageStatus | null>(null);
  const [pluginRegistry, setPluginRegistry] = useState<PluginRegistryResponse | null>(null);
  const [scripts, setScripts] = useState<ScriptsResponse | null>(null);
  const [lastScriptRun, setLastScriptRun] = useState<ScriptRunResponse | null>(null);
  const [scanBusy, setScanBusy] = useState(false);
  const [stopBusy, setStopBusy] = useState(false);
  const [pluginBusy, setPluginBusy] = useState(false);
  const [scriptBusy, setScriptBusy] = useState(false);
  const [deviceOnline, setDeviceOnline] = useState(false);
  const [playing, setPlaying] = useState(false);
  const [channelA, setChannelA] = useState(12);
  const [channelB, setChannelB] = useState(0);

  const refreshExternalStatus = useCallback(() => {
    invoke<ExternalControlStatus>("external_control_status")
      .then(setExternalStatus)
      .catch(() => setExternalStatus(stoppedExternalControl));
  }, []);

  const refreshPluginRegistry = useCallback(() => {
    invoke<PluginRegistryResponse>("plugin_registry")
      .then(setPluginRegistry)
      .catch(() => setPluginRegistry(emptyPluginRegistry));
  }, []);

  const refreshScripts = useCallback(() => {
    invoke<ScriptsResponse>("list_scripts")
      .then(setScripts)
      .catch(() => setScripts(emptyScripts));
  }, []);

  useEffect(() => {
    invoke<AppStatus>("app_status")
      .then(setStatus)
      .catch(() =>
        setStatus({
          protocolCrate: "arcflow-protocol",
          pluginRuntimes: ["wasm", "javascript"],
          externalControlBind: "127.0.0.1:0",
          maxChannelStrength: 20,
          maxWaveStrength: 30,
        }),
      );
    refreshExternalStatus();
    invoke<StorageStatus>("storage_status")
      .then(setStorageStatus)
      .catch(() => setStorageStatus(fallbackStorageStatus));
    refreshPluginRegistry();
    refreshScripts();
  }, [refreshExternalStatus, refreshPluginRegistry, refreshScripts]);

  const startExternalControl = () => {
    setExternalBusy(true);
    invoke<ExternalControlStatus>("start_external_control")
      .then(setExternalStatus)
      .catch(() => setExternalStatus(stoppedExternalControl))
      .finally(() => setExternalBusy(false));
  };

  const stopExternalControl = () => {
    setExternalBusy(true);
    invoke<ExternalControlStatus>("stop_external_control")
      .then(setExternalStatus)
      .catch(() => setExternalStatus(stoppedExternalControl))
      .finally(() => setExternalBusy(false));
  };

  const scanDevices = () => {
    setScanBusy(true);
    invoke<DeviceScanResponse>("scan_devices")
      .then((result) => {
        setLastScan(result);
        setDeviceOnline(result.devices.some((device) => device.connected));
      })
      .catch(() => {
        setLastScan(emptyDeviceScan);
        setDeviceOnline(false);
      })
      .finally(() => setScanBusy(false));
  };

  const stopOutput = () => {
    setStopBusy(true);
    invoke<StopOutputResponse>("stop_output")
      .then(() => setPlaying(false))
      .catch(() => setPlaying(false))
      .finally(() => setStopBusy(false));
  };

  const installStarterPlugin = () => {
    setPluginBusy(true);
    invoke<PluginRegistryResponse>("install_plugin_manifest", {
      manifestJson: JSON.stringify(starterPluginManifest),
    })
      .then(setPluginRegistry)
      .catch(refreshPluginRegistry)
      .finally(() => setPluginBusy(false));
  };

  const setPluginEnabled = (pluginId: string, enabled: boolean) => {
    setPluginBusy(true);
    invoke<PluginRegistryResponse>("set_plugin_enabled", { pluginId, enabled })
      .then(setPluginRegistry)
      .catch(refreshPluginRegistry)
      .finally(() => setPluginBusy(false));
  };

  const saveStarterScript = () => {
    setScriptBusy(true);
    invoke<ScriptsResponse>("upsert_script", {
      scriptId: starterScriptDocument.id,
      documentJson: JSON.stringify(starterScriptDocument),
    })
      .then(setScripts)
      .catch(refreshScripts)
      .finally(() => setScriptBusy(false));
  };

  const runStoredScript = (scriptId: string) => {
    setScriptBusy(true);
    invoke<ScriptRunResponse>("run_script", { scriptId })
      .then(setLastScriptRun)
      .catch(() => setLastScriptRun(null))
      .finally(() => setScriptBusy(false));
  };

  const deleteStoredScript = (scriptId: string) => {
    setScriptBusy(true);
    invoke<ScriptsResponse>("delete_script", { scriptId })
      .then((result) => {
        setScripts(result);
        setLastScriptRun((current) => (current?.scriptId === scriptId ? null : current));
      })
      .catch(refreshScripts)
      .finally(() => setScriptBusy(false));
  };

  const activeLabel = useMemo(
    () => navItems.find((item) => item.id === activeTab)?.label ?? "Device",
    [activeTab],
  );
  const connectedDeviceCount = lastScan?.devices.filter((device) => device.connected).length ?? 0;
  const deviceSubtitle = useMemo(() => {
    if (connectedDeviceCount > 0) {
      return connectedDeviceCount === 1
        ? "1 Coyote device connected"
        : `${connectedDeviceCount} Coyote devices connected`;
    }

    if (lastScan?.adapterStatus === "unsupported") {
      return "BLE adapter not attached";
    }

    return "No device connected";
  }, [connectedDeviceCount, lastScan]);

  return (
    <main className="min-h-screen bg-stone-50 text-zinc-950">
      <div className="flex min-h-screen">
        <aside className="hidden w-20 shrink-0 border-r border-zinc-200 bg-white md:flex md:flex-col md:items-center md:gap-3 md:py-5">
          <div className="mb-4 grid size-10 place-items-center rounded-lg bg-teal-600 text-white">
            <Zap size={20} />
          </div>
          {navItems.map((item) => {
            const Icon = item.icon;
            const selected = activeTab === item.id;
            return (
              <button
                key={item.id}
                className={`grid size-11 place-items-center rounded-lg border text-sm transition ${
                  selected
                    ? "border-teal-600 bg-teal-50 text-teal-700"
                    : "border-transparent text-zinc-500 hover:border-zinc-200 hover:bg-zinc-50"
                }`}
                title={item.label}
                type="button"
                onClick={() => setActiveTab(item.id)}
              >
                <Icon size={19} />
              </button>
            );
          })}
        </aside>

        <section className="flex min-w-0 flex-1 flex-col">
          <header className="flex flex-wrap items-center justify-between gap-3 border-b border-zinc-200 bg-white px-4 py-3 md:px-6">
            <div>
              <div className="text-xs font-medium uppercase tracking-wide text-zinc-500">
                {activeLabel}
              </div>
              <h1 className="text-xl font-semibold text-zinc-950">ArcFlow Control</h1>
            </div>
            <div className="flex items-center gap-2">
              <button
                className="inline-flex h-10 items-center gap-2 rounded-lg border border-zinc-200 bg-white px-3 text-sm font-medium text-zinc-700 hover:bg-zinc-50"
                disabled={scanBusy}
                title="Refresh device state"
                type="button"
                onClick={scanDevices}
              >
                <RefreshCw size={16} />
                Scan
              </button>
              <button
                className="inline-flex h-10 items-center gap-2 rounded-lg bg-red-600 px-3 text-sm font-semibold text-white hover:bg-red-700 disabled:cursor-not-allowed disabled:opacity-60"
                disabled={stopBusy}
                title="Stop all output"
                type="button"
                onClick={stopOutput}
              >
                <Square size={15} />
                Stop
              </button>
            </div>
          </header>

          <div className="grid min-w-0 gap-4 p-4 md:grid-cols-[1.4fr_0.8fr] md:p-6">
            <section className="min-w-0 rounded-lg border border-zinc-200 bg-white p-4 shadow-sm">
              <div className="mb-4 flex items-start justify-between gap-3">
                <div>
                  <h2 className="text-base font-semibold">Coyote Session</h2>
                  <p className="text-sm text-zinc-500">{deviceSubtitle}</p>
                </div>
                <span
                  className={`inline-flex items-center gap-2 rounded-lg px-2.5 py-1 text-xs font-medium ${
                    deviceOnline ? "bg-emerald-50 text-emerald-700" : "bg-zinc-100 text-zinc-600"
                  }`}
                >
                  <span
                    className={`size-2 rounded-full ${deviceOnline ? "bg-emerald-500" : "bg-zinc-400"}`}
                  />
                  {deviceOnline ? "Online" : "Standby"}
                </span>
              </div>

              <div className="grid gap-4 lg:grid-cols-2">
                <ChannelControl
                  label="Channel A"
                  limit={status?.maxChannelStrength ?? 20}
                  value={channelA}
                  onChange={setChannelA}
                />
                <ChannelControl
                  label="Channel B"
                  limit={status?.maxChannelStrength ?? 20}
                  value={channelB}
                  onChange={setChannelB}
                />
              </div>

              <div className="mt-5 h-32 rounded-lg border border-zinc-200 bg-zinc-950 p-3">
                <div className="flex h-full items-end gap-2">
                  {waveBars.map((height, index) => (
                    <div
                      key={index}
                      className="flex-1 rounded-sm bg-teal-400"
                      style={{ height: `${playing ? height : Math.max(8, height / 4)}%` }}
                    />
                  ))}
                </div>
              </div>

              <div className="mt-4 flex flex-wrap gap-2">
                <button
                  className="inline-flex h-10 items-center gap-2 rounded-lg bg-teal-600 px-3 text-sm font-semibold text-white hover:bg-teal-700"
                  title="Start wave output"
                  type="button"
                  onClick={() => setPlaying(true)}
                >
                  <Play size={16} />
                  Play
                </button>
                <button
                  className="inline-flex h-10 items-center gap-2 rounded-lg border border-zinc-200 bg-white px-3 text-sm font-medium text-zinc-700 hover:bg-zinc-50"
                  title="Pause wave output"
                  type="button"
                  onClick={() => setPlaying(false)}
                >
                  <Pause size={16} />
                  Pause
                </button>
              </div>
            </section>

            <section className="grid min-w-0 gap-4">
              <StatusPanel
                icon={ShieldCheck}
                label="Safety"
                value={`Wave <= ${status?.maxWaveStrength ?? 30}`}
                tone="emerald"
              />
              <PluginRegistryPanel
                busy={pluginBusy}
                registry={pluginRegistry ?? emptyPluginRegistry}
                runtimes={status?.pluginRuntimes ?? ["wasm", "javascript"]}
                onInstallStarter={installStarterPlugin}
                onRefresh={refreshPluginRegistry}
                onSetEnabled={setPluginEnabled}
              />
              <ScriptsPanel
                busy={scriptBusy}
                scripts={scripts ?? emptyScripts}
                lastRun={lastScriptRun}
                onDelete={deleteStoredScript}
                onRefresh={refreshScripts}
                onRun={runStoredScript}
                onSaveStarter={saveStarterScript}
              />
              <StatusPanel
                icon={Database}
                label="Storage"
                value={`SQLite schema ${storageStatus?.schemaVersion ?? 1}`}
                tone="zinc"
              />
              <ExternalControlPanel
                defaultBind={status?.externalControlBind ?? "127.0.0.1:0"}
                busy={externalBusy}
                status={externalStatus}
                onStart={startExternalControl}
                onStop={stopExternalControl}
              />
              <StatusPanel
                icon={Cable}
                label="Protocol crate"
                value={status?.protocolCrate ?? "arcflow-protocol"}
                tone="zinc"
              />
            </section>
          </div>

          <footer className="border-t border-zinc-200 px-4 py-3 text-xs text-zinc-500 md:px-6">
            Local runtime ready
          </footer>
        </section>
      </div>
    </main>
  );
}

type ScriptsPanelProps = {
  busy: boolean;
  scripts: ScriptsResponse;
  lastRun: ScriptRunResponse | null;
  onDelete: (scriptId: string) => void;
  onRefresh: () => void;
  onRun: (scriptId: string) => void;
  onSaveStarter: () => void;
};

function ScriptsPanel({
  busy,
  scripts,
  lastRun,
  onDelete,
  onRefresh,
  onRun,
  onSaveStarter,
}: ScriptsPanelProps) {
  const starterSaved = scripts.scripts.some((script) => script.scriptId === starterScriptDocument.id);

  return (
    <div className="min-w-0 rounded-lg border border-zinc-200 bg-white p-4 shadow-sm">
      <div className="flex items-center gap-3">
        <div className="grid size-10 place-items-center rounded-lg bg-sky-50 text-sky-700">
          <FileCode size={18} />
        </div>
        <div className="min-w-0 flex-1">
          <div className="text-sm font-medium text-zinc-950">Scripts</div>
          <div className="truncate text-sm text-zinc-500">
            {scripts.scripts.length} saved
            {lastRun ? ` - ${lastRun.queued ? "Queued" : "Accepted"}` : ""}
          </div>
        </div>
        <button
          className="grid size-9 place-items-center rounded-lg border border-zinc-200 bg-white text-zinc-700 hover:bg-zinc-50 disabled:cursor-not-allowed disabled:opacity-50"
          disabled={busy}
          title="Refresh scripts"
          type="button"
          onClick={onRefresh}
        >
          <RefreshCw size={16} />
        </button>
        <button
          className="grid size-9 place-items-center rounded-lg bg-teal-600 text-white hover:bg-teal-700 disabled:cursor-not-allowed disabled:opacity-50"
          disabled={busy || starterSaved}
          title="Save starter script"
          type="button"
          onClick={onSaveStarter}
        >
          <Save size={16} />
        </button>
      </div>

      <div className="mt-3 space-y-2">
        {scripts.scripts.length === 0 ? (
          <div className="rounded-lg bg-zinc-50 px-3 py-2 text-sm text-zinc-500">
            No scripts saved
          </div>
        ) : (
          scripts.scripts.map((script) => (
            <div key={script.scriptId} className="rounded-lg bg-zinc-50 px-3 py-2">
              <div className="flex items-center gap-2">
                <div className="min-w-0 flex-1">
                  <div className="truncate text-sm font-medium text-zinc-950">
                    {script.scriptId}
                  </div>
                  <div className="truncate text-xs text-zinc-500">
                    {script.documentJson.length} bytes
                  </div>
                </div>
                <button
                  className="grid size-8 place-items-center rounded-lg border border-zinc-200 bg-white text-zinc-700 hover:bg-zinc-100 disabled:cursor-not-allowed disabled:opacity-50"
                  disabled={busy}
                  title="Run script"
                  type="button"
                  onClick={() => onRun(script.scriptId)}
                >
                  <Play size={15} />
                </button>
                <button
                  className="grid size-8 place-items-center rounded-lg border border-red-200 bg-red-50 text-red-700 hover:bg-red-100 disabled:cursor-not-allowed disabled:opacity-50"
                  disabled={busy}
                  title="Delete script"
                  type="button"
                  onClick={() => onDelete(script.scriptId)}
                >
                  <Trash2 size={15} />
                </button>
              </div>
            </div>
          ))
        )}
      </div>
    </div>
  );
}

type PluginRegistryPanelProps = {
  busy: boolean;
  registry: PluginRegistryResponse;
  runtimes: string[];
  onInstallStarter: () => void;
  onRefresh: () => void;
  onSetEnabled: (pluginId: string, enabled: boolean) => void;
};

function PluginRegistryPanel({
  busy,
  registry,
  runtimes,
  onInstallStarter,
  onRefresh,
  onSetEnabled,
}: PluginRegistryPanelProps) {
  const starterInstalled = registry.plugins.some((plugin) => plugin.id === starterPluginManifest.id);

  return (
    <div className="min-w-0 rounded-lg border border-zinc-200 bg-white p-4 shadow-sm">
      <div className="flex items-center gap-3">
        <div className="grid size-10 place-items-center rounded-lg bg-amber-50 text-amber-700">
          <Puzzle size={18} />
        </div>
        <div className="min-w-0 flex-1">
          <div className="text-sm font-medium text-zinc-950">Plugins</div>
          <div className="truncate text-sm text-zinc-500">
            {registry.plugins.length} installed - {runtimes.join(" / ")}
          </div>
        </div>
        <button
          className="grid size-9 place-items-center rounded-lg border border-zinc-200 bg-white text-zinc-700 hover:bg-zinc-50 disabled:cursor-not-allowed disabled:opacity-50"
          disabled={busy}
          title="Refresh plugin registry"
          type="button"
          onClick={onRefresh}
        >
          <RefreshCw size={16} />
        </button>
        <button
          className="inline-flex h-9 items-center rounded-lg bg-teal-600 px-3 text-sm font-semibold text-white hover:bg-teal-700 disabled:cursor-not-allowed disabled:opacity-50"
          disabled={busy || starterInstalled}
          title="Install starter plugin"
          type="button"
          onClick={onInstallStarter}
        >
          Install
        </button>
      </div>

      <div className="mt-3 space-y-2">
        {registry.plugins.length === 0 ? (
          <div className="rounded-lg bg-zinc-50 px-3 py-2 text-sm text-zinc-500">
            No plugins installed
          </div>
        ) : (
          registry.plugins.map((plugin) => (
            <div key={plugin.id} className="rounded-lg bg-zinc-50 px-3 py-2">
              <div className="flex items-start gap-2">
                <div className="min-w-0 flex-1">
                  <div className="truncate text-sm font-medium text-zinc-950">
                    {plugin.name} {plugin.version}
                  </div>
                  <div className="truncate text-xs text-zinc-500">
                    {plugin.runtime} - API {plugin.apiVersion}
                  </div>
                </div>
                <button
                  className={`grid size-8 place-items-center rounded-lg border ${
                    plugin.enabled
                      ? "border-emerald-200 bg-emerald-50 text-emerald-700"
                      : "border-zinc-200 bg-white text-zinc-600"
                  } disabled:cursor-not-allowed disabled:opacity-50`}
                  disabled={busy}
                  title={plugin.enabled ? "Disable plugin" : "Enable plugin"}
                  type="button"
                  onClick={() => onSetEnabled(plugin.id, !plugin.enabled)}
                >
                  {plugin.enabled ? <ToggleRight size={17} /> : <ToggleLeft size={17} />}
                </button>
              </div>
              <div className="mt-2 flex flex-wrap gap-1.5">
                {plugin.capabilities.map((capability) => (
                  <span
                    key={capability}
                    className="rounded-md bg-white px-2 py-1 text-xs text-zinc-600"
                  >
                    {capability}
                  </span>
                ))}
              </div>
            </div>
          ))
        )}
      </div>
    </div>
  );
}

type ExternalControlPanelProps = {
  defaultBind: string;
  busy: boolean;
  status: ExternalControlStatus;
  onStart: () => void;
  onStop: () => void;
};

function ExternalControlPanel({
  defaultBind,
  busy,
  status,
  onStart,
  onStop,
}: ExternalControlPanelProps) {
  const running = status.running;
  const bind = status.bindAddress ?? defaultBind;

  return (
    <div className="min-w-0 rounded-lg border border-zinc-200 bg-white p-4 shadow-sm">
      <div className="flex items-center gap-3">
        <div
          className={`grid size-10 place-items-center rounded-lg ${
            running ? "bg-sky-50 text-sky-700" : "bg-zinc-100 text-zinc-600"
          }`}
        >
          <Radio size={18} />
        </div>
        <div className="min-w-0 flex-1">
          <div className="text-sm font-medium text-zinc-950">External WS</div>
          <div className="truncate text-sm text-zinc-500">{running ? bind : "Stopped"}</div>
        </div>
        <button
          className={`grid size-9 place-items-center rounded-lg border text-sm transition ${
            running
              ? "border-red-200 bg-red-50 text-red-700 hover:bg-red-100"
              : "border-zinc-200 bg-white text-zinc-700 hover:bg-zinc-50"
          } disabled:cursor-not-allowed disabled:opacity-50`}
          disabled={busy}
          title={running ? "Stop external WebSocket" : "Start external WebSocket"}
          type="button"
          onClick={running ? onStop : onStart}
        >
          <Power size={16} />
        </button>
      </div>
      <div className="mt-3 grid grid-cols-2 gap-2 text-xs">
        <div className="rounded-lg bg-zinc-50 px-2.5 py-2">
          <div className="font-medium text-zinc-950">{status.activeSessions}</div>
          <div className="text-zinc-500">Active</div>
        </div>
        <div className="rounded-lg bg-zinc-50 px-2.5 py-2">
          <div className="font-medium text-zinc-950">{status.acceptedSessions}</div>
          <div className="text-zinc-500">Accepted</div>
        </div>
      </div>
    </div>
  );
}

type ChannelControlProps = {
  label: string;
  value: number;
  limit: number;
  onChange: (value: number) => void;
};

function ChannelControl({ label, value, limit, onChange }: ChannelControlProps) {
  return (
    <div className="min-w-0 rounded-lg border border-zinc-200 p-3">
      <div className="mb-3 flex items-center justify-between">
        <div className="flex items-center gap-2 text-sm font-medium">
          <Gauge size={16} className="text-teal-700" />
          {label}
        </div>
        <div className="tabular-nums text-sm font-semibold">{value}</div>
      </div>
      <input
        className="w-full min-w-0 accent-teal-600"
        max={limit}
        min={0}
        type="range"
        value={value}
        onChange={(event) => onChange(Number(event.currentTarget.value))}
      />
      <div className="mt-2 flex justify-between text-xs text-zinc-500">
        <span>0</span>
        <span>{limit}</span>
      </div>
    </div>
  );
}

type StatusPanelProps = {
  icon: typeof ShieldCheck;
  label: string;
  value: string;
  tone: "emerald" | "amber" | "sky" | "zinc";
};

function StatusPanel({ icon: Icon, label, value, tone }: StatusPanelProps) {
  const toneClass = {
    emerald: "bg-emerald-50 text-emerald-700",
    amber: "bg-amber-50 text-amber-700",
    sky: "bg-sky-50 text-sky-700",
    zinc: "bg-zinc-100 text-zinc-700",
  }[tone];

  return (
    <div className="min-w-0 rounded-lg border border-zinc-200 bg-white p-4 shadow-sm">
      <div className="flex items-center gap-3">
        <div className={`grid size-10 place-items-center rounded-lg ${toneClass}`}>
          <Icon size={18} />
        </div>
        <div className="min-w-0">
          <div className="text-sm font-medium text-zinc-950">{label}</div>
          <div className="truncate text-sm text-zinc-500">{value}</div>
        </div>
      </div>
    </div>
  );
}

export default App;
