import { test, expect, beforeEach } from "bun:test";
import {
  onSummonProposal,
  confirmSummon,
  cancelSummon,
  testSummonConfirms,
  testSummonSends,
  testSummonOk,
  __setTestModeForTest,
  __setInvokeForTest,
  __resetForTest,
} from "../src/command";
import { handleUtteranceOutcome, type SummonProposal } from "../src/mic";

const proposal: SummonProposal = { project: "cyris", task: "跑測試", cwd: "/x/cyris" };

beforeEach(() => {
  __setTestModeForTest(true);
  __resetForTest(); // clears pendingSummon too — a leftover proposal would leak across tests
});

// === SummonConfirm ===

test("T1: 召喚提案先進確認框，不直接建立任何東西", () => {
  onSummonProposal({ ...proposal });
  expect(testSummonConfirms).toEqual([{ project: "cyris", task: "跑測試" }]);
  expect(testSummonSends.length).toBe(0);
});

test("T2: 編輯 task 後按 ✓ → 以編輯後內容呼叫 summon_agent", async () => {
  __setInvokeForTest(async () => null);
  onSummonProposal({ ...proposal });
  await confirmSummon("跑測試並附 coverage");
  expect(testSummonSends).toEqual([
    { cmd: "summon_agent", args: { project: "cyris", task: "跑測試並附 coverage", cwd: "/x/cyris" } },
  ]);
  expect(testSummonOk).toEqual([{ project: "cyris", task: "跑測試並附 coverage" }]);
});

test("T3: 取消（✕）→ 什麼都不送", async () => {
  __setInvokeForTest(async () => null);
  onSummonProposal({ ...proposal });
  cancelSummon();
  expect(testSummonSends.length).toBe(0);
  // 取消後再按 ✓ 也不該復活提案
  await confirmSummon("跑測試");
  expect(testSummonSends.length).toBe(0);
});

test("T4: task 清空後按 ✓ → 什麼都不送", async () => {
  __setInvokeForTest(async () => null);
  onSummonProposal({ ...proposal });
  await confirmSummon("   ");
  expect(testSummonSends.length).toBe(0);
  expect(testSummonOk.length).toBe(0);
});

test("T5: summon_agent 失敗 → 不 throw、記錄嘗試、無成功記錄", async () => {
  __setInvokeForTest(async () => {
    throw new Error("summon agent.wait: timeout");
  });
  onSummonProposal({ ...proposal });
  await confirmSummon("跑測試");
  expect(testSummonSends.length).toBe(1);
  expect(testSummonOk.length).toBe(0);
});

// === SummonOutcomeRoutingFrontend ===

function spyFn() {
  const calls: SummonProposal[] = [];
  const fn = (p: SummonProposal) => { calls.push(p); };
  return { fn, calls };
}

test("T1: kind=summon → 交給 handler，回 true", () => {
  const spy = spyFn();
  const payload = { kind: "summon", project: "cyris", task: "跑測試", cwd: "/x/cyris" };
  expect(handleUtteranceOutcome(payload, spy.fn)).toBe(true);
  expect(spy.calls.length).toBe(1);
  expect(spy.calls[0]).toEqual(payload as unknown as SummonProposal);
});

test("T2: kind=spoken → 安靜無事，回 false", () => {
  const spy = spyFn();
  expect(handleUtteranceOutcome({ kind: "spoken", text: "你好" }, spy.fn)).toBe(false);
  expect(spy.calls.length).toBe(0);
});

test("T3: undefined / null（舊回傳值）→ 皆 false", () => {
  const spy = spyFn();
  expect(handleUtteranceOutcome(undefined, spy.fn)).toBe(false);
  expect(handleUtteranceOutcome(null, spy.fn)).toBe(false);
  expect(spy.calls.length).toBe(0);
});

test("T4: summon 但欄位缺失 → false（缺欄防禦）", () => {
  const spy = spyFn();
  expect(handleUtteranceOutcome({ kind: "summon", project: "", task: "跑測試", cwd: "/x" }, spy.fn)).toBe(false);
  expect(handleUtteranceOutcome({ kind: "summon", project: "cyris", task: "", cwd: "/x" }, spy.fn)).toBe(false);
  expect(handleUtteranceOutcome({ kind: "summon", project: "cyris", task: "跑測試" }, spy.fn)).toBe(false);
  expect(spy.calls.length).toBe(0);
});
