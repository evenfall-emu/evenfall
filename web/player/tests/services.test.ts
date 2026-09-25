import { describe, it, expect, vi } from "vitest";
import { Session, Lab } from "../src/services";
import type { Emulator, LabReport, Scenario } from "@evenfall/gba";
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
  it("termination during initialization disposes the eventual worker", async () => {
    let ready!: (worker: Emulator) => void;
    const create = () =>
      new Promise<Emulator>((resolve) => {
        ready = resolve;
      });
    const lab = new Lab(create);
    const running = lab.run({} as Scenario, {});
    const terminated = expect(running).resolves.toMatchObject({
      status: "terminated",
    });
    lab.terminate();
    const terminate = vi.fn();
    ready({ terminate } as unknown as Emulator);
    await terminated;
    expect(terminate).toHaveBeenCalledWith("LabTerminated");
    expect(lab.state.status).toBe("Terminated");
  });
  it("cooperative cancellation returns a result and finalizes only its test worker", async () => {
    let finish!: (report: LabReport) => void;
    const worker = Object.assign(new EventTarget(), {
      runScenario: () =>
        new Promise<LabReport>((resolve) => {
          finish = resolve;
        }),
      cancel: async () => {
        finish({ status: "cancelled" });
      },
      terminate: vi.fn(),
    });
    const lab = new Lab(async () => worker as unknown as Emulator);
    const running = lab.run({} as Scenario, {});
    await vi.waitFor(() => expect(finish).toBeTypeOf("function"));
    await lab.cancel();
    expect((await running).status).toBe("cancelled");
    expect(worker.terminate).toHaveBeenCalledOnce();
    expect(lab.worker).toBeUndefined();
  });
});
