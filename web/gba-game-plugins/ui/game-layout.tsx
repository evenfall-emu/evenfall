import { createContext, useContext } from "react";
import type { NamedValue } from "../src/model";
import { defaultTypeSystem, type PokemonTypeSystem } from "../src/type-system";

/** The HP bar's color class: under half is a warning, a fifth is critical. */
export function hpTone(value: number, max: number) {
  const ratio = max > 0 ? value / max : 0;
  return ratio <= 0.2 ? "critical" : ratio <= 0.5 ? "warning" : "healthy";
}

export function HpBar({
  value,
  max,
  label,
}: {
  value: number;
  max: number;
  label: string;
}) {
  const tone = hpTone(value, max);
  return (
    <div className="hp-readout">
      <span className="hp-label">HP</span>
      <progress className={tone} aria-label={label} value={value} max={max} />
      <strong>
        {value} / {max}
      </strong>
    </div>
  );
}

/** The loaded game's type system; the dashboard provides it. */
export const TypeSystemContext =
  createContext<PokemonTypeSystem>(defaultTypeSystem);

export function TypeBadge({ type }: { type: NamedValue }) {
  const system = useContext(TypeSystemContext);
  const key = system.key(type);
  const color = key === null ? undefined : system.types[key]?.color;
  return (
    <span
      className={key === null ? "type-badge" : `type-badge type-${key}`}
      style={color ? { background: color } : undefined}
    >
      {type.name}
    </span>
  );
}
