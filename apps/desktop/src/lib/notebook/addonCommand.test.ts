import { describe, expect, it } from "vitest";
import { addonProblem, panelCleanup } from "./addonCommand";

describe("addon command problems", () => {
  it("maps closed Rust codes and hides anything else", () => {
    expect(addonProblem("addon-needs-update")).toBe("addonCommand.problem.needsUpdate");
    expect(addonProblem("invalid notebook: addon-unavailable")).toBe("addonCommand.problem.unavailable");
    expect(addonProblem("addon-sandbox-failed")).toBe("addonCommand.problem.sandboxFailed");
    expect(addonProblem("the proposal is stale: page modified")).toBe("addonCommand.problem.stale");
    expect(addonProblem("proposal_changed")).toBe("addonCommand.problem.stale");
    expect(addonProblem("C:\Users\someone\secret.txt exploded")).toBe("addonCommand.problem.failed");
  });
});

describe("addon command panel cleanup", () => {
  it("cancels a running command and dismisses every unsettled proposal", () => {
    expect(panelCleanup("running", null, false)).toEqual({ cancel: true, dismiss: null });
    // Rerun ({#key}), page switch or close while reviewing must free the pending slot.
    expect(panelCleanup("review", "p1", false)).toEqual({ cancel: false, dismiss: "p1" });
    expect(panelCleanup("failed", "p1", false)).toEqual({ cancel: false, dismiss: "p1" });
    // An inserted or directly applied proposal is already consumed by Rust.
    expect(panelCleanup("review", "p1", true)).toEqual({ cancel: false, dismiss: null });
    expect(panelCleanup("applied", "p1", false)).toEqual({ cancel: false, dismiss: null });
  });
  it("maps the flush and storage codes", () => {
    expect(addonProblem("flush_failed")).toBe("addonCommand.problem.flushFailed");
    expect(addonProblem("addon-storage-exceeded")).toBe("addonCommand.problem.storageExceeded");
  });
});
