import { describe, expect, it } from "vite-plus/test";
import app from "../src/index";

// Stands in for Workers Static Assets: records what the Worker asked for.
function assets() {
  const requested: string[] = [];
  const fetcher = {
    fetch: async (input: RequestInfo | URL) => {
      requested.push(
        new URL(input instanceof Request ? input.url : input).pathname,
      );
      return new Response("player index");
    },
  } as unknown as Fetcher;
  return { requested, fetcher };
}

describe("player pages", () => {
  it("serve the player's index for each client-side page", async () => {
    for (const path of [
      "/library",
      "/library/elite-redux",
      "/manage",
      "/manage/elite-redux",
      "/player",
      "/share/state/Ab-_0123456789abcdefghij",
    ]) {
      const { requested, fetcher } = assets();
      const response = await app.request(
        `https://evenfall.test${path}`,
        {},
        {
          ASSETS: fetcher,
        },
      );
      expect(response.status).toBe(200);
      expect(await response.text()).toBe("player index");
      expect(requested).toEqual(["/"]);
    }
  });

  it("leave other paths as 404s", async () => {
    const { requested, fetcher } = assets();
    for (const path of [
      "/libraries",
      "/library/a/b",
      "/manage/a/b",
      "/managers",
      "/player/1",
      "/home",
      "/share",
      "/share/state",
      "/share/save/abc",
      "/share/state/a/b",
    ]) {
      const response = await app.request(
        `https://evenfall.test${path}`,
        {},
        {
          ASSETS: fetcher,
        },
      );
      expect(response.status).toBe(404);
    }
    expect(requested).toEqual([]);
  });
});
