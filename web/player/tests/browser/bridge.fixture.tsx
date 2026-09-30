// Test-only entry bundled by Vite. Never included in the player distribution.
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { BridgeProvider, useBridge } from "@evenfall/gba/bridge/react";
import { BridgeHost, defineGame, ref, u8 } from "@evenfall/gba/bridge";
import type { Emulator } from "@evenfall/gba";

export function mount(emulator: Emulator, romHash: string) {
  const host = new BridgeHost(emulator);
  const game = defineGame({
    id: "browser-fixture",
    profiles: [
      {
        romHash,
        source: {
          repository: "fixture",
          revision: "1",
          variant: "test",
          compiler: "none",
          options: [],
        },
        symbols: { value: ref(0x02000000, u8) },
      },
    ],
    bridges: {
      value: {
        id: "value",
        decode: ({ memory, symbols }) => ({
          status: "ready",
          data: memory.read(symbols.value),
        }),
      },
    },
  });
  function Probe() {
    const state = useBridge(game, "value");
    return (
      <output aria-label="Bridge probe">
        {state.status === "ready"
          ? `${state.data}:${state.sample.cycle}:${state.sample.timeline}`
          : state.status}
      </output>
    );
  }
  const element = document.body.appendChild(document.createElement("div"));
  const root = createRoot(element);
  root.render(
    <StrictMode>
      <BridgeProvider host={host}>
        <Probe />
        <Probe />
      </BridgeProvider>
    </StrictMode>,
  );
  return () => {
    root.unmount();
    host.dispose();
    element.remove();
  };
}
