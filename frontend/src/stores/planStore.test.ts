import { beforeEach, describe, expect, it } from "vitest";
import type { PlanItem } from "@flamel/shared";
import { usePlanStore } from "./planStore";

const TAB_ID = "tab-1";

function item(overrides: Partial<PlanItem> = {}): PlanItem {
  return {
    id: "item-1",
    text: "Do the thing",
    status: "todo",
    reasoningFloor: "medium",
    position: 0,
    ...overrides,
  };
}

describe("plan store live update stability", () => {
  beforeEach(() => {
    usePlanStore.setState({ plans: {} });
  });

  it("skips state publication when setPlan receives an equivalent item snapshot", () => {
    const firstItem = item();
    usePlanStore.getState().setPlan(TAB_ID, "plan-1", "Plan", undefined, [firstItem]);

    const beforeState = usePlanStore.getState();
    const beforePlan = beforeState.plans[TAB_ID];
    const beforeItem = beforePlan.items[0];

    usePlanStore.getState().setPlan(TAB_ID, "plan-1", "Plan", undefined, [item()]);

    const afterState = usePlanStore.getState();
    expect(afterState).toBe(beforeState);
    expect(afterState.plans[TAB_ID]).toBe(beforePlan);
    expect(afterState.plans[TAB_ID].items[0]).toBe(beforeItem);
  });

  it("skips state publication for equivalent item updates", () => {
    usePlanStore.getState().setPlan(TAB_ID, "plan-1", "Plan", undefined, [item({ status: "in_progress" })]);

    const beforeState = usePlanStore.getState();
    usePlanStore.getState().updateItem(TAB_ID, "item-1", { status: "in_progress" });

    expect(usePlanStore.getState()).toBe(beforeState);
  });

  it("updates only the changed item reference", () => {
    const first = item({ id: "item-1", position: 0 });
    const second = item({ id: "item-2", text: "Keep this stable", position: 1 });
    usePlanStore.getState().setPlan(TAB_ID, "plan-1", "Plan", undefined, [first, second]);

    const beforeItems = usePlanStore.getState().plans[TAB_ID].items;
    usePlanStore.getState().updateItem(TAB_ID, "item-1", { status: "done" });

    const afterItems = usePlanStore.getState().plans[TAB_ID].items;
    expect(afterItems[0]).not.toBe(beforeItems[0]);
    expect(afterItems[0].status).toBe("done");
    expect(afterItems[1]).toBe(beforeItems[1]);
  });

  it("skips state publication for equivalent streaming item ids", () => {
    usePlanStore.getState().setPlan(TAB_ID, "plan-1", "Plan", undefined, [item()]);
    usePlanStore.getState().setStreamingIds(TAB_ID, ["item-1", "item-2"]);

    const beforeState = usePlanStore.getState();
    usePlanStore.getState().setStreamingIds(TAB_ID, ["item-1", "item-2"]);

    expect(usePlanStore.getState()).toBe(beforeState);
  });
});
