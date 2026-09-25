import "./app-header.css";

export type AppView = "home" | "player" | "library";

/** Navigation belongs to the app; page-specific controls stay inside the page. */
export function AppHeader({
  view,
  onNavigate,
  onAddGame,
  onAccount,
  busy = false,
}: {
  view: AppView;
  onNavigate: (view: AppView) => void;
  onAddGame: () => void;
  onAccount: () => void;
  busy?: boolean;
}) {
  return (
    <header className="app-header">
      <button
        className="app-wordmark"
        aria-label="ZiGBA home"
        disabled={busy}
        onClick={() => onNavigate("home")}
      >
        ZIGBA
      </button>
      <nav aria-label="Main navigation">
        {(["home", "player", "library"] as const).map((destination) => (
          <button
            key={destination}
            aria-current={view === destination ? "page" : undefined}
            disabled={busy}
            onClick={() => onNavigate(destination)}
          >
            {destination[0].toUpperCase() + destination.slice(1)}
          </button>
        ))}
      </nav>
      <button className="app-add" disabled={busy} onClick={onAddGame}>
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <path d="M12 4v16M4 12h16" />
        </svg>
        <span>Add game</span>
      </button>
      <button
        className="app-account"
        disabled={busy}
        onClick={onAccount}
        aria-haspopup="dialog"
        aria-label="Account"
      >
        <svg viewBox="0 0 24 24" aria-hidden="true">
          <circle cx="12" cy="8" r="4" />
          <path d="M4 22v-3a8 8 0 0 1 16 0v3" />
        </svg>
        <span>Account</span>
      </button>
    </header>
  );
}
