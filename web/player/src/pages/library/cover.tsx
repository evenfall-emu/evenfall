import { useState } from "react";
import { cx } from "../../ui/styles";

const tints = [
  "bg-success-surface",
  "bg-warning-surface",
  "bg-info-surface",
  "bg-danger-surface",
];

function initials(title: string) {
  const words = title.split(/[^\p{L}\p{N}]+/u).filter(Boolean);
  return (
    words
      .slice(0, 2)
      .map((word) => Array.from(word)[0])
      .join("")
      .toLocaleUpperCase() || "?"
  );
}

function tint(id: string) {
  let hash = 0;
  for (const char of id) hash = (hash * 31 + char.charCodeAt(0)) >>> 0;
  return tints[hash % tints.length];
}

const sizes = {
  sm: "size-14 border text-3x sm:size-16",
  md: "size-16 border-2 text-3x",
  lg: "size-22 border-2 text-4x sm:size-33 sm:border-3 sm:text-5x",
};

/** Catalog artwork, or title initials over a pixel horizon when there is none. */
export function Cover({
  id,
  title,
  imageUrl,
  size,
}: {
  id: string;
  title: string;
  imageUrl: string | null;
  size: keyof typeof sizes;
}) {
  const [failed, setFailed] = useState(false);
  return (
    <span
      aria-hidden="true"
      className={cx(
        "relative grid shrink-0 place-items-center overflow-hidden rounded-control font-bold",
        size === "sm" ? "border-line" : "border-mint/70",
        tint(id),
        sizes[size],
      )}
    >
      {imageUrl && !failed ? (
        <img
          src={imageUrl}
          alt=""
          className="size-full object-cover"
          onError={() => setFailed(true)}
        />
      ) : (
        <>
          <svg
            viewBox="0 0 32 32"
            className="absolute inset-0 size-full text-canvas/60"
            shapeRendering="crispEdges"
            fill="currentColor"
          >
            <path d="M0 32v-5h2v-1h2v-2h2v-1h1v1h2v2h2v1h1v-2h2v-2h2v-1h1v1h2v2h1v2h2v-1h2v-2h1v-1h1v1h2v2h2v1h1v6Z" />
          </svg>
          <span className="relative bg-linear-to-b from-heading from-45% to-peach bg-clip-text leading-none text-transparent">
            {initials(title)}
          </span>
        </>
      )}
    </span>
  );
}
