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
