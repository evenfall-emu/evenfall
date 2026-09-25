import { useState, type ReactNode } from "react";
import { useBridge, useBridgeResources } from "@evenfall/gba/bridge/react";
import { DataUnavailable, LayoutPanel, PartyColumn } from "./game-layout";
import type { GameLayoutProps } from "./layout";
import type { BridgeState } from "@evenfall/gba/bridge";
import { heartNSoul } from "../src/heart-n-soul";
import type { PokemonPartyView } from "../src/model";
import type { PokemonResources } from "../src/bridges";
import {
  partyCreatures,
  PokemonDetails,
  usePokemonArtwork,
} from "./pokemon-ui";

function BridgeMessage({
  state,
  gameName,
}: {
  state: BridgeState<PokemonPartyView>;
  gameName: string;
}) {
  if (state.status === "idle")
    return (
      <DataUnavailable title={`Load ${gameName}`}>
        Open a supported ROM or ZIP. Import a save or state under Checkpoints &
        exports, then press Play.
      </DataUnavailable>
    );
  if (state.status === "loading")
    return (
      <p className="party-message">Waiting for a fresh memory snapshot…</p>
    );
  if (state.status === "unsupported")
    return (
      <div className="party-message">
        <h3>No bridge for this ROM</h3>
        <p>
          This universal player remains fully usable. A matching game profile is
          required for the contextual layout.
        </p>
        <p>Loaded ROM SHA-256</p>
        <code className="hud-hash">{state.romHash}</code>
      </div>
    );
  if (state.status === "unavailable")
    return (
      <div className="party-message">
        <h3>Party data is not ready</h3>
        <p>{state.reason}</p>
        <p>Resume play or advance a frame to try again.</p>
      </div>
    );
  if (state.status === "error")
    return (
      <div className="party-message" role="alert">
        <h3>Could not read the party</h3>
        <p>{state.message}</p>
        <p>Playback is unaffected. The bridge retries on the next frame.</p>
      </div>
    );
  return null;
}

/** A ROM connector composes the generic player child; it never owns emulation. */
export function HeartNSoulLayout({ playing, children }: GameLayoutProps) {
  const state = useBridge(heartNSoul, "gameParty");
  const resources = useBridgeResources(heartNSoul, "gameParty");
  return (
    <PokemonLayout
      gameName="Heart & Soul"
      state={state}
      resources={resources}
      playing={playing}
    >
      {children}
    </PokemonLayout>
  );
}

/** Any Pokémon bridge can supply these typed observations and ROM resources. */
export function PokemonLayout({
  gameName,
  state,
  resources,
  playing,
  children,
}: {
  gameName: string;
  state: BridgeState<PokemonPartyView>;
  resources: PokemonResources | undefined;
  playing: boolean;
  children: ReactNode;
}) {
  const artwork = usePokemonArtwork(resources);
  const sample = "sample" in state ? state.sample : undefined;
  const label =
    state.status === "ready"
      ? playing
        ? "Live"
        : "Paused"
      : {
          idle: "Not connected",
          loading: "Connecting",
          unsupported: "Unsupported",
          unavailable: "Updating",
          error: "Error",
        }[state.status];
  const party =
    state.status === "ready" ? partyCreatures(state.data, artwork) : [];
  const [selection, setSelection] = useState<{
    timeline: string;
    id: string;
  }>();
  const selected =
    party.find(
      ({ id }) =>
        sample?.timeline === selection?.timeline && id === selection?.id,
    ) ?? party[0];
  const selectedIndex = party.findIndex(({ id }) => id === selected?.id);
  const selectedPokemon =
    state.status === "ready" ? state.data.party[selectedIndex] : undefined;

  return (
    <div className="hud-layout" data-game-layout="pokemon">
      <aside
        className="party-hud game-sidebar game-sidebar-left"
        aria-label="Party"
      >
        <p className="eyebrow">{gameName.toUpperCase()} · READ ONLY</p>
        <div className="party-heading">
          <h2 id="party-heading">Party</h2>
          <span className={`bridge-status ${state.status}`} role="status">
            {label}
          </span>
        </div>
        {state.status === "ready" ? (
          <>
            <p className="party-count">{party.length} Pokémon in party</p>
            {party.length ? (
              <PartyColumn
                title="Current team"
                party={party}
                selectedId={selected?.id}
                label="Current team"
                onSelect={(creature) =>
                  setSelection({
                    timeline: state.sample.timeline,
                    id: creature.id,
                  })
                }
              />
            ) : (
              <div className="party-message">
                <h3>No Pokémon in the party</h3>
                <p>
                  Continue your save or choose a starter in-game. An empty party
                  is a valid reading, not a connection error.
                </p>
              </div>
            )}
          </>
        ) : (
          <BridgeMessage state={state} gameName={gameName} />
        )}
        {sample && (
          <details className="bridge-sample">
            <summary>Snapshot details</summary>
            <dl>
              <dt>Cycle</dt>
              <dd>{sample.cycle.toString()}</dd>
              <dt>Sample</dt>
              <dd>{sample.sequence}</dd>
              <dt>Timeline</dt>
              <dd>{sample.timeline}</dd>
              <dt>ROM SHA-256</dt>
              <dd>{sample.romHash}</dd>
            </dl>
            <p>
              Up to 20 snapshots/sec while playing. Paused data stays visible.
            </p>
          </details>
        )}
        {selected && selectedPokemon && (
          <LayoutPanel
            title="Selected Pokémon"
            meta={`Slot ${party.findIndex(({ id }) => id === selected.id) + 1}`}
          >
            <PokemonDetails
              pokemon={selectedPokemon}
              creature={selected}
              artwork={artwork}
            />
          </LayoutPanel>
        )}
      </aside>
      {children}
      <aside className="game-sidebar game-sidebar-right" aria-label="Opponent">
        <LayoutPanel title="Opponent" meta="Battle data">
          <DataUnavailable title="Opponent unavailable">
            The loaded profile does not expose an active battle party yet.
          </DataUnavailable>
        </LayoutPanel>
      </aside>
    </div>
  );
}
