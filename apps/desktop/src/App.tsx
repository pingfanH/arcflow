import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Activity,
  Bluetooth,
  Cable,
  Database,
  FileCode,
  FolderPlus,
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

type FrontendPlatform = "desktop" | "mobile";

type FrontendPlatformResponse = {
  platform: FrontendPlatform;
};

type ExternalControlStatus = {
  running: boolean;
  bindAddress: string | null;
  acceptedSessions: number;
  activeSessions: number;
  controlMode: boolean;
  allowedCapabilities: string[];
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

type OutputDeviceActivationResponse = {
  activeOutputDevices: string[];
};

type PreviewPlaybackStatus = {
  running: boolean;
  deviceId: string | null;
  channelAStrength: number;
  channelBStrength: number;
  intervalMs: number;
};

type StorageStatus = {
  schemaVersion: number;
  databasePath: string;
};

type RuntimeStatus = {
  activeOutputDevices: string[];
  bleOutputQueued: number;
  bleOutputWritten: number;
  bleOutputFailed: number;
  loadedPluginCount: number;
};

type RuntimeEventRecord = {
  sequence: number;
  kind: string;
  message: string;
};

type RuntimeEventsResponse = {
  events: RuntimeEventRecord[];
};

type PluginRegistryEntry = {
  id: string;
  name: string;
  version: string;
  runtime: string;
  apiVersion: string;
  capabilities: string[];
  enabled: boolean;
  bundleRoot: string | null;
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
  { id: "plugins", label: "Plugins", icon: Puzzle },
] as const;

const waveBars = [28, 44, 62, 76, 68, 52, 34, 48, 72, 84, 58, 38];

const mobileFrontendQuery = "(max-width: 767px), (hover: none) and (pointer: coarse)";

const shellClasses = {
  desktop: {
    root: "min-h-screen bg-stone-50 text-zinc-950",
    frame: "flex min-h-screen",
    sideNav:
      "hidden w-20 shrink-0 border-r border-zinc-200 bg-white md:flex md:flex-col md:items-center md:gap-3 md:py-5",
    mobileNav: "hidden",
    content: "flex min-w-0 flex-1 flex-col",
    header:
      "flex flex-wrap items-center justify-between gap-3 border-b border-zinc-200 bg-white px-4 py-3 md:px-6",
    workspace: "grid min-w-0 gap-4 p-4 md:grid-cols-[1.4fr_0.8fr] md:p-6",
    footer: "border-t border-zinc-200 px-4 py-3 text-xs text-zinc-500 md:px-6",
  },
  mobile: {
    root: "min-h-screen bg-zinc-50 pb-24 text-zinc-950",
    frame: "flex min-h-screen",
    sideNav: "hidden",
    mobileNav:
      "fixed inset-x-0 bottom-0 z-20 grid grid-cols-3 gap-1 border-t border-zinc-200 bg-white/95 px-2 pb-[calc(env(safe-area-inset-bottom)+0.5rem)] pt-2 shadow-[0_-8px_24px_rgba(24,24,27,0.08)] backdrop-blur",
    content: "flex min-w-0 flex-1 flex-col",
    header:
      "sticky top-0 z-10 flex flex-wrap items-center justify-between gap-3 border-b border-zinc-200 bg-white/95 px-4 py-3 backdrop-blur",
    workspace: "grid min-w-0 gap-3 p-3 sm:p-4",
    footer: "px-4 pb-4 pt-2 text-xs text-zinc-500",
  },
} satisfies Record<
  FrontendPlatform,
  {
    root: string;
    frame: string;
    sideNav: string;
    mobileNav: string;
    content: string;
    header: string;
    workspace: string;
    footer: string;
  }
>;

const stoppedExternalControl: ExternalControlStatus = {
  running: false,
  bindAddress: null,
  acceptedSessions: 0,
  activeSessions: 0,
  controlMode: false,
  allowedCapabilities: [],
};

const emptyDeviceScan: DeviceScanResponse = {
  adapterStatus: "unsupported",
  devices: [],
};

const fallbackStorageStatus: StorageStatus = {
  schemaVersion: 1,
  databasePath: "",
};

const fallbackRuntimeStatus: RuntimeStatus = {
  activeOutputDevices: [],
  bleOutputQueued: 0,
  bleOutputWritten: 0,
  bleOutputFailed: 0,
  loadedPluginCount: 0,
};

const emptyRuntimeEvents: RuntimeEventsResponse = {
  events: [],
};

const stoppedPreviewPlayback: PreviewPlaybackStatus = {
  running: false,
  deviceId: null,
  channelAStrength: 0,
  channelBStrength: 0,
  intervalMs: 100,
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

function viewportFrontendPlatform(): FrontendPlatform {
  if (typeof window === "undefined") {
    return "desktop";
  }

  return window.matchMedia(mobileFrontendQuery).matches ? "mobile" : "desktop";
}

function normalizeFrontendPlatform(platform: string): FrontendPlatform {
  return platform === "mobile" ? "mobile" : "desktop";
}

function useFrontendPlatform(): FrontendPlatform {
  const [shellPlatform, setShellPlatform] = useState<FrontendPlatform | null>(null);
  const [viewportPlatform, setViewportPlatform] =
    useState<FrontendPlatform>(viewportFrontendPlatform);

  useEffect(() => {
    const mediaQuery = window.matchMedia(mobileFrontendQuery);
    const updateViewportPlatform = () => {
      setViewportPlatform(mediaQuery.matches ? "mobile" : "desktop");
    };

    updateViewportPlatform();
    mediaQuery.addEventListener("change", updateViewportPlatform);
    invoke<FrontendPlatformResponse>("frontend_platform")
      .then((result) => setShellPlatform(normalizeFrontendPlatform(result.platform)))
      .catch(() => setShellPlatform(null));

    return () => mediaQuery.removeEventListener("change", updateViewportPlatform);
  }, []);

  return shellPlatform === "mobile" || viewportPlatform === "mobile" ? "mobile" : "desktop";
}

function App() {
  const [activeTab, setActiveTab] = useState<(typeof navItems)[number]["id"]>("device");
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [externalStatus, setExternalStatus] =
    useState<ExternalControlStatus>(stoppedExternalControl);
  const [externalBusy, setExternalBusy] = useState(false);
  const [externalControlMode, setExternalControlMode] = useState(false);
  const [lastScan, setLastScan] = useState<DeviceScanResponse | null>(null);
  const [storageStatus, setStorageStatus] = useState<StorageStatus | null>(null);
  const [runtimeStatus, setRuntimeStatus] = useState<RuntimeStatus | null>(null);
  const [runtimeEvents, setRuntimeEvents] = useState<RuntimeEventsResponse | null>(null);
  const [previewStatus, setPreviewStatus] =
    useState<PreviewPlaybackStatus>(stoppedPreviewPlayback);
  const [pluginRegistry, setPluginRegistry] = useState<PluginRegistryResponse | null>(null);
  const [scripts, setScripts] = useState<ScriptsResponse | null>(null);
  const [lastScriptRun, setLastScriptRun] = useState<ScriptRunResponse | null>(null);
  const [scanBusy, setScanBusy] = useState(false);
  const [stopBusy, setStopBusy] = useState(false);
  const [waveBusy, setWaveBusy] = useState(false);
  const [waveError, setWaveError] = useState<string | null>(null);
  const [outputDeviceBusyId, setOutputDeviceBusyId] = useState<string | null>(null);
  const [pluginBusy, setPluginBusy] = useState(false);
  const [pluginBundlePath, setPluginBundlePath] = useState("");
  const [pluginError, setPluginError] = useState<string | null>(null);
  const [scriptBusy, setScriptBusy] = useState(false);
  const [deviceOnline, setDeviceOnline] = useState(false);
  const [channelA, setChannelA] = useState(12);
  const [channelB, setChannelB] = useState(0);
  const frontendPlatform = useFrontendPlatform();
  const shell = shellClasses[frontendPlatform];

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

  const refreshRuntimeStatus = useCallback(() => {
    invoke<RuntimeStatus>("runtime_status")
      .then(setRuntimeStatus)
      .catch(() => setRuntimeStatus(fallbackRuntimeStatus));
  }, []);

  const refreshRuntimeEvents = useCallback(() => {
    invoke<RuntimeEventsResponse>("runtime_events")
      .then(setRuntimeEvents)
      .catch(() => setRuntimeEvents(emptyRuntimeEvents));
  }, []);

  const refreshPreviewPlayback = useCallback(() => {
    invoke<PreviewPlaybackStatus>("preview_playback_status")
      .then(setPreviewStatus)
      .catch(() => setPreviewStatus(stoppedPreviewPlayback));
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
    refreshRuntimeStatus();
    refreshRuntimeEvents();
    refreshPreviewPlayback();
    refreshPluginRegistry();
    refreshScripts();
  }, [
    refreshExternalStatus,
    refreshPreviewPlayback,
    refreshPluginRegistry,
    refreshRuntimeEvents,
    refreshRuntimeStatus,
    refreshScripts,
  ]);

  const startExternalControl = () => {
    setExternalBusy(true);
    invoke<ExternalControlStatus>("start_external_control", {
      controlMode: externalControlMode,
    })
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
        refreshRuntimeStatus();
      })
      .catch(() => {
        setLastScan(emptyDeviceScan);
        setDeviceOnline(false);
      })
      .finally(() => setScanBusy(false));
  };

  const applyOutputDeviceActivation = (result: OutputDeviceActivationResponse) => {
    setRuntimeStatus((current) => ({
      ...(current ?? fallbackRuntimeStatus),
      activeOutputDevices: result.activeOutputDevices,
    }));
    refreshRuntimeEvents();
  };

  const activateOutputDevice = (deviceId: string) => {
    setOutputDeviceBusyId(deviceId);
    invoke<OutputDeviceActivationResponse>("activate_output_device", { deviceId })
      .then(applyOutputDeviceActivation)
      .catch(refreshRuntimeStatus)
      .finally(() => setOutputDeviceBusyId(null));
  };

  const deactivateOutputDevice = (deviceId: string) => {
    setOutputDeviceBusyId(deviceId);
    invoke<OutputDeviceActivationResponse>("deactivate_output_device", { deviceId })
      .then(applyOutputDeviceActivation)
      .catch(refreshRuntimeStatus)
      .finally(() => setOutputDeviceBusyId(null));
  };

  const stopOutput = () => {
    setStopBusy(true);
    invoke<StopOutputResponse>("stop_output")
      .then(() => {
        setPreviewStatus(stoppedPreviewPlayback);
        setWaveError(null);
        refreshRuntimeStatus();
        refreshRuntimeEvents();
      })
      .catch(() => setPreviewStatus(stoppedPreviewPlayback))
      .finally(() => setStopBusy(false));
  };

  const startPreviewPlayback = () => {
    const deviceId = activeOutputDeviceIds[0];
    if (!deviceId) {
      setWaveError("No active output device");
      setPreviewStatus(stoppedPreviewPlayback);
      return;
    }

    setWaveBusy(true);
    invoke<PreviewPlaybackStatus>("start_preview_playback", {
      deviceId,
      channelAStrength: channelA,
      channelBStrength: channelB,
    })
      .then((result) => {
        setPreviewStatus(result);
        setWaveError(null);
        refreshRuntimeStatus();
        refreshRuntimeEvents();
      })
      .catch((error) => {
        setPreviewStatus(stoppedPreviewPlayback);
        setWaveError(errorMessage(error));
        refreshRuntimeStatus();
      })
      .finally(() => setWaveBusy(false));
  };

  const stopPreviewPlayback = () => {
    setWaveBusy(true);
    invoke<PreviewPlaybackStatus>("stop_preview_playback")
      .then((result) => {
        setPreviewStatus(result);
        setWaveError(null);
        refreshRuntimeStatus();
        refreshRuntimeEvents();
      })
      .catch((error) => {
        setPreviewStatus(stoppedPreviewPlayback);
        setWaveError(errorMessage(error));
        refreshRuntimeStatus();
      })
      .finally(() => setWaveBusy(false));
  };

  const installStarterPlugin = () => {
    setPluginBusy(true);
    invoke<PluginRegistryResponse>("install_plugin_manifest", {
      manifestJson: JSON.stringify(starterPluginManifest),
    })
      .then((result) => {
        setPluginRegistry(result);
        setPluginError(null);
        refreshRuntimeStatus();
        refreshRuntimeEvents();
      })
      .catch((error) => {
        setPluginError(errorMessage(error));
        refreshPluginRegistry();
      })
      .finally(() => setPluginBusy(false));
  };

  const installPluginBundle = () => {
    const bundlePath = pluginBundlePath.trim();
    if (!bundlePath) {
      return;
    }

    setPluginBusy(true);
    invoke<PluginRegistryResponse>("install_plugin_bundle", { bundlePath })
      .then((result) => {
        setPluginRegistry(result);
        setPluginBundlePath("");
        setPluginError(null);
        refreshRuntimeStatus();
        refreshRuntimeEvents();
      })
      .catch((error) => {
        setPluginError(errorMessage(error));
        refreshPluginRegistry();
      })
      .finally(() => setPluginBusy(false));
  };

  const setPluginEnabled = (pluginId: string, enabled: boolean) => {
    setPluginBusy(true);
    invoke<PluginRegistryResponse>("set_plugin_enabled", { pluginId, enabled })
      .then((result) => {
        setPluginRegistry(result);
        setPluginError(null);
        refreshRuntimeStatus();
        refreshRuntimeEvents();
      })
      .catch((error) => {
        setPluginError(errorMessage(error));
        refreshPluginRegistry();
      })
      .finally(() => setPluginBusy(false));
  };

  const deletePlugin = (pluginId: string) => {
    setPluginBusy(true);
    invoke<PluginRegistryResponse>("delete_plugin", { pluginId })
      .then((result) => {
        setPluginRegistry(result);
        setPluginError(null);
        refreshRuntimeStatus();
        refreshRuntimeEvents();
      })
      .catch((error) => {
        setPluginError(errorMessage(error));
        refreshPluginRegistry();
      })
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
  const activeOutputDeviceIds = runtimeStatus?.activeOutputDevices ?? [];
  const playing = previewStatus.running;
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
    <main className={shell.root} data-platform={frontendPlatform}>
      <div className={shell.frame}>
        <aside className={shell.sideNav}>
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

        <section className={shell.content}>
          <header className={shell.header}>
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

          <div className={shell.workspace}>
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

              <DeviceList
                activeOutputDeviceIds={activeOutputDeviceIds}
                busyDeviceId={outputDeviceBusyId}
                devices={lastScan?.devices ?? []}
                onActivate={activateOutputDevice}
                onDeactivate={deactivateOutputDevice}
              />

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
                  className="inline-flex h-10 items-center gap-2 rounded-lg bg-teal-600 px-3 text-sm font-semibold text-white hover:bg-teal-700 disabled:cursor-not-allowed disabled:opacity-50"
                  disabled={waveBusy || playing || activeOutputDeviceIds.length === 0}
                  title="Start preview playback"
                  type="button"
                  onClick={startPreviewPlayback}
                >
                  <Play size={16} />
                  Play
                </button>
                <button
                  className="inline-flex h-10 items-center gap-2 rounded-lg border border-zinc-200 bg-white px-3 text-sm font-medium text-zinc-700 hover:bg-zinc-50 disabled:cursor-not-allowed disabled:opacity-50"
                  disabled={waveBusy || !playing}
                  title="Pause preview playback"
                  type="button"
                  onClick={stopPreviewPlayback}
                >
                  <Pause size={16} />
                  Pause
                </button>
              </div>

              {waveError ? (
                <div className="mt-3 rounded-lg border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-700">
                  {waveError}
                </div>
              ) : null}
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
                bundlePath={pluginBundlePath}
                error={pluginError}
                registry={pluginRegistry ?? emptyPluginRegistry}
                runtimes={status?.pluginRuntimes ?? ["wasm", "javascript"]}
                onDelete={deletePlugin}
                onBundlePathChange={setPluginBundlePath}
                onInstallBundle={installPluginBundle}
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
              <StatusPanel
                icon={Activity}
                label="Output"
                value={`${runtimeStatus?.activeOutputDevices.length ?? 0} active - ${
                  runtimeStatus?.bleOutputQueued ?? 0
                }/${runtimeStatus?.bleOutputWritten ?? 0}/${runtimeStatus?.bleOutputFailed ?? 0}`}
                tone="sky"
              />
              <StatusPanel
                icon={Puzzle}
                label="Loaded plugins"
                value={`${runtimeStatus?.loadedPluginCount ?? 0} sandboxed`}
                tone="emerald"
              />
              <RuntimeEventsPanel
                events={runtimeEvents ?? emptyRuntimeEvents}
                onRefresh={refreshRuntimeEvents}
              />
              <ExternalControlPanel
                defaultBind={status?.externalControlBind ?? "127.0.0.1:0"}
                busy={externalBusy}
                controlMode={externalControlMode}
                status={externalStatus}
                onControlModeChange={setExternalControlMode}
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

          <footer className={shell.footer}>
            Local runtime ready
          </footer>
        </section>
      </div>

      <nav className={shell.mobileNav} aria-label="Primary navigation">
        {navItems.map((item) => {
          const Icon = item.icon;
          const selected = activeTab === item.id;
          return (
            <button
              key={item.id}
              className={`grid min-w-0 place-items-center gap-1 rounded-lg px-1.5 py-1.5 text-[11px] font-medium transition ${
                selected ? "bg-teal-50 text-teal-700" : "text-zinc-500 hover:bg-zinc-50"
              }`}
              title={item.label}
              type="button"
              onClick={() => setActiveTab(item.id)}
            >
              <Icon size={18} />
              <span className="max-w-full truncate">{item.label}</span>
            </button>
          );
        })}
      </nav>
    </main>
  );
}

type DeviceListProps = {
  activeOutputDeviceIds: string[];
  busyDeviceId: string | null;
  devices: DeviceSummary[];
  onActivate: (deviceId: string) => void;
  onDeactivate: (deviceId: string) => void;
};

function DeviceList({
  activeOutputDeviceIds,
  busyDeviceId,
  devices,
  onActivate,
  onDeactivate,
}: DeviceListProps) {
  if (devices.length === 0) {
    return (
      <div className="mb-4 rounded-lg border border-dashed border-zinc-200 bg-zinc-50 px-3 py-2 text-sm text-zinc-500">
        No devices
      </div>
    );
  }

  return (
    <div className="mb-4 space-y-2">
      {devices.map((device) => {
        const outputActive = activeOutputDeviceIds.includes(device.id);
        const supportsOutput = device.connected && device.model === "coyoteV3";
        const busy = busyDeviceId === device.id;

        return (
          <div
            key={device.id}
            className="flex min-w-0 items-center gap-3 rounded-lg bg-zinc-50 px-3 py-2"
          >
            <div
              className={`grid size-9 shrink-0 place-items-center rounded-lg ${
                outputActive ? "bg-teal-600 text-white" : "bg-white text-zinc-600"
              }`}
            >
              <Bluetooth size={16} />
            </div>
            <div className="min-w-0 flex-1">
              <div className="flex min-w-0 flex-wrap items-center gap-2">
                <span className="truncate text-sm font-medium text-zinc-950">
                  {deviceModelLabel(device.model)}
                </span>
                <span
                  className={`rounded-md px-1.5 py-0.5 text-[11px] font-medium ${
                    outputActive ? "bg-teal-100 text-teal-700" : "bg-zinc-200 text-zinc-600"
                  }`}
                >
                  {outputActive ? "Output" : device.connected ? "Ready" : "Offline"}
                </span>
              </div>
              <div className="truncate text-xs text-zinc-500">
                {device.id} - {batteryLabel(device.batteryPercent)}
              </div>
            </div>
            <button
              className={`grid size-9 shrink-0 place-items-center rounded-lg border disabled:cursor-not-allowed disabled:opacity-50 ${
                outputActive
                  ? "border-red-200 bg-red-50 text-red-700 hover:bg-red-100"
                  : "border-zinc-200 bg-white text-zinc-700 hover:bg-zinc-100"
              }`}
              disabled={!supportsOutput || busy}
              title={outputActive ? "Deactivate output" : "Activate output"}
              type="button"
              onClick={() => {
                if (outputActive) {
                  onDeactivate(device.id);
                } else {
                  onActivate(device.id);
                }
              }}
            >
              <Power size={16} />
            </button>
          </div>
        );
      })}
    </div>
  );
}

function deviceModelLabel(model: string) {
  if (model === "coyoteV3") {
    return "Coyote V3";
  }

  if (model === "coyoteV2") {
    return "Coyote V2";
  }

  return model;
}

function batteryLabel(percent: number | null) {
  return percent === null ? "Battery --" : `Battery ${percent}%`;
}

type RuntimeEventsPanelProps = {
  events: RuntimeEventsResponse;
  onRefresh: () => void;
};

function RuntimeEventsPanel({ events, onRefresh }: RuntimeEventsPanelProps) {
  const recentEvents = events.events.slice(-3).reverse();

  return (
    <div className="min-w-0 rounded-lg border border-zinc-200 bg-white p-4 shadow-sm">
      <div className="flex items-center gap-3">
        <div className="grid size-10 place-items-center rounded-lg bg-emerald-50 text-emerald-700">
          <Activity size={18} />
        </div>
        <div className="min-w-0 flex-1">
          <div className="text-sm font-medium text-zinc-950">Runtime Events</div>
          <div className="truncate text-sm text-zinc-500">{events.events.length} recent</div>
        </div>
        <button
          className="grid size-9 place-items-center rounded-lg border border-zinc-200 bg-white text-zinc-700 hover:bg-zinc-50"
          title="Refresh runtime events"
          type="button"
          onClick={onRefresh}
        >
          <RefreshCw size={16} />
        </button>
      </div>

      <div className="mt-3 space-y-2">
        {recentEvents.length === 0 ? (
          <div className="rounded-lg bg-zinc-50 px-3 py-2 text-sm text-zinc-500">No events</div>
        ) : (
          recentEvents.map((event) => (
            <div key={event.sequence} className="rounded-lg bg-zinc-50 px-3 py-2">
              <div className="truncate text-sm font-medium text-zinc-950">{event.kind}</div>
              <div className="truncate text-xs text-zinc-500">{event.message}</div>
            </div>
          ))
        )}
      </div>
    </div>
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
          <div className="text-sm font-medium text-zinc-950">Plugin Automations</div>
          <div className="truncate text-sm text-zinc-500">
            {scripts.scripts.length} saved
            {lastRun ? ` - ${lastRun.queued ? "Queued" : "Accepted"}` : ""}
          </div>
        </div>
        <button
          className="grid size-9 place-items-center rounded-lg border border-zinc-200 bg-white text-zinc-700 hover:bg-zinc-50 disabled:cursor-not-allowed disabled:opacity-50"
          disabled={busy}
          title="Refresh plugin automations"
          type="button"
          onClick={onRefresh}
        >
          <RefreshCw size={16} />
        </button>
        <button
          className="grid size-9 place-items-center rounded-lg bg-teal-600 text-white hover:bg-teal-700 disabled:cursor-not-allowed disabled:opacity-50"
          disabled={busy || starterSaved}
          title="Save starter plugin automation"
          type="button"
          onClick={onSaveStarter}
        >
          <Save size={16} />
        </button>
      </div>

      <div className="mt-3 space-y-2">
        {scripts.scripts.length === 0 ? (
          <div className="rounded-lg bg-zinc-50 px-3 py-2 text-sm text-zinc-500">
            No plugin automations saved
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
                  title="Run plugin automation"
                  type="button"
                  onClick={() => onRun(script.scriptId)}
                >
                  <Play size={15} />
                </button>
                <button
                  className="grid size-8 place-items-center rounded-lg border border-red-200 bg-red-50 text-red-700 hover:bg-red-100 disabled:cursor-not-allowed disabled:opacity-50"
                  disabled={busy}
                  title="Delete plugin automation"
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
  bundlePath: string;
  error: string | null;
  registry: PluginRegistryResponse;
  runtimes: string[];
  onBundlePathChange: (path: string) => void;
  onDelete: (pluginId: string) => void;
  onInstallBundle: () => void;
  onInstallStarter: () => void;
  onRefresh: () => void;
  onSetEnabled: (pluginId: string, enabled: boolean) => void;
};

function PluginRegistryPanel({
  busy,
  bundlePath,
  error,
  registry,
  runtimes,
  onBundlePathChange,
  onDelete,
  onInstallBundle,
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
          <div className="text-sm font-medium text-zinc-950">Plugin Runtime</div>
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

      <div className="mt-3 flex gap-2">
        <input
          id="plugin-bundle-path"
          name="pluginBundlePath"
          aria-label="Plugin bundle path"
          className="h-9 min-w-0 flex-1 rounded-lg border border-zinc-200 bg-white px-3 text-sm text-zinc-900 outline-none placeholder:text-zinc-400 focus:border-teal-500"
          disabled={busy}
          placeholder="Bundle path"
          type="text"
          value={bundlePath}
          onChange={(event) => onBundlePathChange(event.currentTarget.value)}
        />
        <button
          className="grid size-9 place-items-center rounded-lg bg-zinc-900 text-white hover:bg-zinc-800 disabled:cursor-not-allowed disabled:opacity-50"
          disabled={busy || bundlePath.trim().length === 0}
          title="Install plugin bundle"
          type="button"
          onClick={onInstallBundle}
        >
          <FolderPlus size={16} />
        </button>
      </div>

      {error ? (
        <div className="mt-3 rounded-lg border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-700">
          {error}
        </div>
      ) : null}

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
                  {plugin.bundleRoot ? (
                    <div className="truncate text-xs text-zinc-500">{plugin.bundleRoot}</div>
                  ) : null}
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
                <button
                  className="grid size-8 place-items-center rounded-lg border border-red-200 bg-red-50 text-red-700 hover:bg-red-100 disabled:cursor-not-allowed disabled:opacity-50"
                  disabled={busy}
                  title="Delete plugin"
                  type="button"
                  onClick={() => onDelete(plugin.id)}
                >
                  <Trash2 size={15} />
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

function errorMessage(error: unknown): string {
  if (error instanceof Error) {
    return error.message;
  }

  if (typeof error === "string") {
    return error;
  }

  return "Operation failed";
}

type ExternalControlPanelProps = {
  defaultBind: string;
  busy: boolean;
  controlMode: boolean;
  status: ExternalControlStatus;
  onControlModeChange: (enabled: boolean) => void;
  onStart: () => void;
  onStop: () => void;
};

function ExternalControlPanel({
  defaultBind,
  busy,
  controlMode,
  status,
  onControlModeChange,
  onStart,
  onStop,
}: ExternalControlPanelProps) {
  const running = status.running;
  const bind = status.bindAddress ?? defaultBind;
  const effectiveControlMode = running ? status.controlMode : controlMode;
  const modeLabel = effectiveControlMode ? "Control" : "Read-only";

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
          <div className="text-sm font-medium text-zinc-950">Plugin Bridge</div>
          <div className="truncate text-sm text-zinc-500">
            {running ? `${bind} - ${modeLabel}` : modeLabel}
          </div>
        </div>
        <button
          className={`grid size-9 place-items-center rounded-lg border text-sm transition ${
            effectiveControlMode
              ? "border-sky-200 bg-sky-50 text-sky-700"
              : "border-zinc-200 bg-white text-zinc-600 hover:bg-zinc-50"
          } disabled:cursor-not-allowed disabled:opacity-50`}
          disabled={busy || running}
          title={effectiveControlMode ? "Use read-only plugin bridge" : "Allow plugin bridge control"}
          type="button"
          onClick={() => onControlModeChange(!controlMode)}
        >
          {effectiveControlMode ? <ToggleRight size={17} /> : <ToggleLeft size={17} />}
        </button>
        <button
          className={`grid size-9 place-items-center rounded-lg border text-sm transition ${
            running
              ? "border-red-200 bg-red-50 text-red-700 hover:bg-red-100"
              : "border-zinc-200 bg-white text-zinc-700 hover:bg-zinc-50"
          } disabled:cursor-not-allowed disabled:opacity-50`}
          disabled={busy}
          title={running ? "Stop plugin bridge" : "Start plugin bridge"}
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
  const inputId = `${label.toLowerCase().replace(/\s+/g, "-")}-strength`;

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
        id={inputId}
        name={inputId}
        aria-label={`${label} strength`}
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
