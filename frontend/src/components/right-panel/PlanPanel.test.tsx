import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { usePlanStore } from "../../stores/planStore";
import PlanPanel from "./PlanPanel";
vi.mock("../../stores/planStore", async (original) => {
  const actual = await original<typeof import("../../stores/planStore")>();
  const store = actual.usePlanStore;
  return { ...actual, usePlanStore: Object.assign((select: (s: ReturnType<typeof store.getState>) => unknown) => select(store.getState()), store) };
});
describe("plan execution controls", () => {
  beforeEach(() => usePlanStore.setState({ plans: {} }));
  for (const mobile of [false, true]) {
    for (const executionStatus of ["ready", "paused", "blocked", "running", "completed"] as const) {
      it(`shows the usable action for ${executionStatus} on ${mobile ? "mobile" : "desktop"}`, () => {
        usePlanStore.getState().setPlan("tab", "plan-id", "Website", undefined, [{id:"item",text:"Implement",status:"todo",reasoningFloor:"high",position:0}], false, { executionStatus, approvalStatus:"approved", finished:executionStatus==="completed" });
        const html=renderToStaticMarkup(<PlanPanel tabId="tab" mobile={mobile} />);
        if (executionStatus==="ready") expect(html).toContain("Start plan");
        else if (executionStatus==="paused" || executionStatus==="blocked") expect(html).toContain("Resume plan");
        else {expect(html).not.toContain("Start plan");expect(html).not.toContain("Resume plan");}
      });
    }
  }
});
