// The Golden Sun dashboard: the party on the left, the enemies on the right,
// the coins above the screen and the estimated turn order below it.
import type { ReactNode } from "react";
import type { BridgeState } from "@evenfall/gba/bridge";
import { useBridge } from "@evenfall/gba/bridge/react";
import {
  elementalAffinity,
  estimatedTurnOrder,
  goldenSun,
  goldenSunElements,
  standbyDjinn,
  type GoldenSunBattleView,
  type GoldenSunElement,
  type GoldenSunPartyView,
  type GoldenSunStatus,
  type GoldenSunStatusKind,
  type GoldenSunUnit,
} from "../src/golden-sun";
import { BuildWarning } from "./build-warning";
import { hpTone } from "./game-layout";
import type { BuildInfo, GameLayoutProps } from "./layout";

const elementNames: Record<GoldenSunElement, string> = {
  venus: "Venus",
  mercury: "Mercury",
  mars: "Mars",
  jupiter: "Jupiter",
};

const statusLabels: Record<
  GoldenSunStatusKind,
  { readonly icon: string; readonly name: string; readonly tone: string }
> = {
  poison: { icon: "☣", name: "Poison", tone: "toxin" },
  venom: { icon: "☣", name: "Venom", tone: "doom" },
  attack: { icon: "ATK", name: "Attack", tone: "stat" },
  defense: { icon: "DEF", name: "Defense", tone: "stat" },
  resist: { icon: "RES", name: "Resist", tone: "stat" },
  delusion: { icon: "◎", name: "Delusion", tone: "mind" },
  stun: { icon: "ϟ", name: "Stun", tone: "mind" },
  sleep: { icon: "zz", name: "Sleep", tone: "mind" },
  seal: { icon: "⊘", name: "Psynergy seal", tone: "mind" },
  regen: { icon: "✚", name: "Regenerate", tone: "good" },
  reflect: { icon: "◇", name: "Reflect", tone: "good" },
  haunt: { icon: "☽", name: "Haunt", tone: "toxin" },
  doom: { icon: "⌛", name: "Countdown to downed", tone: "doom" },
  curse: { icon: "✝", name: "Curse", tone: "doom" },
};

function Gem({ element }: { element: GoldenSunElement }) {
  return (
    <span
      className={`gs-gem gs-${element}`}
      role="img"
      aria-label={elementNames[element]}
      title={elementNames[element]}
    />
  );
}

function Bar({
  kind,
  value,
  max,
}: {
  kind: "HP" | "PP";
  value: number;
  max: number;
}) {
  return (
    <div className="gs-bar">
      <span>{kind}</span>
      <progress
        className={kind === "HP" ? hpTone(value, max) : "pp"}
        aria-label={kind}
        value={value}
        max={max}
      />
      <strong>
        {value} / {max}
      </strong>
    </div>
  );
}

function Stats({ unit }: { unit: GoldenSunUnit }) {
  return (
    <dl className="gs-stats">
      {(
        [
          ["Atk", unit.attack],
          ["Def", unit.defense],
          ["Agi", unit.agility],
          ["Lck", unit.luck],
        ] as const
      ).map(([label, value]) => (
        <div key={label}>
          <dt>{label}</dt>
          <dd>{value}</dd>
        </div>
      ))}
    </dl>
  );
}

function statusText(status: GoldenSunStatus) {
  const direction =
    status.stage === undefined ? "" : status.stage > 0 ? "▲" : "▼";
  return { direction, turns: status.turns ?? "" };
}

/** One row of small pills: icon, optional direction and the turns left. */
function Statuses({ statuses }: { statuses: readonly GoldenSunStatus[] }) {
  if (!statuses.length) return null;
  return (
    <ul className="gs-statuses" aria-label="Statuses">
      {statuses.map((status) => {
        const label = statusLabels[status.kind];
        const { direction, turns } = statusText(status);
        const title = [
          status.stage === undefined
            ? label.name
            : `${label.name} ${status.stage > 0 ? "up" : "down"} ${Math.abs(status.stage)}`,
          status.turns === null ? "" : `${status.turns} turns left`,
        ]
          .filter(Boolean)
          .join(", ");
        return (
          <li
            key={status.kind}
            className={`gs-status gs-${label.tone}`}
            title={title}
            aria-label={title}
          >
            <span aria-hidden="true">{label.icon}</span>
            {direction && <span aria-hidden="true">{direction}</span>}
            {turns !== "" && <span aria-hidden="true">{turns}</span>}
          </li>
        );
      })}
    </ul>
  );
}

/** Power/resist per element in one row; enemy cells are tinted by affinity. */
function Elements({
  unit,
  affinity,
}: {
  unit: GoldenSunUnit;
  affinity?: readonly string[];
}) {
  return (
    <table className="gs-elements">
      <thead>
        <tr>
          <th scope="col">
            <span className="gs-visually-hidden">Element</span>
          </th>
          {goldenSunElements.map((element) => (
            <th key={element} scope="col">
              <Gem element={element} />
            </th>
          ))}
        </tr>
      </thead>
      <tbody>
        <tr>
          <th scope="row">Pwr/Rst</th>
          {unit.elements.map((element, i) => (
            <td
              key={goldenSunElements[i]}
              className={affinity ? `gs-${affinity[i]}` : undefined}
              title={
                affinity && affinity[i] !== "neutral"
                  ? `${affinity[i] === "weak" ? "Weak to" : "Resists"} ${elementNames[goldenSunElements[i]]}`
                  : undefined
              }
            >
              {`${element.power}/${element.resist}`}
            </td>
          ))}
        </tr>
      </tbody>
    </table>
  );
}

function DjinnPips({ unit }: { unit: GoldenSunUnit }) {
  const pips = unit.djinn.flatMap(({ owned, set }, i) =>
    Array.from({ length: owned }, (_, n) => ({
      element: goldenSunElements[i],
      set: n < set,
    })),
  );
  if (!pips.length) return null;
  const summary = unit.djinn
    .map(({ owned, set }, i) =>
      owned
        ? `${elementNames[goldenSunElements[i]]} ${set}/${owned} Set`
        : null,
    )
    .filter(Boolean)
    .join(", ");
  return (
    <span
      className="gs-djinn"
      role="img"
      aria-label={`Djinn: ${summary}`}
      title={summary}
    >
      {pips.map((pip, n) => (
        <span
          key={n}
          className={`gs-pip gs-${pip.element}${pip.set ? "" : " gs-standby"}`}
        />
      ))}
    </span>
  );
}

function UnitCard({
  unit,
  element,
  affinity,
}: {
  unit: GoldenSunUnit;
  element?: GoldenSunElement;
  affinity?: readonly string[];
}) {
  return (
    <article
      className={`gs-unit${element ? ` gs-edge-${element}` : " gs-edge-enemy"}`}
      data-downed={unit.hp === 0 || undefined}
    >
      <header>
        <h3>{unit.name}</h3>
        <span>Lv{unit.level}</span>
        <DjinnPips unit={unit} />
      </header>
      <Bar kind="HP" value={unit.hp} max={unit.maxHp} />
      <Bar kind="PP" value={unit.pp} max={unit.maxPp} />
      <Stats unit={unit} />
      <Statuses statuses={unit.statuses} />
      <Elements unit={unit} affinity={affinity} />
    </article>
  );
}

function Message({ children }: { children: ReactNode }) {
  return <p className="gs-message">{children}</p>;
}

function stateMessage(state: BridgeState<unknown>) {
  return state.status === "unavailable"
    ? state.reason
    : state.status === "error"
      ? state.message
      : state.status === "loading"
        ? "Reading the game…"
        : "Waiting for the game";
}

export function PartyColumn({
  party,
}: {
  party: BridgeState<GoldenSunPartyView>;
}) {
  const members = party.status === "ready" ? party.data.members : [];
  const standby = standbyDjinn(members);
  return (
    <section className="gs-column" aria-label="Your party">
      <h2 className="strip roster-heading">Your party</h2>
      {party.status !== "ready" ? (
        <Message>{stateMessage(party)}</Message>
      ) : (
        <>
          {members.map((member) => (
            <UnitCard
              key={member.slot}
              unit={member}
              element={member.element}
            />
          ))}
          <div
            className="gs-summons"
            aria-label="Djinn on Standby, for summons"
          >
            <h3>Summons ready</h3>
            {goldenSunElements.map((element, i) => (
              <span
                key={element}
                title={`${standby[i]} ${elementNames[element]} djinn on Standby`}
              >
                <Gem element={element} />
                {standby[i]}
              </span>
            ))}
          </div>
        </>
      )}
    </section>
  );
}

export function EnemyColumn({
  battle,
}: {
  battle: BridgeState<GoldenSunBattleView>;
}) {
  return (
    <section className="gs-column" aria-label="Enemies">
      <h2 className="strip roster-heading">Enemies</h2>
      {battle.status !== "ready" ? (
        <Message>{stateMessage(battle)}</Message>
      ) : !battle.data.active ? (
        <Message>Not in battle</Message>
      ) : (
        battle.data.enemies.map((enemy) => (
          <UnitCard
            key={enemy.slot}
            unit={enemy}
            affinity={elementalAffinity(enemy)}
          />
        ))
      )}
    </section>
  );
}

export function BattleHeading({
  party,
  battle,
  build,
}: {
  party: BridgeState<GoldenSunPartyView>;
  battle: BridgeState<GoldenSunBattleView>;
  build?: BuildInfo;
}) {
  const inBattle = battle.status === "ready" && battle.data.active;
  return (
    <header className="scene-heading gs-heading">
      <div className="arena-title">
        <h1>{inBattle ? "Battle" : "Golden Sun"}</h1>
        <BuildWarning build={build} />
      </div>
      {party.status === "ready" && (
        <p className="gs-coins">
          Coins <strong>{party.data.coins.toLocaleString("en-US")}</strong>
        </p>
      )}
    </header>
  );
}

/** Agility order of the living units; the game adds a random factor each round. */
export function TurnOrder({
  party,
  battle,
}: {
  party: BridgeState<GoldenSunPartyView>;
  battle: BridgeState<GoldenSunBattleView>;
}) {
  if (
    party.status !== "ready" ||
    battle.status !== "ready" ||
    !battle.data.active
  )
    return null;
  const order = estimatedTurnOrder(party.data.members, battle.data.enemies);
  return (
    <section className="gs-turns" aria-label="Estimated turn order">
      <h2>Turn order (est.)</h2>
      <ol>
        {order.map(({ unit, side }) => {
          const skips = unit.statuses.some(
            (s) => s.kind === "sleep" || s.kind === "stun",
          );
          return (
            <li
              key={`${side}:${unit.slot}`}
              className={`gs-turn gs-${side}`}
              data-skips={skips || undefined}
              title={skips ? `${unit.name} cannot act` : undefined}
            >
              <b>
                {unit.name} <span>{unit.agility}</span>
              </b>
            </li>
          );
        })}
      </ol>
    </section>
  );
}

/** The dashboard from two view states; the player passes itself as `children`. */
export function GoldenSunLayout({
  party,
  battle,
  build,
  children,
  handheld,
  docked,
}: Omit<GameLayoutProps, "playing"> & {
  party: BridgeState<GoldenSunPartyView>;
  battle: BridgeState<GoldenSunBattleView>;
}) {
  const inBattle = battle.status === "ready" && battle.data.active;
  const heading = <BattleHeading party={party} battle={battle} build={build} />;
  const partyColumn = <PartyColumn party={party} />;
  const enemyColumn = (
    <>
      <EnemyColumn battle={battle} />
      <TurnOrder party={party} battle={battle} />
    </>
  );
  const panels = [
    { id: "party", label: "Party", content: partyColumn },
    { id: "battle", label: "Battle", content: enemyColumn },
  ];
  if (handheld)
    return handheld({
      summary: <HandheldSummary party={party} battle={battle} />,
      panels,
    });
  if (docked)
    return docked({ heading, panels, focus: inBattle ? "battle" : "party" });
  return (
    <main className="adventure-grid gs-dashboard" data-game-layout="golden-sun">
      <div className="dashboard-team">{partyColumn}</div>
      <div className="adventure-center">
        {heading}
        {children}
        <TurnOrder party={party} battle={battle} />
      </div>
      <div className="dashboard-info">
        <EnemyColumn battle={battle} />
      </div>
    </main>
  );
}

function HandheldSummary({
  party,
  battle,
}: {
  party: BridgeState<GoldenSunPartyView>;
  battle: BridgeState<GoldenSunBattleView>;
}) {
  if (party.status !== "ready") return <span>{stateMessage(party)}</span>;
  const enemies =
    battle.status === "ready" && battle.data.active ? battle.data.enemies : [];
  return (
    <span className="gs-handheld">
      {party.data.members.map((m) => (
        <span key={m.slot}>
          {m.name} {m.hp}
        </span>
      ))}
      {enemies.length > 0 && (
        <span>vs {enemies.filter((e) => e.hp > 0).length}</span>
      )}
    </span>
  );
}

export function GoldenSunDashboard({
  build,
  children,
  handheld,
  docked,
}: GameLayoutProps) {
  const party = useBridge(goldenSun, "party");
  const battle = useBridge(goldenSun, "battle");
  return (
    <GoldenSunLayout
      party={party}
      battle={battle}
      build={build}
      handheld={handheld}
      docked={docked}
    >
      {children}
    </GoldenSunLayout>
  );
}
