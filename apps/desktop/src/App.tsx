import { useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import {
  Activity,
  Bluetooth,
  Cable,
  Gauge,
  Pause,
  Play,
  Puzzle,
  Radio,
  RefreshCw,
  ShieldCheck,
  Square,
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

const navItems = [
  { id: "device", label: "Device", icon: Bluetooth },
  { id: "wave", label: "Wave", icon: Activity },
  { id: "plugins", label: "Plugins", icon: Puzzle },
  { id: "external", label: "External", icon: Wifi },
] as const;

const waveBars = [28, 44, 62, 76, 68, 52, 34, 48, 72, 84, 58, 38];

function App() {
  const [activeTab, setActiveTab] = useState<(typeof navItems)[number]["id"]>("device");
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [deviceOnline, setDeviceOnline] = useState(false);
  const [playing, setPlaying] = useState(false);
  const [channelA, setChannelA] = useState(12);
  const [channelB, setChannelB] = useState(0);

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
  }, []);

  const activeLabel = useMemo(
    () => navItems.find((item) => item.id === activeTab)?.label ?? "Device",
    [activeTab],
  );

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
                title="Refresh device state"
                type="button"
                onClick={() => setDeviceOnline((value) => !value)}
              >
                <RefreshCw size={16} />
                Scan
              </button>
              <button
                className="inline-flex h-10 items-center gap-2 rounded-lg bg-red-600 px-3 text-sm font-semibold text-white hover:bg-red-700"
                title="Stop all output"
                type="button"
                onClick={() => setPlaying(false)}
              >
                <Square size={15} />
                Stop
              </button>
            </div>
          </header>

          <div className="grid gap-4 p-4 md:grid-cols-[1.4fr_0.8fr] md:p-6">
            <section className="rounded-lg border border-zinc-200 bg-white p-4 shadow-sm">
              <div className="mb-4 flex items-start justify-between gap-3">
                <div>
                  <h2 className="text-base font-semibold">Coyote Session</h2>
                  <p className="text-sm text-zinc-500">
                    {deviceOnline ? "Coyote V3 connected" : "No device connected"}
                  </p>
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

            <section className="grid gap-4">
              <StatusPanel
                icon={ShieldCheck}
                label="Safety"
                value={`Wave <= ${status?.maxWaveStrength ?? 30}`}
                tone="emerald"
              />
              <StatusPanel
                icon={Puzzle}
                label="Plugin runtime"
                value={(status?.pluginRuntimes ?? ["wasm", "javascript"]).join(" / ")}
                tone="amber"
              />
              <StatusPanel
                icon={Radio}
                label="External WS"
                value={status?.externalControlBind ?? "127.0.0.1:0"}
                tone="sky"
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

type ChannelControlProps = {
  label: string;
  value: number;
  limit: number;
  onChange: (value: number) => void;
};

function ChannelControl({ label, value, limit, onChange }: ChannelControlProps) {
  return (
    <div className="rounded-lg border border-zinc-200 p-3">
      <div className="mb-3 flex items-center justify-between">
        <div className="flex items-center gap-2 text-sm font-medium">
          <Gauge size={16} className="text-teal-700" />
          {label}
        </div>
        <div className="tabular-nums text-sm font-semibold">{value}</div>
      </div>
      <input
        className="w-full accent-teal-600"
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
    <div className="rounded-lg border border-zinc-200 bg-white p-4 shadow-sm">
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
