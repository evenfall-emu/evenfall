import React, { useState } from "react";
import { createRoot } from "react-dom/client";
import { AboutPage } from "../../src/about";
import "../../src/console.css";

function Fixture() {
  const [returned, setReturned] = useState(false);
  return returned ? (
    <main className="about-page">
      <h1>Player restored</h1>
    </main>
  ) : (
    <AboutPage
      onPlay={() => setReturned(true)}
      onLibrary={() => setReturned(true)}
      onAccount={() => location.assign("/?account=1")}
      hasGame={false}
    />
  );
}

const root = document.createElement("div");
root.id = "root";
document.body.replaceChildren(root);
createRoot(root).render(<Fixture />);
