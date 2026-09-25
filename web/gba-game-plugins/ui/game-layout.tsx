import type { ReactNode } from "react";
import { Radio } from "@base-ui/react/radio";
import { RadioGroup } from "@base-ui/react/radio-group";

export type GameCreature = {
  /** Stable across party reordering; never use the current visual slot. */
  id: string;
  name: string;
  level?: number;
  hp?: number;
  maxHp?: number;
  artwork?: string;
  subtitle?: string;
  tags?: readonly string[];
  status?: string;
};

export type GameMove = {
  name: string;
  type?: string;
  category?: "physical" | "special" | "status";
  current?: number;
  maximum?: number;
};

export function LayoutPanel({
  title,
  meta,
  children,
  className = "",
  label,
}: {
  title: string;
  meta?: ReactNode;
  children: ReactNode;
  className?: string;
  label?: string;
}) {
  return (
    <section
      className={`layout-panel ${className}`}
      aria-label={label ?? title}
    >
      <header className="panel-heading">
        <h2>{title}</h2>
        {meta && <span>{meta}</span>}
      </header>
      {children}
    </section>
  );
}

export function DataUnavailable({
  title,
  children,
}: {
  title: string;
  children: ReactNode;
}) {
  return (
    <div className="data-unavailable">
      <span aria-hidden="true">◇</span>
      <h3>{title}</h3>
      <p>{children}</p>
    </div>
  );
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
  const ratio = max > 0 ? value / max : 0;
  const tone = ratio <= 0.2 ? "critical" : ratio <= 0.5 ? "warning" : "healthy";
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

const typeAliases: Readonly<Record<string, string>> = {
  electr: "electric",
  fight: "fighting",
  psychc: "psychic",
};

export function TypeBadge({ type }: { type: string }) {
  const key = type.toLowerCase().replace(/[^a-z]/g, "");
  return (
    <span className={`type-badge type-${typeAliases[key] ?? key}`}>{type}</span>
  );
}

export function CreatureCell({
  creature,
  selected = false,
}: {
  creature: GameCreature;
  selected?: boolean;
}) {
  return (
    <article className={`creature-cell ${selected ? "selected" : ""}`}>
      <div className="creature-avatar" aria-hidden="true">
        {creature.artwork ? (
          <img src={creature.artwork} alt="" />
        ) : (
          <span>◆</span>
        )}
      </div>
      <div className="creature-main">
        <div className="creature-name">
          <strong>{creature.name}</strong>
          {creature.level !== undefined && <span>Level {creature.level}</span>}
        </div>
        {creature.status && (
          <span className="creature-status">{creature.status}</span>
        )}
        {creature.subtitle && <small>{creature.subtitle}</small>}
        {creature.hp !== undefined && creature.maxHp !== undefined && (
          <HpBar
            value={creature.hp}
            max={creature.maxHp}
            label={`HP for ${creature.name}`}
          />
        )}
      </div>
    </article>
  );
}

export function PartyColumn({
  title,
  meta,
  party,
  selectedId,
  empty,
  label = "Current team",
  onSelect,
}: {
  title: string;
  meta?: ReactNode;
  party: readonly GameCreature[];
  selectedId?: GameCreature["id"];
  empty?: ReactNode;
  label?: string;
  onSelect?: (creature: GameCreature) => void;
}) {
  const items = party.map((creature) => (
    <li key={creature.id} role={onSelect ? "presentation" : undefined}>
      {onSelect ? (
        <Radio.Root
          render={<button />}
          nativeButton
          className="creature-select"
          value={creature.id}
        >
          <CreatureCell
            creature={creature}
            selected={creature.id === selectedId}
          />
        </Radio.Root>
      ) : (
        <CreatureCell
          creature={creature}
          selected={creature.id === selectedId}
        />
      )}
    </li>
  ));
  return (
    <LayoutPanel title={title} meta={meta} className="party-column">
      {party.length ? (
        onSelect ? (
          <RadioGroup
            render={<ol />}
            className="creature-grid"
            aria-label={label}
            value={selectedId ?? null}
            onValueChange={(id) => {
              const creature = party.find((entry) => entry.id === id);
              if (creature) onSelect(creature);
            }}
          >
            {items}
          </RadioGroup>
        ) : (
          <ol className="creature-grid" aria-label={label}>
            {items}
          </ol>
        )
      ) : (
        empty
      )}
    </LayoutPanel>
  );
}

export function MoveList({ moves }: { moves: readonly GameMove[] }) {
  return (
    <div className="move-list" aria-label="Moves">
      {moves.map((move) => (
        <div className="move-row" key={move.name}>
          <TypeBadge type={move.type ?? "Unknown"} />
          <strong>{move.name}</strong>
          {move.category && (
            <span className={`move-category ${move.category}`}>
              {move.category}
            </span>
          )}
          {move.current !== undefined && move.maximum !== undefined && (
            <span className="move-pp">
              {move.current} / {move.maximum}
            </span>
          )}
        </div>
      ))}
    </div>
  );
}

export function StatBars({
  stats,
}: {
  stats: readonly { label: string; value: number; maximum?: number }[];
}) {
  return (
    <dl className="stat-bars">
      {stats.map(({ label, value, maximum = 255 }) => (
        <div key={label}>
          <dt>{label}</dt>
          <dd>
            <i
              style={{ width: `${Math.min(100, (value / maximum) * 100)}%` }}
            />
          </dd>
          <dd className="stat-value">{value}</dd>
        </div>
      ))}
    </dl>
  );
}

export function CreatureDetails({
  creature,
  stats,
  moves,
}: {
  creature?: GameCreature;
  stats?: readonly { label: string; value: number; maximum?: number }[];
  moves?: readonly GameMove[];
}) {
  if (!creature)
    return (
      <DataUnavailable title="No selection">
        Select a party member when game data becomes available.
      </DataUnavailable>
    );
  return (
    <div className="creature-details">
      <CreatureCell creature={creature} selected />
      {creature.tags?.length ? (
        <div className="type-row">
          {creature.tags.map((tag) => (
            <TypeBadge type={tag} key={tag} />
          ))}
        </div>
      ) : null}
      {stats?.length ? (
        <>
          <h3>Stats</h3>
          <StatBars stats={stats} />
        </>
      ) : (
        <DataUnavailable title="Stats unavailable">
          This game profile does not expose decoded stats yet.
        </DataUnavailable>
      )}
      {moves?.length ? (
        <>
          <h3>Moves</h3>
          <MoveList moves={moves} />
        </>
      ) : (
        <DataUnavailable title="Moves unavailable">
          Move names, types, PP and categories require a matching game profile.
        </DataUnavailable>
      )}
    </div>
  );
}
