import { describe, it, expect, vi } from "vitest";
import { Session } from "../src/services";
import type { Emulator } from "@evenfall/gba";
describe("scoped session resources", () => {
  it("replaces and releases the save lease and closes presentation with the worker", () => {
    const terminate = vi.fn(),
      first = vi.fn(),
      second = vi.fn(),
      cleanup = vi.fn();
    const session = new Session(
      Object.assign(new EventTarget(), { terminate }) as unknown as Emulator,
    );
    session.ownSaveLock(first);
    session.ownSaveLock(second);
    session.onClose(cleanup);
    expect(first).toHaveBeenCalledOnce();
    expect(second).not.toHaveBeenCalled();
    session.close();
    expect(second).toHaveBeenCalledOnce();
    expect(cleanup).toHaveBeenCalledOnce();
    expect(terminate).toHaveBeenCalledOnce();
  });
});
