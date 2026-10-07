import { InfoTip } from "./info-tip";
import type { BuildInfo } from "./layout";

/** A small pixel warning beside a heading, only for a detected build with unconfirmed values. */
export function BuildWarning({ build }: { build?: BuildInfo }) {
  if (build?.provenance !== "located" || !build.review.length) return null;
  return (
    <span className="build-warning" data-tooltip-anchor>
      <InfoTip
        label="HUD compatibility warning"
        content={
          <p>
            This HUD was auto-detected with some unconfirmed values and may not
            work perfectly. Please report any bugs you find.
          </p>
        }
      >
        <svg
          viewBox="0 0 16 16"
          width="16"
          height="16"
          fill="currentColor"
          shapeRendering="crispEdges"
          aria-hidden="true"
          focusable="false"
        >
          <path
            fillRule="evenodd"
            d="M7 1h2v2h1v2h1v2h1v2h1v2h1v2h1v2H1v-2h1v-2h1v-2h1V7h1V5h1V3h1ZM7 5h2v5H7ZM7 12h2v2H7Z"
          />
        </svg>
      </InfoTip>
    </span>
  );
}
