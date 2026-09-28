import {
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react";
import { createRoot } from "react-dom/client";
import {
  createRootRoute,
  createRoute,
  createRouter,
  redirect,
  RouterProvider,
  useNavigate,
  useRouterState,
} from "@tanstack/react-router";
import { Button } from "@base-ui/react/button";
import { Dialog } from "@base-ui/react/dialog";
import { Popover } from "@base-ui/react/popover";
import { Select } from "@base-ui/react/select";
import { player } from "./player";
import type { BridgeHost } from "@evenfall/gba/bridge";
import { BridgeProvider } from "@evenfall/gba/bridge/react";
import { ConsolePlayer, PlayerHelp, quickState } from "./console-player";
import {
  DeletePlaythroughDialog,
  PlaythroughDialog,
} from "./playthrough-dialog";
import { LandingPage } from "./pages/landing/landing-page";
import { LibraryPage } from "./pages/library/library-page";
import type { LibraryAction } from "./pages/library/actions";
import type { LibraryDevice } from "./pages/library/catalog";
import { patchRelease } from "./pages/library/install";
import { useCatalog, useDumpNames } from "./pages/library/use-catalog";
import { AppHeader, type AppView } from "./app-header";
import { AppUpdate } from "./app-update";
import { Account } from "./account";
import {
  fetchSharedRom,
  fetchSharedState,
  type CloudResume,
  type SharedState,
} from "./cloud";
import {
  nameTaken,
  patchedRelease,
  ShareLink,
  SharedStateChoices,
  type PatchedRelease,
  type ShareProgress,
} from "./share-state";
import {
  playthroughLockKey,
  QUICK_SLOTS,
  type CloudJob,
  type Game,
  type PlaythroughRef,
} from "./library";
import { acquireSaveLock } from "./storage.js";
import { QuickConflictChoices } from "./quick-conflicts";
import { gameRegistry, watchBuild } from "./games";
import { download } from "./media";
import { trackShare, trackView } from "./analytics";
import { cx } from "./ui/styles";
import "./styles.css";

const act = (action: () => Promise<unknown>) =>
  void action().catch(player.error);
const appUpdate = new AppUpdate({
  idle: () =>
    !player.state.game &&
    !player.state.changing &&
    !document.querySelector('dialog[open],[role="dialog"]'),
  beforeReload: () => player.pause(),
});
const icons = {
  cloudUpload: (
    <>
      <path d="M12 13v8m-4-4 4-4 4 4" />
      <path d="M20.88 18.09A5 5 0 0 0 18 9h-1.26A8 8 0 1 0 4 16.25" />
    </>
  ),
  share: (
    <>
      <path d="M12 15V3m-4 4 4-4 4 4" />
      <path d="M8 10H5v11h14V10h-3" />
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
  const update = useSyncExternalStore(appUpdate.subscribe, appUpdate.snapshot);
  const [updateDismissed, setUpdateDismissed] = useState(false);
  const [menu, setMenu] = useState<"speed" | "more" | null>(null);
  // The URL is the page, so a refresh or a shared link lands on it again.
  const navigate = useNavigate();
  const page = useRouterState({
    select: (state) => state.matches[state.matches.length - 1],
  });
  const view: AppView =
    page.routeId === "/player"
      ? "player"
      : page.routeId === "/library" || page.routeId === "/library/$gameId"
        ? "library"
        : "home";
  const libraryGameId =
    page.routeId === "/library/$gameId" ? page.params.gameId : undefined;
  // A share link opens over Home and leaves for Player once it starts.
  const shareId =
    page.routeId === "/share/state/$shareId" ? page.params.shareId : undefined;
  const home = view === "home";
  const library = view === "library";
  const {
    catalog,
    offline: catalogOffline,
    settled: catalogSettled,
  } = useCatalog();
  const dumpNames = useDumpNames(library);
  const [cloudJobs, setCloudJobs] = useState<CloudJob[]>([]);
  const libraryDevice = useMemo(
    (): LibraryDevice => ({
      games: state.games,
      playthroughs: state.runs,
      cloud: state.cloud.resumes ?? [],
      jobs: cloudJobs,
    }),
    [state.games, state.runs, state.cloud.resumes, cloudJobs],
  );
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
  const [share, setShare] = useState<ShareProgress | null>(null);
  const shareResume = useRef(false);
  const [sharedOpen, setSharedOpen] = useState<{
    id: string;
    shared?: SharedState;
    /** The catalog release the shared ROM is, when a patch builds it. */
    release?: PatchedRelease;
    /** Neither the shared ROM nor the release's base is on this device. */
    needsRom?: boolean;
    /** The user was asked for a name or ROM, so the choices stay on screen. */
    prompted?: boolean;
    busy?: boolean;
    error?: string;
  } | null>(null);
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
  const romHash = state.game?.hash;
  // Bumped when a build is recognized or forgotten, to pick a layout again.
  const [, setBuilds] = useState(0);
  // The ROM whose build is being located after its game was detected.
  const [recognizing, setRecognizing] = useState<string>();
  const experience = gameRegistry.layoutForRom("dashboards", romHash);
  const Layout = experience?.Layout;
  const build = gameRegistry.buildInfo(romHash);
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
  // A ROM no reviewed build matches may still be a build of an installed
  // game: recognize it from the cache or the game's locators, then show its
  // layout.
  useEffect(() => {
    if (!state.ready || !romHash || gameRegistry.gameForRom(romHash)) return;
    const controller = new AbortController();
    void gameRegistry
      .recognize(
        romHash,
        async () => {
          if (player.state.game?.hash !== romHash)
            throw new Error("The game changed during recognition");
          return player.emulator.exportRom();
        },
        {
          cache: player.library.builds,
          signal: controller.signal,
          onLocating: () => setRecognizing(romHash),
        },
      )
      .then((result) => {
        if (result.status !== "unsupported") setBuilds((count) => count + 1);
      })
      .catch((error: unknown) => {
        if (!controller.signal.aborted) console.error(error);
      })
      .finally(() =>
        setRecognizing((hash) => (hash === romHash ? undefined : hash)),
      );
    return () => controller.abort();
  }, [state.ready, romHash]);
  // A located build whose views never decode is dropped for the generic player.
  useEffect(() => {
    if (!experience || !romHash || build?.provenance !== "located") return;
    const store = player.session.bridges.observe(
      // Plugins list their games as identities; each is a game definition.
      experience.game as Parameters<BridgeHost["observe"]>[0],
    );
    return watchBuild(store, () => {
      setNotice("This game's dashboard could not read this build.");
      void gameRegistry
        .forget(romHash, player.library.builds)
        .catch((error: unknown) => console.error(error))
        .finally(() => setBuilds((count) => count + 1));
    });
  }, [experience?.game, romHash, build?.provenance]);
  // Swapping layouts remounts the screen; keep keyboard focus on it.
  useEffect(() => {
    if (document.activeElement === document.body) player.canvas?.focus();
  }, [experience?.game]);
  // A share link's ID is its only credential, so its page is never tracked.
  useEffect(() => {
    if (!shareId) trackView(view);
  }, [view, shareId]);
  // The shared ROM may be a catalog release that a base on this device
  // becomes, so a link opens once the catalog is as fresh as it will get.
  useEffect(() => {
    if (!shareId || !state.ready || !catalogSettled) return;
    let current = true;
    setSharedOpen({ id: shareId });
    void fetchSharedState(shareId).then(
      (shared) => {
        if (!current) return;
        const release = patchedRelease(catalog.versions, shared.hash);
        const needsRom = needsSharedRom(shared, release);
        const prompted =
          needsRom ||
          nameTaken(playthroughNames(shared.hash), shared.playthroughName);
        const open = { id: shareId, shared, release, needsRom, prompted };
        if (prompted) setSharedOpen(open);
        else void startShared(open, shared.playthroughName);
      },
      (error: unknown) => {
        if (current)
          setSharedOpen({
            id: shareId,
            error: error instanceof Error ? error.message : String(error),
          });
      },
    );
    return () => {
      current = false;
    };
  }, [shareId, state.ready, catalogSettled]);
  useEffect(() => {
    if ((home || library) && state.ready && state.cloud.accountId)
      void player.cloud.discover();
  }, [home, library, state.ready, state.cloud.accountId]);
  // Sync states compare this outbox with the cloud listing.
  useEffect(() => {
    if (!library || !state.ready) return;
    let current = true;
    player.library
      .cloudJobs(state.cloud.accountId ?? "")
      .then((jobs) => current && setCloudJobs(jobs), player.error);
    return () => {
      current = false;
    };
  }, [
    library,
    state.ready,
    state.cloud.accountId,
    state.cloud.uploadedAt,
    state.runs,
  ]);
  useEffect(() => {
    if (library && catalogOffline)
      setNotice("Catalog unavailable. Showing the last copy on this device.");
  }, [library, catalogOffline]);
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
  function openShare() {
    shareResume.current = player.state.playing;
    setMenu(null);
    createShare();
  }
  function createShare() {
    if (!player.state.cloud.accountId) {
      setShare({ status: "signed-out" });
      act(() => player.pause());
      return;
    }
    setShare({ status: "creating" });
    player.shareState().then(
      (link) => {
        trackShare("state_share_created");
        setShare({ status: "ready", ...link });
      },
      (error: unknown) =>
        setShare({
          status: "failed",
          message: error instanceof Error ? error.message : String(error),
        }),
    );
  }
  function closeShare() {
    setShare(null);
    if (shareResume.current && !player.state.changing) act(() => player.play());
  }
  /** Names the user already sees for a game: local runs and cloud-only runs. */
  function playthroughNames(hash: string) {
    return [
      ...player.state.runs
        .filter((run) => run.hash === hash)
        .map((run) => run.name),
      ...(player.state.cloud.resumes ?? [])
        .filter((run) => run.hash === hash)
        .map((run) => run.playthroughName),
    ];
  }
  const romStored = (hash: string) =>
    player.state.games.some((game) => game.hash === hash);
  /**
   * Whether a file is needed: the ROM is not here, nor its patch's base, and
   * the sharer did not include it.
   */
  function needsSharedRom(shared: SharedState, release?: PatchedRelease) {
    return (
      !romStored(shared.hash) &&
      !(release && romStored(release.requiredBase.sha1)) &&
      !shared.romIncluded
    );
  }
  /**
   * Starts a shared state from the stored ROM. A chosen `file` is stored
   * first if it is that ROM or the release's base, as the Library's Choose
   * file does. Without the ROM, a stored base is patched into the release, as
   * Install does; failing that, an included ROM is downloaded and stored once
   * its SHA-1 matches.
   */
  async function startShared(
    open: NonNullable<typeof sharedOpen> & { shared: SharedState },
    name: string,
    file?: File,
  ) {
    const { id, shared, release } = open;
    setSharedOpen({ ...open, busy: true, error: undefined });
    try {
      await exclusive(async () => {
        const rom = {
          sha1: shared.hash,
          title:
            (release &&
              catalog.games.find((game) => game.id === release.gameId)
                ?.displayName) ??
            shared.title,
        };
        const base = release?.requiredBase;
        if (file)
          await libraryAction({
            type: "add-file",
            file,
            wanted: [
              rom,
              ...(base ? [{ sha1: base.sha1, title: base.displayName }] : []),
            ],
          });
        if (!romStored(shared.hash)) {
          if (release && romStored(release.requiredBase.sha1))
            await libraryAction({ type: "install", versionId: release.id });
          else if (shared.romIncluded) {
            const downloaded = await player.inspectRom(
              await fetchSharedRom(id),
            );
            if (downloaded.hash !== shared.hash)
              throw new Error("The shared game does not match this state.");
            await player.library.addRom({
              hash: downloaded.hash,
              title: rom.title,
              rom: downloaded.rom,
              lastPlayed: 0,
            });
            await player.refresh();
          }
        }
        await player.openSharedState(shared, name);
        trackShare("state_share_opened");
        navigationResume.current = false;
        // Replace the link, so Back cannot start it a second time.
        showView("player", true);
      });
      setSharedOpen(null);
    } catch (error) {
      setSharedOpen({
        ...open,
        // A chosen base stays stored even when its patch then fails.
        needsRom: needsSharedRom(shared, release),
        prompted: true,
        busy: false,
        error: error instanceof Error ? error.message : String(error),
      });
    }
  }
  function closeShared() {
    setSharedOpen(null);
    void navigate({ to: "/", replace: true });
  }
  /** Leaving Player pauses it; returning resumes a session that was playing. */
  function transition(destination: AppView) {
    if (navigation.current === destination) return;
    if (navigation.current === "player") {
      navigationResume.current ||= player.state.playing;
      player.releaseKeys();
      // A resume may still be awaiting its worker acknowledgement. Pause after
      // it settles, so a late play result cannot overwrite the paused state.
      navigationWork.current = navigationWork.current
        .then(() => player.pause())
        .catch(player.error);
    }
    navigation.current = destination;
    setMenu(null);
    if (destination !== "player") return;
    const shouldResume = navigationResume.current;
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
  // Back, Forward and typed URLs take the same steps as the app's controls.
  useLayoutEffect(() => transition(view), [view]);
  useEffect(() => {
    if (page.routeId === "__root__") void navigate({ to: "/", replace: true });
  }, [page.routeId]);
  function showView(destination: AppView, replace = false) {
    transition(destination);
    void navigate({ to: paths[destination], replace });
  }
  function browse(destination: "home" | "library") {
    if (player.state.changing) return;
    showView(destination);
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
    showView("player");
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
  /** Runs one launch or storage change at a time; a second is refused. */
  async function exclusive(work: () => Promise<void>) {
    if (launchPending.current || player.state.changing)
      throw new Error("Another game action is still running.");
    launchPending.current = true;
    setLaunching(true);
    try {
      await navigationWork.current;
      await work();
    } finally {
      launchPending.current = false;
      setLaunching(false);
    }
  }
  async function launchGame(game: Game, fromBattery = false) {
    if (launchPending.current || player.state.changing) return;
    await exclusive(() => launchGameInternal(game, fromBattery));
  }
  const isActive = (ref: PlaythroughRef) =>
    player.activeRef?.hash === ref.hash &&
    player.activeRef.playthroughId === ref.playthroughId;
  function storedGame(hash: string) {
    const game = player.state.games.find((item) => item.hash === hash);
    if (!game) throw new Error("This game's ROM is not on this device.");
    return game;
  }
  async function libraryAction(action: LibraryAction) {
    switch (action.type) {
      case "play": {
        const game = player.state.games.find(
          (item) => item.hash === action.hash,
        );
        const playthroughId =
          action.playthroughId ?? game?.activePlaythroughId ?? "default";
        const ref = { hash: action.hash, playthroughId };
        if (action.progress === "device")
          return exclusive(() =>
            launchGameInternal(storedGame(action.hash), false, playthroughId),
          );
        if (action.progress === "cloud") {
          const backup = player.state.cloud.resumes?.find(
            (item) =>
              item.hash === ref.hash &&
              item.playthroughId === ref.playthroughId,
          );
          if (!backup)
            throw new Error("This cloud backup is no longer available.");
          return exclusive(async () => {
            await player.restoreCloud("resume", backup);
            navigationResume.current = false;
            showView("player");
          });
        }
        return loadRun(ref);
      }
      case "install": {
        const version = catalog.versions.find(
          (item) => item.id === action.versionId,
        );
        const base = version?.requiredBase;
        if (!version || !base)
          throw new Error("This release is no longer in the catalog.");
        const source = await player.library.rom(base.sha1);
        if (!source) throw new Error(`Add ${base.displayName} first.`);
        const rom = await patchRelease(version, source);
        await player.library.addRom({
          hash: version.outputSha1,
          title:
            catalog.games.find((item) => item.id === version.gameId)
              ?.displayName ?? version.displayName,
          rom,
          lastPlayed: 0,
        });
        await player.refresh();
        if (action.play)
          await exclusive(() =>
            launchGameInternal(storedGame(version.outputSha1)),
          );
        return;
      }
      case "add-file": {
        const rom = await player.inspectRom(action.file);
        const wanted = action.wanted.find((item) => item.sha1 === rom.hash);
        if (!wanted)
          throw new Error(
            `That file isn’t ${[...new Set(action.wanted.map((item) => item.title))].join(" or ")}. Choose that exact ROM.`,
          );
        await player.library.addRom({
          hash: rom.hash,
          title: wanted.title,
          rom: rom.rom,
          lastPlayed: 0,
        });
        return player.refresh();
      }
      case "new-playthrough": {
        const game = storedGame(action.hash);
        return exclusive(async () => {
          if (game.hash === player.state.game?.hash && player.activeRef) {
            await player.switchPlaythrough(undefined, action.name);
            navigationResume.current = false;
            showView("player");
          } else {
            const run = await player.library.createPlaythrough(
              game.hash,
              action.name,
            );
            await launchGameInternal(game, false, run.id);
          }
        });
      }
      case "update": {
        const game = storedGame(action.toHash);
        return exclusive(async () => {
          if (isActive(action.from)) await player.pause();
          const save = await player.library.getSave(action.from);
          if (!save?.length)
            throw new Error("This playthrough has no in-game save to copy.");
          const run = await player.library.createPlaythrough(
            game.hash,
            action.name,
          );
          await player.library.save(
            { hash: game.hash, playthroughId: run.id },
            save,
          );
          await launchGameInternal(game, true, run.id);
        });
      }
      case "export-save": {
        const active = isActive(action);
        if (active) await player.pause();
        const save = active
          ? await player.emulator.exportSave()
          : await player.library.getSave(action);
        if (!save?.length)
          throw new Error("This playthrough has no in-game save yet.");
        const title =
          player.state.games.find((item) => item.hash === action.hash)?.title ??
          "Game";
        download(
          save,
          `${title} - ${action.name}.sav`.replace(/[\\/:*?"<>|]/g, "_"),
        );
        return;
      }
      case "rename-playthrough":
        if (isActive(action)) return player.renamePlaythrough(action.name);
        await player.library.renamePlaythrough(action, action.name);
        return player.refresh();
      case "delete-playthrough": {
        if (isActive(action)) return player.deletePlaythrough();
        const release = await acquireSaveLock(playthroughLockKey(action));
        if (!release && navigator.locks)
          throw new Error(
            "Another tab has this playthrough open. Close it before deleting.",
          );
        try {
          await player.cloud.deletePlaythrough(action);
        } finally {
          release?.();
        }
        return player.refresh();
      }
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
      } else if (game && !run && game.activePlaythroughId) {
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
    navigationResume.current = false;
    showView("player");
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
        className={cx(
          "console-app",
          dragging && "dragging",
          library && "bg-canvas md:h-dvh md:overflow-hidden",
        )}
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
            onAccount={openAccount}
            back={
              library && libraryGameId
                ? {
                    label: "Library",
                    onBack: () => void navigate({ to: "/library" }),
                  }
                : undefined
            }
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
              aria-label="Share state"
              title="Share state"
              aria-haspopup="dialog"
              disabled={
                !state.playthrough || state.changing || state.resumeFailed
              }
              onClick={openShare}
            >
              <Icon name="share" />
            </button>
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
            catalog={catalog}
            dumpNames={dumpNames}
            device={libraryDevice}
            busy={!state.ready || state.changing || launching}
            selectedId={libraryGameId}
            onSelect={(gameId) =>
              void navigate({ to: "/library/$gameId", params: { gameId } })
            }
            onBack={() => void navigate({ to: "/library" })}
            onAddGame={() => romInput.current?.click()}
            onAction={libraryAction}
          />
        ) : home ? (
          <>
            <LandingPage
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
          <Layout
            key={experience.game.id}
            playing={state.playing}
            build={build}
          >
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
        <div className="console-notices">
          {romHash && recognizing === romHash && !notice && (
            <div className="console-notice" role="status">
              Recognizing game…
            </div>
          )}
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
          {update !== "current" && !updateDismissed && (
            <div className="update-notice" role="status">
              A new version of Evenfall is ready.
              <button
                className="update-reload"
                disabled={update === "reloading"}
                onClick={() => act(() => appUpdate.apply())}
              >
                {update === "reloading" ? "Updating…" : "Update now"}
              </button>
              <button
                aria-label="Dismiss update"
                disabled={update === "reloading"}
                onClick={() => setUpdateDismissed(true)}
              >
                ×
              </button>
            </div>
          )}
        </div>
        {dragging && (
          <div className="drop-message">Drop your game or state</div>
        )}
        {accountOpen && (
          <Modal title="Account" close={closeAccount}>
            <Account />
          </Modal>
        )}
        {share && state.game && state.playthrough && (
          <Modal
            title="Share state"
            close={closeShare}
            busy={share.status === "creating"}
            className="share-dialog"
          >
            <ShareLink
              progress={share}
              title={state.game.title}
              playthroughName={state.playthrough.name}
              retry={createShare}
              signIn={() => {
                const resume = shareResume.current;
                setShare(null);
                openAccount();
                accountResume.current = resume;
              }}
              includeRom={
                share.status === "ready" && share.canIncludeRom
                  ? () => player.includeShareRom(share.id, share.hash)
                  : undefined
              }
            />
          </Modal>
        )}
        {sharedOpen && sharedOpen.id === shareId && (
          <Modal
            title="Shared state"
            close={closeShared}
            busy={
              Boolean(sharedOpen.busy) ||
              (!sharedOpen.shared && !sharedOpen.error)
            }
            className="shared-state-dialog"
          >
            {sharedOpen.shared && sharedOpen.prompted ? (
              <SharedStateChoices
                shared={sharedOpen.shared}
                needsRom={Boolean(sharedOpen.needsRom)}
                base={
                  sharedOpen.needsRom
                    ? sharedOpen.release?.requiredBase
                    : undefined
                }
                takenNames={playthroughNames(sharedOpen.shared.hash)}
                busy={Boolean(sharedOpen.busy)}
                error={sharedOpen.error ?? ""}
                start={(name, file) =>
                  void startShared(
                    { ...sharedOpen, shared: sharedOpen.shared! },
                    name,
                    file,
                  )
                }
              />
            ) : sharedOpen.error ? (
              <>
                <p role="alert">{sharedOpen.error}</p>
                <div className="resume-recovery-actions">
                  <button onClick={closeShared}>Go to Home</button>
                </div>
              </>
            ) : (
              <p role="status">
                {sharedOpen.shared
                  ? `Starting ${sharedOpen.shared.title} · ${sharedOpen.shared.playthroughName}…`
                  : "Opening shared state…"}
              </p>
            )}
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
              SHA-1: <code>{cloudResumeRequest.hash}</code>
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

const paths = { home: "/", library: "/library", player: "/player" } as const;

// Servers answer these paths with the player (see tools/serve-web.py, the
// Worker and offline.ts); every other path is a 404.
const rootRoute = createRootRoute({ component: App });
// A fresh page has no game running, so /player opens the Library instead.
// Later visits show the Player as it is.
let started = false;
const router = createRouter({
  routeTree: rootRoute.addChildren([
    createRoute({ getParentRoute: () => rootRoute, path: "/" }),
    createRoute({ getParentRoute: () => rootRoute, path: "/library" }),
    createRoute({ getParentRoute: () => rootRoute, path: "/library/$gameId" }),
    createRoute({
      getParentRoute: () => rootRoute,
      path: "/share/state/$shareId",
    }),
    createRoute({
      getParentRoute: () => rootRoute,
      path: "/player",
      beforeLoad: () => {
        if (!started) throw redirect({ to: "/library", replace: true });
      },
    }),
  ]),
  basepath: import.meta.env.BASE_URL,
});
router.subscribe("onResolved", () => {
  started = true;
});
declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}

// Installed game plugins' layouts (and their styles) load before the first
// render, so a dashboard never replaces the player under a running game. A
// plugin that fails to load leaves the generic player.
void gameRegistry
  .load("dashboards")
  .catch((error: unknown) => console.error(error))
  .finally(() =>
    createRoot(document.getElementById("root")!).render(
      <RouterProvider router={router} />,
    ),
  );

if (import.meta.env.PROD && "serviceWorker" in navigator) {
  void appUpdate
    .start(
      new URL(`${import.meta.env.BASE_URL}service-worker.js`, location.origin),
    )
    .catch(() => {
      /* Cache/storage denial must not prevent online or local play. */
    });
}
