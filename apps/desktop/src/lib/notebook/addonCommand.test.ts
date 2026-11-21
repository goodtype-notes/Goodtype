import { describe, expect, it } from "vitest";
import { addonProblem } from "./addonCommand";

describe("addon command problems", () => {
  it("maps closed Rust codes and hides anything else", () => {
    expect(addonProblem("addon-needs-update")).toBe("addonCommand.problem.needsUpdate");
    expect(addonProblem("invalid notebook: addon-unavailable")).toBe("addonCommand.problem.unavailable");
    expect(addonProblem("the proposal is stale: page modified")).toBe("addonCommand.problem.stale");
    expect(addonProblem("proposal_changed")).toBe("addonCommand.problem.stale");
    expect(addonProblem("C:\Users\someone\secret.txt exploded")).toBe("addonCommand.problem.failed");
  });
});
