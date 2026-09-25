import {
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react";
import { createRoot } from "react-dom/client";
import { Button } from "@base-ui/react/button";
import { Dialog } from "@base-ui/react/dialog";
import { Popover } from "@base-ui/react/popover";
import { Select } from "@base-ui/react/select";
import { player } from "./player";
import { BridgeProvider } from "@evenfall/gba/bridge/react";
import { ConsolePlayer, PlayerHelp, quickState } from "./console-player";
import {
  DeletePlaythroughDialog,
  PlaythroughDialog,
} from "./playthrough-dialog";
import { AboutPage } from "./about";
import { LibraryPage } from "./library-page";
import { AppHeader, type AppView } from "./app-header";
import { Account } from "./account";
import type { CloudResume } from "./cloud";
import { QUICK_SLOTS, type Game, type PlaythroughRef } from "./library";
import { QuickConflictChoices } from "./quick-conflicts";
import { gameRegistry } from "./games";
import { download } from "./media";
import { trackView } from "./analytics";
import "./console.css";

const act = (action: () => Promise<unknown>) =>
  void action().catch(player.error);
const icons = {
  cloudUpload: (
    <>
      <path d="M12 13v8m-4-4 4-4 4 4" />
      <path d="M20.88 18.09A5 5 0 0 0 18 9h-1.26A8 8 0 1 0 4 16.25" />
    </>
  ),
  fullscreen: <path d="M3 9V3h6m6 0h6v6m0 6v6h-6m-6 0H3v-6" />,
  sound: (
    <>
      <path d="M3 9h4l5-5v16l-5-5H3z" />
      <path d="M16 8c3 2 3 6 0 8m3-11c5 4 5 10 0 14" />
    </>
  ),
  muted: (
    <>
      <path d="M3 9h4l5-5v16l-5-5H3z" />
      <path d="m16 9 6 6m0-6-6 6" />
    </>
  ),
  gamepad: (
    <>
      <path d="M6 6h12c3 0 5 12 3 14-2 1-5-4-6-4H9c-1 0-4 5-6 4C1 18 3 6 6 6Z" />
      <path d="M7 9v6m-3-3h6m6-2h.01m3 3h.01" />
    </>
  ),
  more: (
    <>
      <path d="m4 9 3 3-3 3-3-3Zm8 0 3 3-3 3-3-3Zm8 0 3 3-3 3-3-3Z" />
    </>
  ),
};
function Icon({ name }: { name: keyof typeof icons }) {
  return (
    <svg
      viewBox="0 0 24 24"
      width="24"
      height="24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.8"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {icons[name]}
    </svg>
  );
}

function Modal({
  title,
  children,
  close,
  className = "",
  busy = false,
}: {
  title: string;
  children: ReactNode;
  close: () => void;
  className?: string;
  busy?: boolean;
}) {
  return (
    <Dialog.Root
      open
      onOpenChange={(open, details) => {
        if (busy) details.cancel();
        else if (!open) close();
      }}
    >
      <Dialog.Portal
        container={
          (document.fullscreenElement as HTMLElement | null) ?? undefined
        }
      >
        <Dialog.Backdrop className="console-dialog-backdrop" />
        <Dialog.Popup
          className={`console-dialog ${className}`}
          aria-busy={busy}
        >
          <header>
            <Dialog.Title>{title}</Dialog.Title>
            <Dialog.Close aria-label="Close dialog" disabled={busy}>
              ×
            </Dialog.Close>
          </header>
          {children}
        </Dialog.Popup>
      </Dialog.Portal>
    </Dialog.Root>
  );
}

function App() {
  const state = useSyncExternalStore(player.subscribe, player.snapshot);
  const [menu, setMenu] = useState<"speed" | "more" | null>(null);
  const [view, setView] = useState<AppView>("home");
  const home = view === "home";
  const library = view === "library";
  const navigationResume = useRef(false);
  const navigationWork = useRef(Promise.resolve());
  const navigation = useRef(view);
  const [help, setHelp] = useState(false);
  const helpResume = useRef(false);
  const [accountOpen, setAccountOpen] = useState(() =>
    new URLSearchParams(location.search).has("account"),
  );
  const accountResume = useRef(false);
  const [controls, setControls] = useState(false);
  const [muted, setMuted] = useState(player.audio.muted);
  const [volume, setVolume] = useState(player.audio.volume);
  const [notice, setNotice] = useState("");
  const [dragging, setDragging] = useState(false);
  const [launching, setLaunching] = useState(false);
  const [quickReview, setQuickReview] = useState(false);
  const [quickBusy, setQuickBusy] = useState(false);
  const quickResume = useRef(false);
  const launchPending = useRef(false);
  const [cloudResumeRequest, setCloudResumeRequest] =
    useState<CloudResume | null>(null);
  const [cloudResumeHasLocal, setCloudResumeHasLocal] = useState(false);
  const [cloudResumeError, setCloudResumeError] = useState("");
  const [resumeFailure, setResumeFailure] = useState<{
    game: Game;
    message: string;
  } | null>(null);
  const romInput = useRef<HTMLInputElement>(null);
  const cloudRomInput = useRef<HTMLInputElement>(null);
  const playthroughTrigger = useRef<HTMLButtonElement>(null);
  const [createPlaythroughOpen, setCreatePlaythroughOpen] = useState(false);
  // Keep the panels in the toolbar's existing desktop/mobile CSS coordinates.
  const toolbar = useRef<HTMLDivElement>(null);
  const dragDepth = useRef(0);
  const experience = gameRegistry.layoutForRom("dashboards", state.game?.hash);
  const Layout = experience?.Layout;
  const lastGame = state.game ?? state.games[0];
  const lastRun = state.runs.find(
    (run) =>
      run.hash === lastGame?.hash &&
      run.id === (lastGame?.activePlaythroughId ?? "default"),
  );
  const cloudFallback = lastGame ? undefined : state.cloud.latest;
  const cloudLocalGame = state.games.find(
    (game) => game.hash === cloudResumeRequest?.hash,
  );
  const cloudLocalRun = state.runs.find(
    (run) =>
      run.hash === cloudResumeRequest?.hash &&
      run.id === cloudResumeRequest?.playthroughId,
  );
  const playerRuns = [
    ...state.playthroughs.map((run) => ({ id: run.id, name: run.name })),
    ...(state.cloud.resumes ?? [])
      .filter(
        (run) =>
          run.hash === state.game?.hash &&
          !state.playthroughs.some((local) => local.id === run.playthroughId),
      )
      .map((run) => ({
        id: run.playthroughId,
        name: `${run.playthroughName} · cloud`,
      })),
  ];
  const canResume = Boolean(lastGame || cloudFallback);
  const quickConflicts =
    state.cloud.quickConflicts?.filter(
      (conflict) =>
        conflict.hash === state.game?.hash &&
        conflict.playthroughId === state.playthrough?.id,
    ) ?? [];
  const unpromptedQuick = quickConflicts.filter(
    (conflict) => !conflict.prompted,
  );
  // True divergence is asked about once. Later reminders stay in the menu.
  useEffect(() => {
    if (
      !unpromptedQuick.length ||
      quickReview ||
      help ||
      accountOpen ||
      state.changing
    )
      return;
    openQuickReview();
    void player.markQuickPrompted(unpromptedQuick).catch(player.error);
  }, [
    unpromptedQuick.map((conflict) => conflict.etag).join(),
    help,
    accountOpen,
    state.changing,
  ]);
  useEffect(() => {
    if (quickReview && !quickConflicts.length && !quickBusy) closeQuickReview();
  }, [quickReview, quickConflicts.length, quickBusy]);
  useEffect(() => trackView(view), [view]);
  useEffect(() => {
    if (home && state.ready && state.cloud.accountId)
      void player.cloud.discover();
  }, [home, state.ready, state.cloud.accountId]);
  useEffect(() => {
    if (cloudResumeRequest?.accountId !== state.cloud.accountId)
      setCloudResumeRequest(null);
  }, [state.cloud.accountId, cloudResumeRequest?.accountId]);
  useEffect(() => {
    const benign = [
      "Starting…",
      "Open a game to begin.",
      "Ready. Press Play.",
      "Playing",
    ];
    if (benign.includes(state.message)) return;
    setNotice(state.message);
  }, [state.message]);
  useEffect(() => {
    if (!notice) return;
    const timer = setTimeout(() => setNotice(""), 2000);
    return () => clearTimeout(timer);
  }, [notice]);
  useEffect(() => {
    const escape = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        if (!document.querySelector('dialog[open],[aria-modal="true"]'))
          setControls(false);
        player.releaseKeys();
      }
    };
    window.addEventListener("keydown", escape);
    return () => {
      window.removeEventListener("keydown", escape);
    };
  }, []);
  function openQuickReview() {
    quickResume.current = player.state.playing;
    setMenu(null);
    setQuickReview(true);
    act(() => player.pause());
  }
  function closeQuickReview() {
    setQuickReview(false);
    if (quickResume.current && !player.state.changing) act(() => player.play());
  }
  function openHelp() {
    helpResume.current = player.state.playing;
    setMenu(null);
    setHelp(true);
    act(() => player.pause());
  }
  function closeHelp() {
    setHelp(false);
    if (helpResume.current && !player.state.changing) act(() => player.play());
  }
  function openAccount() {
    accountResume.current = player.state.playing;
    setMenu(null);
    act(async () => {
      // Persist progress before Discord can navigate away from the player.
      await player.pause();
      setAccountOpen(true);
    });
  }
  function closeAccount() {
    setAccountOpen(false);
    if (accountResume.current && !player.state.changing)
      act(() => player.play());
  }
  function showView(destination: typeof view) {
    navigation.current = destination;
    setView(destination);
  }
  function browse(destination: "home" | "library") {
    if (player.state.changing) return;
    if (navigation.current === "player") {
      navigationResume.current ||= player.state.playing;
      player.releaseKeys();
      // A resume may still be awaiting its worker acknowledgement. Pause after
      // it settles, so a late play result cannot overwrite the paused state.
      navigationWork.current = navigationWork.current
        .then(() => player.pause())
        .catch(player.error);
    }
    showView(destination);
    setMenu(null);
  }
  function openPlayer() {
    if (player.state.resumeFailed && player.state.game) {
      setResumeFailure({
        game: player.state.game,
        message:
          "The local checkpoint could not be restored. Your stored progress is preserved.",
      });
      return;
    }
    const shouldResume = navigationResume.current;
    showView("player");
    navigationWork.current = navigationWork.current
      .then(async () => {
        if (
          shouldResume &&
          navigation.current === "player" &&
          !player.state.changing &&
          !player.state.playing
        )
          await player.play();
        if (navigation.current === "player") navigationResume.current = false;
      })
      .catch(player.error);
  }
  async function launchGameInternal(
    game: Game,
    fromBattery = false,
    playthroughId = game.activePlaythroughId ?? "default",
  ) {
    setResumeFailure(null);
    try {
      await navigationWork.current;
      const ref = { hash: game.hash, playthroughId };
      const run = await player.library.getPlaythrough(ref);
      const selected = {
        ...game,
        activePlaythroughId: playthroughId,
        resumeSavedAt: run?.resumeSavedAt,
      };
      if (fromBattery) await player.open(selected, undefined, true, ref);
      else if (
        game.hash === player.state.game?.hash &&
        playthroughId === player.activeRef?.playthroughId &&
        !player.state.resumeFailed
      ) {
        if (!player.state.playing) await player.play();
      } else if (
        game.hash === player.state.game?.hash &&
        player.activeRef &&
        !player.state.resumeFailed
      )
        await player.switchPlaythrough(playthroughId);
      else if (run?.resumeSavedAt) await player.resume(selected);
      else await player.open(selected, undefined, true, ref);
      navigationResume.current = false;
      showView("player");
    } catch (error) {
      if (
        !fromBattery &&
        (
          await player.library.getPlaythrough({
            hash: game.hash,
            playthroughId,
          })
        )?.resumeSavedAt
      )
        setResumeFailure({
          game: { ...game, activePlaythroughId: playthroughId },
          message: error instanceof Error ? error.message : String(error),
        });
      throw error;
    }
  }
  async function launchGame(game: Game, fromBattery = false) {
    if (launchPending.current || player.state.changing) return;
    launchPending.current = true;
    setLaunching(true);
    try {
      await launchGameInternal(game, fromBattery);
    } finally {
      launchPending.current = false;
      setLaunching(false);
    }
  }
  async function loadRun(
    ref: PlaythroughRef,
    source: "home" | "selection" | "restore" = "selection",
  ) {
    if (launchPending.current || player.state.changing) return;
    launchPending.current = true;
    setLaunching(true);
    try {
      const game = player.state.games.find((item) => item.hash === ref.hash);
      const run = await player.library.getPlaythrough(ref);
      const local = Boolean(
        run?.resumeSavedAt ||
          (await player.library.getSave(ref))?.length ||
          (player.activeRef?.hash === ref.hash &&
            player.activeRef.playthroughId === ref.playthroughId),
      );
      const listed =
        player.state.cloud.resumes?.find(
          (item) =>
            item.hash === ref.hash && item.playthroughId === ref.playthroughId,
        ) ??
        (player.state.cloud.latest?.hash === ref.hash &&
        player.state.cloud.latest.playthroughId === ref.playthroughId
          ? player.state.cloud.latest
          : undefined);
      let remote: Awaited<ReturnType<typeof player.cloud.inspectResume>>;
      try {
        remote = await player.cloud.inspectResume(
          ref,
          game?.title ?? listed?.title ?? "Game",
          run?.name ?? listed?.playthroughName ?? "Playthrough",
        );
      } catch {
        if (source === "restore") {
          setNotice("Cloud check unavailable. Try again later.");
          return;
        }
        if (game)
          setNotice("Cloud check unavailable. Continuing on this device.");
      }
      const job = (
        await player.library.cloudJobs(player.state.cloud.accountId ?? "")
      ).find(
        (item) =>
          item.hash === ref.hash &&
          item.playthroughId === ref.playthroughId &&
          item.kind === "resume",
      );
      const backup = remote?.backup ?? (!game ? listed : undefined);
      if (
        backup &&
        (source === "restore" ||
          !local ||
          !job?.baseEtag ||
          job.baseEtag !== remote?.etag)
      ) {
        setCloudResumeError("");
        setCloudResumeHasLocal(local);
        setCloudResumeRequest(backup);
      } else if (source === "restore") {
        setNotice("No cloud checkpoint is available for this playthrough.");
      } else if (game && !run) {
        setNotice(
          "This playthrough is not on this device and its cloud checkpoint is unavailable.",
        );
      } else if (game) {
        if (
          source === "home" &&
          player.activeRef?.hash === ref.hash &&
          player.activeRef.playthroughId === ref.playthroughId &&
          !player.state.resumeFailed
        )
          openPlayer();
        else await launchGameInternal(game, false, ref.playthroughId);
      } else if (listed) {
        setCloudResumeHasLocal(false);
        setCloudResumeRequest(listed);
      }
    } finally {
      launchPending.current = false;
      setLaunching(false);
    }
  }
  function resumeFromHome() {
    if (lastGame)
      act(() =>
        loadRun(
          {
            hash: lastGame.hash,
            playthroughId: lastGame.activePlaythroughId ?? "default",
          },
          "home",
        ),
      );
    else if (cloudFallback)
      act(() =>
        loadRun({
          hash: cloudFallback.hash,
          playthroughId: cloudFallback.playthroughId,
        }),
      );
    else openPlayer();
  }
  async function launchCloud(backup: CloudResume, file?: File) {
    if (launchPending.current || player.state.changing) return;
    launchPending.current = true;
    setLaunching(true);
    setCloudResumeError("");
    try {
      await navigationWork.current;
      await player.restoreCloud("resume", backup, file);
      navigationResume.current = false;
      setCloudResumeRequest(null);
      showView("player");
    } catch (error) {
      setCloudResumeError(
        error instanceof Error ? error.message : String(error),
      );
    } finally {
      launchPending.current = false;
      setLaunching(false);
    }
  }
  async function openFile(file: File) {
    if (cloudResumeRequest) return launchCloud(cloudResumeRequest, file);
    await navigationWork.current;
    showView("player");
    navigationResume.current = false;
    await (/\.state$/i.test(file.name) ? importState(file) : player.open(file));
  }
  async function importState(file: File) {
    if (!state.game || player.state.changing) return;
    player.update({ changing: true });
    try {
      await player.pause(true);
      player.latest = await player.emulator.loadState(
        new Uint8Array(await file.arrayBuffer()),
      );
      player.update({ message: "State loaded" });
      await player.checkpointState();
    } finally {
      player.update({ changing: false });
    }
    setMenu(null);
  }
  const display = (
    <div className="player-stage">
      <ConsolePlayer onHelp={openHelp} />
      {controls && (
        <div className="touch-pad" aria-label="Touch controls">
          {[
            ["↑", 64],
            ["←", 32],
            ["↓", 128],
            ["→", 16],
            ["B", 2],
            ["A", 1],
            ["L", 512],
            ["R", 256],
            ["Select", 4],
            ["Start", 8],
          ].map(([label, value]) => (
            <button
              key={label}
              onPointerDown={(event) => {
                event.preventDefault();
                event.currentTarget.setPointerCapture(event.pointerId);
                player.touch |= Number(value);
                player.keys();
              }}
              onPointerUp={() => {
                player.touch &= ~Number(value);
                player.keys();
              }}
              onPointerCancel={() => {
                player.touch &= ~Number(value);
                player.keys();
              }}
              onLostPointerCapture={() => {
                player.touch &= ~Number(value);
                player.keys();
              }}
            >
              {label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
  return (
    <BridgeProvider host={state.ready ? player.session.bridges : undefined}>
      <div
        className={`console-app ${dragging ? "dragging" : ""}`}
        data-view={view}
        onDragEnter={(event) => {
          if (event.dataTransfer.types.includes("Files")) {
            event.preventDefault();
            if (++dragDepth.current === 1) setDragging(true);
          }
        }}
        onDragOver={(event) => {
          if (event.dataTransfer.types.includes("Files"))
            event.preventDefault();
        }}
        onDragLeave={() => {
          if (--dragDepth.current <= 0) {
            dragDepth.current = 0;
            setDragging(false);
          }
        }}
        onDrop={(event) => {
          event.preventDefault();
          dragDepth.current = 0;
          setDragging(false);
          const file = event.dataTransfer.files[0];
          if (file && state.ready && !state.changing) act(() => openFile(file));
        }}
      >
        {!home && (
          <AppHeader
            view={view}
            onNavigate={(destination) =>
              destination === "player" ? openPlayer() : browse(destination)
            }
            onAddGame={() => romInput.current?.click()}
            onAccount={openAccount}
            busy={!state.ready || state.changing || launching}
          />
        )}
        <input
          ref={romInput}
          className="file-input"
          type="file"
          aria-label="Open ROM"
          accept=".gba,.zip"
          disabled={!state.ready || state.changing || launching}
          onChange={(event) => {
            const file = event.target.files?.[0];
            event.target.value = "";
            if (file) act(() => openFile(file));
          }}
        />
        <div className="player-tools" hidden={home || library}>
          <h1 className="player-tools-title">
            {state.game?.title ?? "No game loaded"}
          </h1>
          {state.game && state.playthrough && (
            <div className="playthrough-controls">
              <Select.Root
                items={playerRuns.map((run) => ({
                  label: run.name,
                  value: run.id,
                }))}
                value={state.playthrough?.id ?? ""}
                disabled={state.changing || launching}
                onValueChange={(value) => {
                  if (value === "__new__") setCreatePlaythroughOpen(true);
                  else if (value && state.game)
                    act(() =>
                      loadRun({ hash: state.game!.hash, playthroughId: value }),
                    );
                }}
              >
                <Select.Label className="playthrough-label">
                  Playthrough
                </Select.Label>
                <Select.Trigger
                  ref={playthroughTrigger}
                  className="playthrough-select"
                >
                  <Select.Value />
                  <Select.Icon aria-hidden="true">▾</Select.Icon>
                </Select.Trigger>
                <Select.Portal>
                  <Select.Positioner
                    alignItemWithTrigger={false}
                    sideOffset={4}
                    className="playthrough-select-positioner"
                  >
                    <Select.Popup className="playthrough-select-popup">
                      <Select.List>
                        {playerRuns.map((run) => (
                          <Select.Item key={run.id} value={run.id}>
                            <Select.ItemText>{run.name}</Select.ItemText>
                            <Select.ItemIndicator>
                              <svg
                                viewBox="0 0 16 16"
                                width="16"
                                height="16"
                                fill="currentColor"
                                shapeRendering="crispEdges"
                                aria-hidden="true"
                              >
                                <path d="M1 8h2v2H1zm2 2h2v2H3zm2 2h2v2H5zm2-2h2v2H7zm2-2h2v2H9zm2-2h2v2h-2zm2-2h2v2h-2z" />
                              </svg>
                            </Select.ItemIndicator>
                          </Select.Item>
                        ))}
                        <Select.Separator />
                        <Select.Item value="__new__" label="New playthrough">
                          <Select.ItemText>＋ New playthrough</Select.ItemText>
                        </Select.Item>
                      </Select.List>
                    </Select.Popup>
                  </Select.Positioner>
                </Select.Portal>
              </Select.Root>
              <PlaythroughDialog
                mode="create"
                gameTitle={state.game.title}
                disabled={state.changing || launching}
                createOpen={createPlaythroughOpen}
                onCreateOpenChange={setCreatePlaythroughOpen}
                selectTrigger={playthroughTrigger}
              />
              <PlaythroughDialog
                mode="rename"
                gameTitle={state.game.title}
                currentName={state.playthrough.name}
                disabled={state.changing || launching}
              />
              <DeletePlaythroughDialog
                name={state.playthrough.name}
                disabled={state.changing || launching}
              />
            </div>
          )}
          <div className="console-toolbar" ref={toolbar}>
            <button
              className="icon-button"
              aria-label="Back up now"
              title="Back up now"
              disabled={!state.game || state.changing || !state.cloud.enabled}
              onClick={() => act(() => player.backupCloud())}
            >
              <Icon name="cloudUpload" />
            </button>
            <button
              className="icon-button"
              aria-label="Fullscreen"
              title="Fullscreen"
              onClick={() =>
                act(async () => {
                  if (document.fullscreenElement)
                    await document.exitFullscreen();
                  else
                    await document
                      .getElementById("game-display")
                      ?.requestFullscreen();
                })
              }
            >
              <Icon name="fullscreen" />
            </button>
            <div className="volume-control">
              <button
                className="icon-button"
                aria-label={muted ? "Unmute sound" : "Mute sound"}
                title={muted ? "Sound off" : "Sound on"}
                aria-pressed={muted}
                onClick={() =>
                  act(async () => {
                    await player.audio.mute();
                    setMuted(player.audio.muted);
                  })
                }
              >
                <Icon name={muted ? "muted" : "sound"} />
              </button>
              <div className="volume-panel">
                <label htmlFor="sound-volume">
                  Volume <output>{Math.round(volume * 100)}%</output>
                </label>
                <input
                  id="sound-volume"
                  type="range"
                  min="0"
                  max="100"
                  value={Math.round(volume * 100)}
                  onChange={(event) => {
                    const next = Number(event.target.value) / 100;
                    player.audio.setVolume(next);
                    setVolume(next);
                  }}
                />
              </div>
            </div>
            <Popover.Root
              open={menu === "speed"}
              onOpenChange={(open) =>
                setMenu((current) =>
                  open ? "speed" : current === "speed" ? null : current,
                )
              }
            >
              <Popover.Trigger
                className="icon-button speed-button"
                aria-label="Playback speed"
              >
                {state.boosting ? "16" : state.speed}×
              </Popover.Trigger>
              <Popover.Portal
                container={toolbar}
                style={{ display: "contents" }}
              >
                <Popover.Positioner
                  className="toolbar-menu-positioner"
                  style={{
                    position: "absolute",
                    top: 0,
                    left: 0,
                    transform: "none",
                    width: "100%",
                  }}
                >
                  <Popover.Popup
                    className="toolbar-menu speed-menu"
                    aria-label="Playback speed"
                  >
                    <h2>Playback speed</h2>
                    {[0.5, 1, 2, 4, 8, 16].map((speed) => (
                      <button
                        key={speed}
                        aria-pressed={state.speed === speed}
                        onClick={() => {
                          player.setSpeed(speed);
                          setMenu(null);
                        }}
                      >
                        {speed}× {state.speed === speed ? "✓" : ""}
                      </button>
                    ))}
                  </Popover.Popup>
                </Popover.Positioner>
              </Popover.Portal>
            </Popover.Root>
            <button
              className="icon-button"
              aria-label="How to play"
              title="How to play"
              aria-haspopup="dialog"
              aria-expanded={help}
              disabled={state.changing}
              onClick={openHelp}
            >
              <Icon name="gamepad" />
            </button>
            <Popover.Root
              open={menu === "more"}
              onOpenChange={(open) =>
                setMenu((current) =>
                  open ? "more" : current === "more" ? null : current,
                )
              }
            >
              <Popover.Trigger
                className="icon-button"
                aria-label="More options"
                title="Saves & options"
              >
                <Icon name="more" />
              </Popover.Trigger>
              <Popover.Portal
                container={toolbar}
                style={{ display: "contents" }}
              >
                <Popover.Positioner
                  className="toolbar-menu-positioner"
                  style={{
                    position: "absolute",
                    top: 0,
                    left: 0,
                    transform: "none",
                    width: "100%",
                  }}
                >
                  <Popover.Popup
                    className="toolbar-menu"
                    aria-label="Saves and options"
                  >
                    <h2>Saves & options</h2>
                    <label className="menu-file">
                      Import state
                      <input
                        type="file"
                        aria-label="Import state"
                        accept=".state"
                        disabled={!state.game || state.changing}
                        onChange={(event) => {
                          const file = event.target.files?.[0];
                          event.target.value = "";
                          if (file) act(() => importState(file));
                        }}
                      />
                    </label>
                    <button
                      disabled={!state.game || state.changing}
                      onClick={() =>
                        act(async () => {
                          await player.pause();
                          download(
                            await player.emulator.saveState(),
                            "game.state",
                          );
                          setMenu(null);
                        })
                      }
                    >
                      Export state
                    </button>
                    <button
                      disabled={!state.game || state.changing}
                      onClick={() =>
                        act(async () => {
                          await player.pause();
                          download(
                            await player.emulator.exportSave(),
                            "game.sav",
                          );
                          setMenu(null);
                        })
                      }
                    >
                      Export battery save
                    </button>
                    <label className="menu-file">
                      Import battery save
                      <input
                        type="file"
                        aria-label="Import battery save"
                        accept=".sav"
                        disabled={!state.game || state.changing}
                        onChange={(event) => {
                          const file = event.target.files?.[0];
                          event.target.value = "";
                          if (file && state.game)
                            act(async () => {
                              await player.open(
                                state.game!,
                                new Uint8Array(await file.arrayBuffer()),
                              );
                              setMenu(null);
                            });
                        }}
                      />
                    </label>
                    <hr />
                    <h2>Cloud progress</h2>
                    <p role="status">{state.cloud.message}</p>
                    {state.cloud.uploadedAt && (
                      <p>
                        Last confirmed:{" "}
                        {new Date(state.cloud.uploadedAt).toLocaleString()}
                      </p>
                    )}
                    <button
                      disabled={
                        !state.game || state.changing || !state.cloud.enabled
                      }
                      onClick={() => act(() => player.backupCloud())}
                    >
                      Back up now
                    </button>
                    <button
                      disabled={
                        !state.playthrough ||
                        state.changing ||
                        !state.cloud.accountId
                      }
                      onClick={() => {
                        setMenu(null);
                        const ref = player.activeRef;
                        if (ref) act(() => loadRun(ref, "restore"));
                      }}
                    >
                      Restore cloud resume
                    </button>
                    <button
                      disabled={
                        !state.game || state.changing || !state.cloud.accountId
                      }
                      onClick={() => {
                        if (
                          confirm(
                            "Restart from the cloud in-game save? Export local progress first if you want to keep both.",
                          )
                        )
                          act(() => player.restoreCloud("battery"));
                      }}
                    >
                      Restore cloud in-game save
                    </button>
                    <button
                      disabled={
                        !state.game || state.changing || !state.cloud.enabled
                      }
                      onClick={() => {
                        if (
                          confirm(
                            "Replace this game's cloud copies with local progress? This can overwrite progress from another device.",
                          )
                        )
                          act(() => player.backupCloud(true));
                      }}
                    >
                      Replace cloud with local progress
                    </button>
                    {!state.cloud.enabled && (
                      <button onClick={openAccount}>
                        Set up cloud backups
                      </button>
                    )}
                    <hr />
                    <h2>Quick slots</h2>
                    <p className="menu-note">
                      Shift/Ctrl + 1–9, 0 save/load.{" "}
                      {state.cloud.enabled
                        ? "Saved slots also back up to your account."
                        : "Slots stay on this device while cloud backups are off."}
                    </p>
                    {quickConflicts.length > 0 && (
                      <button onClick={openQuickReview}>
                        Resolve {quickConflicts.length} changed slot
                        {quickConflicts.length === 1 ? "" : "s"}…
                      </button>
                    )}
                    {Array.from({ length: QUICK_SLOTS }, (_, index) => {
                      const slot = index + 1;
                      const info = state.quickSlots.find(
                        (item) => item.slot === slot,
                      );
                      return (
                        <div className="slot-row" key={slot}>
                          <span>
                            Slot {slot}
                            <small>
                              {info?.savedAt === undefined
                                ? "Empty"
                                : info.savedAt
                                  ? new Date(info.savedAt).toLocaleString(
                                      undefined,
                                      {
                                        dateStyle: "short",
                                        timeStyle: "short",
                                      },
                                    )
                                  : "Saved"}
                              {info?.sync === "conflict"
                                ? " · differs"
                                : info?.sync === "pending"
                                  ? " · backing up"
                                  : info?.sync === "synced"
                                    ? " · synced"
                                    : ""}
                            </small>
                          </span>
                          <button
                            disabled={!state.playthrough || state.changing}
                            onClick={() => act(() => quickState(slot, false))}
                          >
                            Save
                          </button>
                          <button
                            disabled={
                              !state.playthrough ||
                              state.changing ||
                              info?.savedAt === undefined
                            }
                            onClick={() => act(() => quickState(slot, true))}
                          >
                            Load
                          </button>
                        </div>
                      );
                    })}
                  </Popover.Popup>
                </Popover.Positioner>
              </Popover.Portal>
            </Popover.Root>
          </div>
        </div>
        {library ? (
          <LibraryPage
            entries={[
              ...state.games.map((game) => ({
                id: game.hash,
                title: game.title,
                status: "installed" as const,
                romSize: game.romSize,
                lastPlayed: game.lastPlayed,
                resume:
                  game.hash === state.game?.hash || Boolean(game.resumeSavedAt),
                onPlay: () =>
                  loadRun({
                    hash: game.hash,
                    playthroughId: game.activePlaythroughId ?? "default",
                  }),
                playthroughs: [
                  ...state.runs
                    .filter((run) => run.hash === game.hash)
                    .map((run) => ({
                      id: run.id,
                      name: run.name,
                      localSavedAt: run.resumeSavedAt,
                      cloudUploadedAt: state.cloud.resumes?.find(
                        (remote) =>
                          remote.hash === game.hash &&
                          remote.playthroughId === run.id,
                      )?.uploadedAt,
                      onPlay: () =>
                        loadRun({ hash: game.hash, playthroughId: run.id }),
                    })),
                  ...(state.cloud.resumes ?? [])
                    .filter(
                      (remote) =>
                        remote.hash === game.hash &&
                        !state.runs.some(
                          (run) =>
                            run.hash === game.hash &&
                            run.id === remote.playthroughId,
                        ),
                    )
                    .map((remote) => ({
                      id: remote.playthroughId,
                      name: remote.playthroughName,
                      cloudUploadedAt: remote.uploadedAt,
                      onPlay: () =>
                        loadRun({
                          hash: game.hash,
                          playthroughId: remote.playthroughId,
                        }),
                    })),
                ],
              })),
              ...(state.cloud.resumes ?? [])
                .filter(
                  (remote, index, all) =>
                    !state.games.some((game) => game.hash === remote.hash) &&
                    all.findIndex((other) => other.hash === remote.hash) ===
                      index,
                )
                .map((remote) => ({
                  id: remote.hash,
                  title: remote.title,
                  status: "cloud" as const,
                  onPlay: () =>
                    loadRun({
                      hash: remote.hash,
                      playthroughId: remote.playthroughId,
                    }),
                  playthroughs: (state.cloud.resumes ?? [])
                    .filter((run) => run.hash === remote.hash)
                    .map((run) => ({
                      id: run.playthroughId,
                      name: run.playthroughName,
                      cloudUploadedAt: run.uploadedAt,
                      onPlay: () =>
                        loadRun({
                          hash: run.hash,
                          playthroughId: run.playthroughId,
                        }),
                    })),
                })),
            ]}
            busy={!state.ready || state.changing || launching}
            onAddGame={() => romInput.current?.click()}
          />
        ) : home ? (
          <>
            <AboutPage
              onPlay={resumeFromHome}
              onLibrary={() => browse("library")}
              onAccount={openAccount}
              hasGame={canResume}
              resumeTitle={
                lastGame
                  ? `${lastGame.title} · ${lastRun?.name ?? "Playthrough 1"}`
                  : cloudFallback
                    ? `${cloudFallback.title} · ${cloudFallback.playthroughName}`
                    : undefined
              }
              resumeDetail={
                lastRun?.resumeSavedAt
                  ? `On this device · ${new Date(lastRun.resumeSavedAt).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" })}`
                  : cloudFallback
                    ? `Cloud checkpoint · ${new Date(cloudFallback.uploadedAt).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" })}`
                    : undefined
              }
              busy={!state.ready || state.changing || launching}
            />
          </>
        ) : Layout ? (
          <Layout key={experience.game.id} playing={state.playing}>
            {display}
          </Layout>
        ) : (
          <main className="generic-experience" data-game-layout="generic">
            {display}
          </main>
        )}
        <span className="sr-only" role="status">
          {state.message}
        </span>
        {notice && (
          <div className="console-notice" role="status">
            {notice}
            <button
              aria-label="Dismiss notification"
              onClick={() => setNotice("")}
            >
              ×
            </button>
          </div>
        )}
        {dragging && (
          <div className="drop-message">Drop your game or state</div>
        )}
        {accountOpen && (
          <Modal title="Account" close={closeAccount}>
            <Account />
          </Modal>
        )}
        {cloudResumeRequest && (
          <Modal
            title="Resume from cloud"
            close={() => setCloudResumeRequest(null)}
            busy={launching}
            className="cloud-resume-dialog"
          >
            <p>
              <strong>
                {cloudResumeRequest.title} ·{" "}
                {cloudResumeRequest.playthroughName}
              </strong>
            </p>
            {cloudResumeHasLocal && (
              <p>
                On this device:{" "}
                {cloudLocalRun?.resumeSavedAt
                  ? new Date(cloudLocalRun.resumeSavedAt).toLocaleString()
                  : "In-game save available"}
              </p>
            )}
            <p>
              Cloud checkpoint uploaded:{" "}
              {new Date(cloudResumeRequest.uploadedAt).toLocaleString(
                undefined,
                { dateStyle: "medium", timeStyle: "short" },
              )}
            </p>
            {cloudResumeHasLocal && (
              <p>
                Choose which copy to use. Restoring the cloud checkpoint
                replaces this playthrough’s local progress; other playthroughs
                and the cloud copy remain unchanged.
              </p>
            )}
            {!cloudLocalGame && (
              <p>
                Choose the exact game version to resume. Your ROM stays on this
                device and is never uploaded.
              </p>
            )}
            <details className="cloud-rom-identity">
              <summary>ROM details</summary>
              SHA-256: <code>{cloudResumeRequest.hash}</code>
            </details>
            {cloudResumeError && <p role="alert">{cloudResumeError}</p>}
            {launching && (
              <p role="status">Validating and restoring progress…</p>
            )}
            <div className="resume-recovery-actions">
              {cloudResumeHasLocal && cloudLocalGame && (
                <button
                  disabled={launching}
                  onClick={() => {
                    setCloudResumeRequest(null);
                    act(() =>
                      launchGame({
                        ...cloudLocalGame,
                        activePlaythroughId: cloudResumeRequest.playthroughId,
                      }),
                    );
                  }}
                >
                  Continue on this device instead
                </button>
              )}
              {cloudLocalGame && (
                <button
                  disabled={launching}
                  onClick={() => void launchCloud(cloudResumeRequest)}
                >
                  Resume cloud checkpoint
                </button>
              )}
              <Button
                disabled={launching}
                onClick={() => cloudRomInput.current?.click()}
              >
                {cloudLocalGame
                  ? "Choose another ROM file"
                  : "Choose ROM and resume"}
              </Button>
              <input
                ref={cloudRomInput}
                hidden
                type="file"
                accept=".gba,.zip"
                aria-label="Choose ROM for cloud resume"
                disabled={launching}
                onChange={(event) => {
                  const file = event.target.files?.[0];
                  event.target.value = "";
                  if (file) void launchCloud(cloudResumeRequest, file);
                }}
              />
            </div>
          </Modal>
        )}
        {resumeFailure && (
          <Modal
            title="Resume needs attention"
            close={() => setResumeFailure(null)}
          >
            <p>{resumeFailure.message}</p>
            <p>
              The checkpoint is preserved. Retry it, or open{" "}
              {resumeFailure.game.title} from its separate in-game save instead.
            </p>
            <div className="resume-recovery-actions">
              <button onClick={() => act(() => launchGame(resumeFailure.game))}>
                Retry checkpoint
              </button>
              <button
                onClick={() => act(() => launchGame(resumeFailure.game, true))}
              >
                Start from in-game save
              </button>
            </div>
          </Modal>
        )}
        {quickReview && quickConflicts.length > 0 && (
          <Modal
            title="Quick slots changed"
            close={closeQuickReview}
            busy={quickBusy}
            className="quick-conflict-dialog"
          >
            <QuickConflictChoices
              key={quickConflicts.map((conflict) => conflict.etag).join()}
              conflicts={quickConflicts}
              busy={quickBusy}
              later={closeQuickReview}
              apply={(choices) => {
                setQuickBusy(true);
                void player
                  .resolveQuickConflicts(choices)
                  .catch(player.error)
                  .finally(() => setQuickBusy(false));
              }}
            />
          </Modal>
        )}
        {help && (
          <Modal
            title="How to play"
            close={closeHelp}
            className="controls-dialog"
          >
            <PlayerHelp
              controls={controls}
              toggleControls={() => setControls(!controls)}
            />
          </Modal>
        )}
      </div>
    </BridgeProvider>
  );
}

// Installed game plugins' layouts (and their styles) load before the first
// render, so a dashboard never replaces the player under a running game. A
// plugin that fails to load leaves the generic player.
void gameRegistry
  .load("dashboards")
  .catch((error: unknown) => console.error(error))
  .finally(() => createRoot(document.getElementById("root")!).render(<App />));

if (import.meta.env.PROD && "serviceWorker" in navigator) {
  void navigator.serviceWorker
    .register(new URL("service-worker.js", document.baseURI))
    .catch(() => {
      /* Cache/storage denial must not prevent online or local play. */
    });
}
