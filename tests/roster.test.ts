import { test, expect, beforeEach } from "bun:test";
import {
  setRoster,
  applyStatus,
  nameForPane,
  noteActivity,
  attribute,
  arcPositions,
  magnifyScale,
  ATTRIBUTION_WINDOW_MS,
  testStripRenders,
  __setTestModeForTest,
  __resetForTest,
} from "../src/roster";
import type { AgentEntry, Activity } from "../src/events";

const entry = (id: string, name: string, pane: string, status: string): AgentEntry => ({
  id,
  name,
  pane,
  status,
});

const activity = (pane: string, session = "sess-1234-abcd"): Activity => ({
  source: "cchooks",
  session,
  pane,
  kind: "stop",
  ts: "2026-07-23T10:00:00Z",
});

beforeEach(() => {
  __setTestModeForTest(true);
  __resetForTest();
});

test("T1: setRoster 依名字排序並 render strip", () => {
  setRoster([entry("b", "zeta", "p2", "idle"), entry("a", "alpha", "p1", "working")]);
  expect(testStripRenders.length).toBe(1);
  expect(testStripRenders[0].map((a) => a.name)).toEqual(["alpha", "zeta"]);
});

test("T2: applyStatus 更新已知 id 並 re-render；未知 id / 相同狀態不動", () => {
  setRoster([entry("a", "alpha", "p1", "working")]);
  applyStatus({ id: "a", status: "idle" });
  expect(testStripRenders.length).toBe(2);
  expect(testStripRenders[1][0].status).toBe("idle");
  applyStatus({ id: "ghost", status: "done" }); // unknown id → no-op
  applyStatus({ id: "a", status: "idle" }); // same status → no-op
  expect(testStripRenders.length).toBe(2);
});

test("T3: nameForPane 命中/未中/空字串", () => {
  setRoster([entry("a", "alpha", "wT:p1", "working")]);
  expect(nameForPane("wT:p1")).toBe("alpha");
  expect(nameForPane("wX:p9")).toBeNull();
  expect(nameForPane("")).toBeNull();
});

test("T4: activity 在 30s 窗內 → report 冠名；窗外不冠", () => {
  setRoster([entry("a", "alpha", "wT:p1", "working")]);
  noteActivity(activity("wT:p1"), 1000);
  expect(attribute("測試完成", 1000 + ATTRIBUTION_WINDOW_MS)).toBe("alpha：測試完成");
  expect(attribute("測試完成", 1001 + ATTRIBUTION_WINDOW_MS)).toBe("測試完成");
});

test("T5: 未知 pane 退回 session 前 8 碼", () => {
  setRoster([entry("a", "alpha", "wT:p1", "working")]);
  noteActivity(activity("wX:p9", "deadbeef-cafe"), 1000);
  expect(attribute("回報", 2000)).toBe("deadbeef：回報");
});

test("T6: 沒有 activity → 原文不動；pane 與 session 都空 → 不記", () => {
  expect(attribute("素文", 999)).toBe("素文");
  noteActivity(activity("", ""), 1000);
  expect(attribute("素文", 1001)).toBe("素文");
});

test("T7: arcPositions n=1 正上方；n=4 左右對稱、全在圓心上方", () => {
  const c = { x: 75, y: 75 };
  const one = arcPositions(1, c, 66, 24);
  expect(one).toEqual([{ x: 75, y: 9 }]); // -90°: straight up
  const four = arcPositions(4, c, 66, 24);
  expect(four.length).toBe(4);
  // symmetric about x=75, ascending x, all above center
  expect(four[0].x + four[3].x).toBeCloseTo(150, 1);
  expect(four[1].x + four[2].x).toBeCloseTo(150, 1);
  expect(four[0].x).toBeLessThan(four[1].x);
  for (const p of four) expect(p.y).toBeLessThan(75);
  expect(arcPositions(0, c)).toEqual([]);
});

test("T8: magnifyScale 貼齊游標最大、影響圈外為 1、單調遞減", () => {
  expect(magnifyScale(0)).toBeCloseTo(1.9, 5); // on the cursor: max
  expect(magnifyScale(48)).toBe(1); // at influence edge
  expect(magnifyScale(100)).toBe(1); // far away
  expect(magnifyScale(12)).toBeGreaterThan(magnifyScale(24));
  expect(magnifyScale(24)).toBeGreaterThan(magnifyScale(40));
});
