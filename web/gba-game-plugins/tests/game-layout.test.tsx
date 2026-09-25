import { renderToStaticMarkup } from "react-dom/server";
import { expect, it } from "vitest";
import { PartyColumn } from "../ui/game-layout";

const party = [
  { id: "alpha", name: "Alpha" },
  { id: "beta", name: "Beta" },
];

it("renders selectable parties as named radio groups with the controlled selection", () => {
  const html = renderToStaticMarkup(
    <PartyColumn
      title="Party"
      party={party}
      selectedId="beta"
      onSelect={() => {}}
    />,
  );
  expect(html).toContain('role="radiogroup"');
  expect(html).toContain('aria-label="Current team"');
  expect(html.match(/role="radio"/g)).toHaveLength(2);
  expect(html.match(/aria-checked="true"/g)).toHaveLength(1);
  expect(html).toContain('value="beta"');
  expect(html).not.toContain("aria-pressed");
});

it("keeps nonselectable parties as lists and empty content outside a radio group", () => {
  const html = renderToStaticMarkup(
    <PartyColumn title="Party" party={party} />,
  );
  expect(html).toContain(
    '<ol class="creature-grid" aria-label="Current team">',
  );
  expect(html).not.toContain('role="radio');
  expect(html).not.toContain("<button");
  const empty = renderToStaticMarkup(
    <PartyColumn
      title="Party"
      party={[]}
      empty="No party"
      onSelect={() => {}}
    />,
  );
  expect(empty).toContain("No party");
  expect(empty).not.toContain('role="radiogroup"');
});
