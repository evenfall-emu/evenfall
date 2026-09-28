import { describe, expect, it } from "vitest";
import worker from "../src/patches";

const key = `${"a".repeat(40)}.ups`;
const url = (path = `/${key}`) => `https://patches.evenfall-emu.org${path}`;
const bytes = new Uint8Array([0x55, 0x50, 0x53, 0x31, 0, 1]);

function bucket() {
  const reads: string[] = [];
  const patch = {
    size: bytes.length,
    body: new Blob([bytes]).stream(),
  };
  return {
    reads,
    binding: {
      async get(name: string) {
        reads.push(`GET ${name}`);
        return name === key ? patch : null;
      },
      async head(name: string) {
        reads.push(`HEAD ${name}`);
        return name === key ? { size: bytes.length } : null;
      },
    } as unknown as R2Bucket,
  };
}

describe("patch CDN Worker", () => {
  it("streams a content-addressed object and allows the player's origin", async () => {
    const { binding, reads } = bucket();
    const response = await worker.fetch(
      new Request(url(), {
        headers: { Origin: "https://play.evenfall-emu.org" },
      }),
      { PATCHES: binding },
    );
    expect(response.status).toBe(200);
    expect(new Uint8Array(await response.arrayBuffer())).toEqual(bytes);
    expect(response.headers.get("Content-Length")).toBe(String(bytes.length));
    expect(response.headers.get("Content-Type")).toBe(
      "application/octet-stream",
    );
    expect(response.headers.get("Cache-Control")).toContain("immutable");
    expect(response.headers.get("Access-Control-Allow-Origin")).toBe(
      "https://play.evenfall-emu.org",
    );
    expect(reads).toEqual([`GET ${key}`]);
  });

  it("uses head without downloading bytes and does not grant CORS to other origins", async () => {
    const { binding, reads } = bucket();
    const response = await worker.fetch(
      new Request(url(), {
        method: "HEAD",
        headers: { Origin: "https://attacker.example" },
      }),
      { PATCHES: binding },
    );
    expect(response.status).toBe(200);
    expect(response.body).toBeNull();
    expect(response.headers.get("Content-Length")).toBe(String(bytes.length));
    expect(response.headers.has("Access-Control-Allow-Origin")).toBe(false);
    expect(reads).toEqual([`HEAD ${key}`]);
  });

  it("rejects unknown paths and writes without touching the bucket", async () => {
    const { binding, reads } = bucket();
    for (const path of ["/", "/other.ups", `/nested/${key}`, `/${key}/more`]) {
      const response = await worker.fetch(new Request(url(path)), {
        PATCHES: binding,
      });
      expect(response.status, path).toBe(404);
    }
    const write = await worker.fetch(new Request(url(), { method: "PUT" }), {
      PATCHES: binding,
    });
    expect(write.status).toBe(405);
    expect(write.headers.get("Allow")).toBe("GET, HEAD");
    expect(reads).toEqual([]);
    expect((await worker.fetch(new Request(url()), {})).status).toBe(503);
  });
});
