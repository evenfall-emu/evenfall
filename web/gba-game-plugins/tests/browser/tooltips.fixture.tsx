import React from "react";
import { createRoot } from "react-dom/client";
import { InfoTip } from "../../ui/info-tip";
import "../../../player/src/console.css";
import "../../ui/dashboard.css";

createRoot(document.body.appendChild(document.createElement("div"))).render(
  <main style={{ padding: 24 }}>
    <section data-tooltip-anchor style={{ width: 280, marginTop: 120 }}>
      <InfoTip
        label="Ability: Adaptability"
        content={
          <>
            <h3>Adaptability</h3>
            <p>Powers up moves of the same type as the Pokémon.</p>
          </>
        }
      >
        Adaptability
      </InfoTip>
    </section>
    <section
      data-tooltip-anchor
      style={{ position: "fixed", right: 24, bottom: 24 }}
    >
      <InfoTip
        label="Long move details"
        content={
          <>
            <h3>Move details</h3>
            {Array.from({ length: 40 }, (_, index) => (
              <p key={index}>
                Move effect {index + 1}: an extremely fast attack with priority
                +1.
              </p>
            ))}
          </>
        }
      >
        Long move details
      </InfoTip>
    </section>
    <button style={{ marginTop: 60 }}>Outside</button>
  </main>,
);
