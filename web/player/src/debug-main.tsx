import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { createRoot } from "react-dom/client";
import {
  createRootRoute,
  createRoute,
  createRouter,
  createHashHistory,
  Link,
  Outlet,
  RouterProvider,
} from "@tanstack/react-router";
import { player, getLab } from "./player";
import { BridgeProvider } from "@evenfall/gba/bridge/react";
import { GameExperience, hasHud } from "./game-experience";
import { gameRegistry } from "./games";
import { GamePlayer } from "./game-player";
import { type Lab } from "./services";
import {
  Emulator,
  SAVE_TYPES,
  type Scenario,
  type FailureBundle,
  type ScenarioExport,
} from "@evenfall/gba";
import { sha256, validateScenario } from "@evenfall/gba/devtools/scenario";
import { download, draw, json, screenshot } from "./media";
import { validateFailureBundle } from "@evenfall/gba/devtools/capture";
import { acquireSaveLock } from "./storage.js";
import "./debug.css";

// Console and browser-test hooks for this diagnostic entry only. The SDK is
// bundled, so there is no standalone /zigba.js to import.
Object.assign(window, { zigbaDebug: { Emulator, acquireSaveLock } });
const act = (fn: () => Promise<unknown>) => () => void fn().catch(player.error);
function FileInput({
  label,
  accept,
  onFile,
}: {
  label: string;
  accept?: string;
  onFile: (f: File) => Promise<unknown>;
}) {
  return (
    <label className="file">
      {label}
      <input
        type="file"
        accept={accept}
        onChange={(e) => {
          const f = e.target.files?.[0];
          e.target.value = "";
          if (f) {
            if (f.size > 40 * 1024 * 1024) {
              player.error(new Error("File exceeds 40 MiB"));
              return;
            }
            void onFile(f).catch(player.error);
          }
        }}
      />
    </label>
  );
}
function Shell() {
  const state = useSyncExternalStore(player.subscribe, player.snapshot);
  return (
    <>
      <header className="app-header">
        <Link to="/" className="brand">
          ZIGBA
        </Link>
        <nav>
          <Link to="/">Player</Link>
          <Link to="/hud">HUD</Link>
          <Link to="/lab">Lab</Link>
        </nav>
        <span className="local">
          <i /> Local session
        </span>
      </header>
      <BridgeProvider host={player.session?.bridges}>
        <Outlet />
      </BridgeProvider>
      <footer className="app-status" role="status">
        {state.message}
      </footer>
    </>
  );
}
function Preview({ pixels }: { pixels: ArrayLike<number> }) {
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    if (ref.current) draw(ref.current, pixels);
  }, [pixels]);
  return (
    <canvas
      className="preview"
      ref={ref}
      width="240"
      height="160"
      aria-label="Captured game frame"
    />
  );
}

function GameLibraryPanel({ onClose }: { onClose: () => void }) {
  const s = useSyncExternalStore(player.subscribe, player.snapshot);
  const [search, setSearch] = useState("");
  return (
    <aside className="library">
      <div className="row">
        <h2>Library</h2>
        <button className="close-library" onClick={onClose}>
          Close
        </button>
      </div>
      <input
        aria-label="Search games"
        placeholder="Search games"
        value={search}
        onChange={(event) => setSearch(event.target.value)}
      />
      {!s.games.length && (
        <p>
          Your games live here.
          <br />
          Open a .gba or single-game ZIP to add one.
        </p>
      )}
      {s.games
        .filter((game) =>
          game.title.toLowerCase().includes(search.toLowerCase()),
        )
        .map((game) => (
          <article className="game" key={game.hash}>
            <button
              className="game-title"
              disabled={s.changing}
              onClick={act(() => player.open(game, undefined, false))}
            >
              {game.title}
            </button>
            <small>
              {game.hash.slice(0, 12)} · {(game.romSize / 1048576).toFixed(1)}{" "}
              MB
            </small>
            <details>
              <summary>Manage</summary>
              <form
                onSubmit={(event) => {
                  event.preventDefault();
                  const title = new FormData(event.currentTarget).get(
                    "title",
                  ) as string;
                  void player.library
                    .rename(game.hash, title)
                    .then(() => player.refresh())
                    .catch(player.error);
                }}
              >
                <input
                  name="title"
                  aria-label={`Rename ${game.title}`}
                  defaultValue={game.title}
                />
                <button>Rename</button>
              </form>
              <button
                onClick={act(async () => {
                  await player.library.remove(game.hash);
                  await player.refresh();
                })}
              >
                Remove game
              </button>
              <p>Removing a game keeps its save.</p>
              <button
                onClick={act(() =>
                  player.deleteProgress({
                    hash: game.hash,
                    playthroughId: game.activePlaythroughId ?? "default",
                  }),
                )}
              >
                Delete progress
              </button>
            </details>
          </article>
        ))}
    </aside>
  );
}

function PlayerView({ hud = false }: { hud?: boolean }) {
  const s = useSyncExternalStore(player.subscribe, player.snapshot);
  const [library, setLibrary] = useState(() => window.innerWidth > 760);
  const [note, setNote] = useState("");
  const [checkpoint, setCheckpoint] = useState("");
  const [captureOpen, setCaptureOpen] = useState(false);
  const [exported, setExported] = useState<ScenarioExport>();
  const [dragging, setDragging] = useState(false);
  const dragDepth = useRef(0);
  const enhanced = hud || hasHud(s.game?.hash);
  useEffect(() => {
    player.labActive = false;
    // Player and HUD share playback. LabView owns the pause on entering Lab.
    return () => player.releaseKeys();
  }, []);
  useEffect(() => {
    if (enhanced) setLibrary(false);
  }, [enhanced]);
  return (
    <main
      className={dragging ? "player drop-active" : "player"}
      onDragEnter={(e) => {
        if (!e.dataTransfer.types.includes("Files")) return;
        e.preventDefault();
        if (++dragDepth.current === 1) setDragging(true);
      }}
      onDragOver={(e) => {
        if (!e.dataTransfer.types.includes("Files")) return;
        e.preventDefault();
        e.dataTransfer.dropEffect = s.ready && !s.changing ? "copy" : "none";
      }}
      onDragLeave={(e) => {
        e.preventDefault();
        if (--dragDepth.current <= 0) {
          dragDepth.current = 0;
          setDragging(false);
        }
      }}
      onDrop={(e) => {
        e.preventDefault();
        dragDepth.current = 0;
        setDragging(false);
        const files = [...e.dataTransfer.files];
        if (files.length !== 1 || !/\.(gba|zip)$/i.test(files[0].name)) {
          player.error("Drop one .gba or single-game .zip file.");
          return;
        }
        if (!s.ready || s.changing) {
          player.error(
            "Wait for the player to finish loading, then drop your game again.",
          );
          return;
        }
        void player.open(files[0], undefined, false).catch(player.error);
      }}
    >
      {dragging && (
        <div className="drop-hint">Drop a .gba or .zip to open a game</div>
      )}
      <div className="page-title">
        <div>
          <p className="eyebrow">GAME BOY ADVANCE</p>
          <h1>{s.game?.title ?? "Universal player"}</h1>
          {enhanced && (
            <p className="hud-subtitle">
              <span className="enhanced-badge">Enhanced layout</span>
              Live data for verified game profiles.
            </p>
          )}
        </div>
        <div className="row">
          <button aria-expanded={library} onClick={() => setLibrary(!library)}>
            Library
          </button>
          <FileInput
            label="Open game"
            accept=".gba,.zip"
            onFile={(f) => player.open(f, undefined, false)}
          />
        </div>
      </div>
      <GameExperience
        romHash={s.game?.hash}
        playing={s.playing}
        libraryOpen={library}
        forceEnhanced={hud}
      >
        {library && <GameLibraryPanel onClose={() => setLibrary(false)} />}
        <GamePlayer onCapture={() => setCaptureOpen(true)} />
      </GameExperience>
      <section className="console player-tools">
        <div className="recording">
          <label>
            <input
              type="checkbox"
              checked={s.recording?.enabled ?? true}
              disabled={!s.game}
              onChange={(e) =>
                void player.emulator
                  .recording(e.target.checked)
                  .then((recording) => player.update({ recording }))
                  .catch(player.error)
              }
            />{" "}
            Record gameplay
          </label>
          {s.recording && (
            <small>
              {(Number(s.recording.historyCycles) / 16777216).toFixed(1)} s
              retained · {s.recording.snapshots} snapshots ·{" "}
              {s.recording.overheadMs.toFixed(0)} ms recording
            </small>
          )}
        </div>
        {(s.recording?.full ||
          s.recording?.gap ||
          s.recording?.snapshotError != null) && (
          <p role="alert">
            {s.recording.full
              ? "1,024 inputs reached. Scenario journal stopped; failure capture continues."
              : "History or snapshot unavailable."}{" "}
            <button
              onClick={act(async () => {
                await player.pause();
                player.update({
                  recording: await player.emulator.newSegment(),
                });
              })}
            >
              New recording segment
            </button>
          </p>
        )}
        <details>
          <summary>Checkpoints & exports</summary>
          <div className="row">
            <input
              aria-label="Checkpoint name"
              placeholder="Checkpoint name"
              value={checkpoint}
              onChange={(e) => setCheckpoint(e.target.value)}
            />
            <button
              disabled={!s.game}
              onClick={act(async () => {
                await player.pause();
                player.update({
                  recording: await player.emulator.checkpoint(checkpoint),
                });
                setCheckpoint("");
              })}
            >
              Mark checkpoint
            </button>
            <button
              disabled={!s.game}
              onClick={act(async () => {
                await player.pause();
                const item = await player.emulator.exportScenario(
                  s.game?.title,
                );
                setExported(item);
                try {
                  await player.library.remember(item);
                } catch (e) {
                  player.error(e);
                }
              })}
            >
              Prepare scenario
            </button>
            <button
              disabled={!s.game || s.changing}
              onClick={act(async () => {
                await player.pause();
                player.update({
                  recording: await player.emulator.newSegment(),
                });
              })}
            >
              Start new segment
            </button>
          </div>
          {s.recording?.checkpoints.map((p) => (
            <p key={p.name}>
              {p.name} · cycle {p.at}
            </p>
          ))}
          {exported && (
            <div>
              <button
                onClick={() =>
                  download(json(exported.manifest), "scenario.json")
                }
              >
                Download manifest
              </button>
              {Object.entries(exported.assets).map(([name, bytes]) => (
                <button
                  key={name}
                  onClick={() =>
                    download(bytes, exported.manifest.assets[name].path!)
                  }
                >
                  Download {name}
                </button>
              ))}
              <p>
                Keep these files together for the CLI. This scenario is also
                available in Lab.
              </p>
            </div>
          )}
          <div className="row">
            <button
              disabled={!s.game}
              onClick={act(async () =>
                download(
                  await player.emulator.exportSave(),
                  `${s.game!.title}.sav`,
                ),
              )}
            >
              Export save
            </button>
            <FileInput
              label="Import save"
              accept=".sav"
              onFile={async (f) => {
                if (!s.game) throw new Error("Open a game first");
                await player.open(
                  s.game,
                  new Uint8Array(await f.arrayBuffer()),
                  false,
                );
                await player.persist();
              }}
            />
            <button
              disabled={!s.game}
              onClick={act(async () => {
                await player.pause();
                download(await player.emulator.saveState(), "game.state");
              })}
            >
              Export state
            </button>
            <FileInput
              label="Import state"
              onFile={async (f) => {
                await player.pause();
                await player.emulator.loadState(
                  new Uint8Array(await f.arrayBuffer()),
                );
              }}
            />
          </div>
        </details>
        <details>
          <summary>Boot options</summary>
          <p>Applied when a game opens.</p>
          <label>
            Save type{" "}
            <select
              defaultValue="auto"
              onChange={(e) => {
                player.options.saveType = e.target
                  .value as (typeof SAVE_TYPES)[number];
              }}
            >
              {SAVE_TYPES.map((t) => (
                <option key={t}>{t}</option>
              ))}
            </select>
          </label>
          <label>
            Cartridge clock{" "}
            <select
              aria-label="Cartridge clock"
              defaultValue={
                player.options.rtc === undefined
                  ? "auto"
                  : player.options.rtc
                    ? "on"
                    : "off"
              }
              onChange={(e) => {
                player.options.rtc =
                  e.target.value === "auto"
                    ? undefined
                    : e.target.value === "on";
              }}
            >
              <option value="auto">Auto-detect</option>
              <option value="on">Enabled</option>
              <option value="off">Disabled</option>
            </select>
          </label>
          <label>
            Fixed clock epoch (UTC seconds){" "}
            <input
              type="number"
              min="0"
              max="253402300799"
              step="1"
              placeholder="Current time at boot"
              defaultValue={player.options.rtcEpoch?.toString() ?? ""}
              onChange={(e) => {
                if (!e.target.validity.valid) return;
                player.options.rtcEpoch = e.target.value
                  ? BigInt(e.target.valueAsNumber)
                  : undefined;
              }}
            />
          </label>
          <p>
            Auto-detect enables the clock for supported cartridges. Leave the
            epoch blank to use current UTC time at boot; captures pin that time
            for replay.
          </p>
          <FileInput
            label="Load BIOS"
            onFile={async (f) => {
              const bytes = new Uint8Array(await f.arrayBuffer());
              if (bytes.length !== 16384)
                throw new Error("BIOS must be 16 KiB");
              player.options.bios = bytes;
              player.update({ message: "BIOS selected for next boot." });
            }}
          />
          <button
            onClick={() => {
              delete player.options.bios;
              player.update({ message: "HLE selected for next boot." });
            }}
          >
            Use HLE
          </button>
        </details>
        <details>
          <summary>Diagnostics & controls</summary>
          <p>
            Standard gamepads are also supported. CPU details refresh once per
            second.
          </p>
          <pre>{s.cpu ? json(s.cpu) : "No game loaded"}</pre>
        </details>
      </section>
      {(captureOpen || s.capture) && (
        <section className="capture-panel" aria-label="Bug capture">
          <h2>Capture bug</h2>
          {!s.capture ? (
            <>
              <label>
                What happened?
                <textarea
                  value={note}
                  onChange={(e) => setNote(e.target.value)}
                  maxLength={1000}
                />
              </label>
              <button onClick={act(() => player.capture(note))}>
                Retain capture
              </button>
            </>
          ) : (
            <>
              <p>
                {s.capture.history.replayable
                  ? "Snapshot replay available."
                  : "No replayable snapshot retained."}
              </p>
              {s.capture.artifacts.framebuffer && (
                <Preview pixels={s.capture.artifacts.framebuffer} />
              )}
              <p>{s.capture.history.audioReplay}</p>
              <button onClick={() => download(json(s.capture), "failure.json")}>
                Download failure bundle
              </button>
              {s.capture.artifacts.framebuffer && (
                <button
                  onClick={() => screenshot(s.capture!.artifacts.framebuffer!)}
                >
                  Download screenshot
                </button>
              )}
            </>
          )}
          <button
            onClick={() => {
              setCaptureOpen(false);
              player.update({ capture: undefined });
            }}
          >
            Close
          </button>
        </section>
      )}
    </main>
  );
}
function LabView() {
  const [lab, setLab] = useState<Lab>();
  useEffect(() => {
    void player.enterLab().catch(player.error);
    void getLab().then(setLab).catch(player.error);
    return () => {
      player.labActive = false;
    };
  }, []);
  return lab ? <LabPanel lab={lab} /> : <main>Starting Lab…</main>;
}
function LabPanel({ lab }: { lab: Lab }) {
  const state = useSyncExternalStore(lab.subscribe, lab.snapshot);
  const [manifest, setManifest] = useState<Scenario>();
  const [bundle, setBundle] = useState<FailureBundle>();
  const [assets, setAssets] = useState<Record<string, Uint8Array>>({});
  const [saved, setSaved] = useState<Scenario[]>([]);
  const [reports, setReports] = useState<unknown[]>([]);
  useEffect(() => {
    void player.library.scenarios().then(setSaved);
    void player.library.reports().then(setReports);
  }, []);
  async function load(value: unknown) {
    if (lab.state.status === "Running")
      throw new Error("Cancel the current test before changing its manifest.");
    if (!value || typeof value !== "object")
      throw new Error("Expected a manifest or failure report.");
    const retained = value as {
      status?: string;
      checkpoints?: Record<string, unknown>[];
      retainedArtifacts?: { name: string; bytes: Uint8Array }[];
    };
    const v = value as Scenario &
      FailureBundle & { manifest?: Scenario; capture?: FailureBundle };
    const failure = v.format === "zigba-failure" ? v : v.capture;
    if (failure) validateFailureBundle(failure);
    const m =
      v.manifest ??
      failure?.identity.manifest ??
      ((failure
        ? {
            version: 1,
            name: "Failure replay",
            assets: {
              rom: { sha256: failure.identity.romHash },
              ...(failure.identity.biosSha256
                ? { bios: { sha256: failure.identity.biosSha256 } }
                : {}),
            },
            config: failure.identity.config ?? {
              boot: "hle",
              saveType: "auto",
              rtc: false,
              rtcEpoch: "0",
              sampleRate: 48000,
            },
            inputs: [],
            checkpoints: [{ name: "Replay", at: "0" }],
            limits: {
              maxCycles: "0",
              chunkCycles: "70224",
              maxRuns: 1000000,
              maxAudioBytes: 268435456,
            },
          }
        : v) as Scenario);
    validateScenario(m);
    if (retained.status)
      lab.update({
        status: retained.status,
        report: value as import("@evenfall/gba").LabReport,
        checkpoints: retained.checkpoints ?? [],
        artifacts: retained.retainedArtifacts ?? [],
      });
    else
      lab.update({
        status: "Ready",
        report: undefined,
        checkpoints: [],
        artifacts: [],
      });
    setManifest(m);
    setBundle(failure);
    const found: Record<string, Uint8Array> = {};
    for (const [name, descriptor] of Object.entries(m.assets)) {
      const bytes = await player.library.asset(descriptor.sha256);
      if (bytes) found[name] = bytes;
    }
    setAssets(found);
  }
  async function run(baseline = false) {
    if (!manifest) return;
    const m = structuredClone(manifest);
    if (baseline) {
      for (const p of m.checkpoints) {
        delete p.expect;
        p.memory?.forEach((r) => delete r.sha256);
      }
    }
    const report = await lab.run(m, assets, baseline ? undefined : bundle);
    try {
      await player.library.retain("reports", {
        ...report,
        retainedArtifacts: lab.state.artifacts,
      });
      setReports(await player.library.reports());
    } catch (error) {
      player.error(error);
    }

    if (baseline && report.status === "passed") {
      m.name = `${m.name.replace(/ — regression baseline$/, "")} — regression baseline`;
      for (let i = 0; i < m.checkpoints.length; i++) {
        const actual = report.checkpoints![i];
        const {
          cycle,
          reason,
          cpu,
          frameSha256,
          audioSha256,
          saveSha256,
          stateSha256,
        } = actual;
        m.checkpoints[i].expect = {
          cycle,
          reason,
          cpu: {
            ...(cpu as object),
            registers: Object.fromEntries(
              (cpu as { registers: number[] }).registers.map((n, i) => [i, n]),
            ),
          },
          frameSha256,
          audioSha256,
          saveSha256,
          ...(stateSha256 ? { stateSha256 } : {}),
        };
        m.checkpoints[i].memory =
          actual.memory as Scenario["checkpoints"][number]["memory"];
      }
      validateScenario(m);
      setManifest(m);
      setBundle(undefined);
      try {
        await player.library.retain("scenarios", m);
        setSaved(await player.library.scenarios());
      } catch (error) {
        player.error(error);
      }
    }
  }
  return (
    <main className="lab">
      <div className="page-title">
        <div>
          <p className="eyebrow">TEST & REPLAY</p>
          <h1>Lab</h1>
        </div>
        <FileInput
          label="Import manifest or failure"
          accept=".json"
          onFile={async (f) => load(JSON.parse(await f.text()))}
        />
      </div>
      <p>Gameplay is paused and preserved. Tests run in an isolated worker.</p>
      <div className="lab-layout">
        <aside>
          <h2>Scenarios</h2>
          {saved.map((m, i) => (
            <button key={i} onClick={act(() => load(m))}>
              {m.name}
            </button>
          ))}
          <h2>Retained reports</h2>
          {reports.map((r, i) => (
            <button key={i} onClick={act(() => load(r))}>
              Report {i + 1}
            </button>
          ))}
        </aside>
        <section>
          <h2>{manifest?.name ?? "A repeatable test starts here."}</h2>
          {!manifest && (
            <p>
              Import a CLI manifest, a failure bundle, or select a recorded
              scenario.
            </p>
          )}
          {manifest && (
            <>
              {bundle && (
                <>
                  <p>
                    {bundle.history.replayable
                      ? "Snapshot replay available."
                      : "No replayable snapshot retained."}
                  </p>
                  {bundle.artifacts.framebuffer && (
                    <Preview pixels={bundle.artifacts.framebuffer} />
                  )}
                  <details>
                    <summary>Failure evidence</summary>
                    <pre>
                      {json({
                        ...bundle,
                        snapshots: undefined,
                        operations: undefined,
                        artifacts: { cpu: bundle.artifacts.cpu },
                      })}
                    </pre>
                  </details>
                </>
              )}
              <h3>Required assets</h3>
              {Object.entries(manifest.assets).map(([name, d]) => (
                <div className="asset" key={name}>
                  <strong>{name}</strong>
                  <code>{d.sha256.slice(0, 16)}</code>
                  <span>{assets[name] ? "Matched" : "Missing"}</span>
                  <FileInput
                    label={`Choose ${name}`}
                    onFile={async (f) => {
                      const bytes = new Uint8Array(await f.arrayBuffer());
                      if ((await sha256(bytes)) !== d.sha256)
                        throw new Error(`${name} hash mismatch`);
                      setAssets((a) => ({ ...a, [name]: bytes }));
                      await player.library.addAsset(d.sha256, bytes);
                    }}
                  />
                </div>
              ))}
              <div className="row">
                <button
                  className="primary"
                  disabled={
                    state.status === "Running" ||
                    Object.keys(manifest.assets).some((k) => !assets[k])
                  }
                  onClick={act(() => run())}
                >
                  {bundle ? "Replay failure" : "Run scenario"}
                </button>
                <button
                  disabled={state.status !== "Running"}
                  onClick={act(async () => {
                    await lab.cancel();
                  })}
                >
                  Cancel
                </button>
                <button
                  disabled={state.status !== "Running"}
                  onClick={() => lab.terminate()}
                >
                  Terminate worker
                </button>
                <button
                  disabled={
                    !!bundle ||
                    state.status === "Running" ||
                    Object.keys(manifest.assets).some((k) => !assets[k])
                  }
                  onClick={act(() => run(true))}
                >
                  Record baseline
                </button>
              </div>
              <p>
                Record baseline generates regression expectations from an
                isolated replay.
              </p>
              <button onClick={() => download(json(manifest), "scenario.json")}>
                Download manifest
              </button>
              <details>
                <summary>Edit assertions</summary>
                <textarea
                  disabled={state.status === "Running"}
                  className="manifest-editor"
                  aria-label="Scenario JSON"
                  key={json(manifest)}
                  defaultValue={json(manifest)}
                  onBlur={(e) => {
                    try {
                      const m = JSON.parse(e.target.value);
                      validateScenario(m);
                      setManifest(m);
                    } catch (error) {
                      player.error(error);
                    }
                  }}
                />
              </details>
            </>
          )}
          <h3 role="status">{state.status}</h3>
          <p className="limitation">
            Snapshot restore resets the host resampler. Exact continuous audio
            comparisons require cold-boot replay.
          </p>
          {state.report && (
            <>
              <button
                onClick={() => download(json(state.report), "result.json")}
              >
                Download result JSON
              </button>
              {state.report.capture && (
                <button
                  onClick={() =>
                    download(json(state.report!.capture), "failure.json")
                  }
                >
                  Download failure bundle
                </button>
              )}
              <pre>{json(state.report.failures ?? [])}</pre>
              {state.report.inspection !== undefined && (
                <details open>
                  <summary>Replayed CPU</summary>
                  <pre>{json(state.report.inspection)}</pre>
                </details>
              )}
            </>
          )}
          {state.checkpoints.map((p, i) => (
            <details key={i} open>
              <summary>
                {String(p.name)} · requested {String(p.requestedCycle)} / actual{" "}
                {String(p.cycle)}
              </summary>
              <div className="comparison">
                <div>
                  <h4>Expected</h4>
                  <pre>
                    {json({
                      ...manifest?.checkpoints[i]?.expect,
                      memory: manifest?.checkpoints[i]?.memory,
                    })}
                  </pre>
                </div>
                <div>
                  <h4>Actual · CPU & hashes</h4>
                  <pre>{json(p)}</pre>
                </div>
              </div>
            </details>
          ))}
          {state.artifacts.map((a, i) => (
            <div key={i}>
              {a.name.endsWith(".bgr555le") && (
                <Preview pixels={new Uint16Array(a.bytes.slice().buffer)} />
              )}
              <button onClick={() => download(a.bytes, a.name)}>
                Download {a.name}
              </button>
            </div>
          ))}
        </section>
      </div>
    </main>
  );
}
const rootRoute = createRootRoute({ component: Shell });
const playerRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: PlayerView,
});
const hudRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/hud",
  component: () => <PlayerView hud />,
});
const labRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/lab",
  component: LabView,
});
const router = createRouter({
  routeTree: rootRoute.addChildren([playerRoute, hudRoute, labRoute]),
  history: createHashHistory(),
});
declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
// Game plugins' HUD layouts and styles load before the first render.
void gameRegistry
  .load("huds")
  .catch((error: unknown) => console.error(error))
  .finally(() =>
    createRoot(document.getElementById("root")!).render(
      <RouterProvider router={router} />,
    ),
  );
