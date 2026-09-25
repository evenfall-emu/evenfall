// Synthetic catalog and RAM, real host/hash check/decoder/hooks/presentation.
// Never shipped in the player. The production HnS profile is not altered.
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import {
  BridgeProvider,
  useBridge,
  useBridgeResources,
} from "@evenfall/gba/bridge/react";
import { PokemonLayout } from "../../ui/party-hud";
import { BridgeHost } from "@evenfall/gba/bridge";
import { heartNSoul } from "../../src/heart-n-soul";
import { pokemonResources } from "../fixtures/pokemon";
import type { Emulator } from "@evenfall/gba";

export function mount(emulator: Emulator, romHash: string) {
  const game: typeof heartNSoul = {
    ...heartNSoul,
    prepare: () => pokemonResources(),
    profiles: [{ ...heartNSoul.profiles[0], romHash }],
  };
  const host = new BridgeHost(emulator);
  function Layout() {
    const state = useBridge(game, "gameParty");
    const resources = useBridgeResources(game, "gameParty");
    return (
      <PokemonLayout
        gameName="Synthetic Pokémon"
        state={state}
        resources={resources}
        playing={false}
      >
        {null}
      </PokemonLayout>
    );
  }
  const element = document.body.appendChild(document.createElement("div"));
  const root = createRoot(element);
  root.render(
    <StrictMode>
      <BridgeProvider host={host}>
        <Layout />
      </BridgeProvider>
    </StrictMode>,
  );
  return () => {
    root.unmount();
    host.dispose();
    element.remove();
  };
}
