import { test, expect } from "bun:test";
import { handleUtteranceOutcome, type SummonProposal } from "../src/mic";

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
