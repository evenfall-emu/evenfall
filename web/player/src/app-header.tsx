import logo from "./assets/brand/evenfall-logo.svg";
import coast from "./assets/sunset/coast.png";
import { PixelIcon, type PixelIconName } from "./ui/icons";
import { button, cx, focusRing } from "./ui/styles";

export type AppView = "home" | "player" | "library" | "manage";

const tabs: { view: AppView; label: string; icon: PixelIconName }[] = [
  { view: "library", label: "Library", icon: "book" },
  { view: "player", label: "Player", icon: "play" },
  { view: "manage", label: "Manage", icon: "manage" },
];

function Tabs({
  view,
  busy,
  manage,
  compact = false,
  onNavigate,
}: {
  view: AppView;
  busy: boolean;
  manage: boolean;
  /** The phone row: Manage shows only its icon, so three tabs fit. */
  compact?: boolean;
  onNavigate: (view: AppView) => void;
}) {
  return (
    <nav aria-label="Main navigation" className="flex self-stretch">
      {tabs.map((tab) => {
        if (tab.view === "manage" && !manage) return null;
        const current = view === tab.view;
        return (
          <button
            key={tab.view}
            aria-current={current ? "page" : undefined}
            disabled={busy}
            onClick={() => onNavigate(tab.view)}
            className={cx(
              "relative flex cursor-pointer items-center gap-2 px-2 disabled:cursor-not-allowed disabled:opacity-60 sm:gap-3 sm:px-4 md:px-6",
              "after:absolute after:inset-x-3 after:bottom-0 after:h-1 after:rounded-t-control",
              current
                ? "text-mint after:bg-mint"
                : "text-content hover:text-mint-hover",
              focusRing,
              "focus-visible:-outline-offset-2",
            )}
          >
            <PixelIcon
              name={tab.icon}
              className={cx(!current && tab.icon === "play" && "text-info")}
            />
            {compact && tab.view === "manage" ? (
              <span className="sr-only">{tab.label}</span>
            ) : (
              tab.label
            )}
          </button>
        );
      })}
    </nav>
  );
}

/**
 * The pixel horizon of `coast.png`, anchored to the header's right edge
 * so the sun stays in view at any width, and faded in from the left. Between
 * the phone layout and wide desktops the tabs leave it too little room.
 */
function Banner({ className }: { className: string }) {
  return (
    <div
      aria-hidden="true"
      className={cx(
        "pointer-events-none bg-no-repeat [mask-image:linear-gradient(to_right,transparent,black_30%,black_90%,transparent)]",
        className,
      )}
      style={{ backgroundImage: `url(${coast})` }}
    />
  );
}

/**
 * The navigation shared by Library, Player and Manage. Home has its own
 * header; the logo returns there. Manage is a tab only for accounts that may
 * manage the catalog, even on its own page. On narrow screens a page can
 * replace the tabs with a Back button.
 */
export function AppHeader({
  view,
  onNavigate,
  onAccount,
  back,
  busy = false,
  manage = false,
}: {
  view: AppView;
  onNavigate: (view: AppView) => void;
  onAccount: () => void;
  back?: { label: string; onBack: () => void };
  busy?: boolean;
  /** Whether the account may manage the catalog, as `/api/me` says. */
  manage?: boolean;
}) {
  return (
    <header
      data-ui
      className="relative isolate shrink-0 border-b border-line bg-canvas font-sans text-2x text-content"
    >
      <div className="relative flex h-18 items-center gap-6 px-4 sm:px-6 md:h-22 md:gap-10 md:px-8">
        <button
          aria-label="evenfall home"
          disabled={busy}
          onClick={() => onNavigate("home")}
          className={cx(
            "z-10 shrink-0 cursor-pointer rounded-control",
            focusRing,
          )}
        >
          <img src={logo} alt="" className="h-10 w-auto md:h-12" />
        </button>
        <div className="hidden self-stretch md:flex">
          <Tabs
            view={view}
            busy={busy}
            manage={manage}
            onNavigate={onNavigate}
          />
        </div>
        <Banner className="absolute inset-y-0 right-0 -z-10 w-[min(440px,calc(100%-144px))] bg-size-[502px_auto] bg-position-[right_-40px_top_-148px] md:static md:z-auto md:ml-auto md:hidden md:h-full md:w-[640px] xl:block md:min-w-0 md:bg-size-[836px_auto] md:bg-position-[right_-40px_top_-269px]" />
        <div className="hidden md:ml-auto md:block xl:ml-0">
          <button
            className={cx(
              button("ghost"),
              "h-14 gap-3 px-3 text-2x font-normal",
            )}
            aria-haspopup="dialog"
            disabled={busy}
            onClick={onAccount}
          >
            <PixelIcon name="avatar" inks={{ "#": "text-mint" }} />
            Account
          </button>
        </div>
      </div>
      <div className="flex h-14 items-center justify-between border-t border-line px-2 sm:px-4 md:hidden">
        {back ? (
          <button
            className={cx(button("ghost"), "text-2x font-normal")}
            aria-label={`Back to ${back.label}`}
            disabled={busy}
            onClick={back.onBack}
          >
            <PixelIcon name="back" className="text-info" />
            {back.label}
          </button>
        ) : (
          <Tabs
            view={view}
            busy={busy}
            manage={manage}
            compact
            onNavigate={onNavigate}
          />
        )}
        <button
          className={button("ghost", "icon")}
          aria-label="Account"
          aria-haspopup="dialog"
          disabled={busy}
          onClick={onAccount}
        >
          <PixelIcon name="gear" />
        </button>
      </div>
    </header>
  );
}
