import { test, expect, beforeEach } from "bun:test";
import {
  startTargetedTalk,
  onUtterance,
  testConfirms,
  testInjects,
  __setTestModeForTest,
  __setInvokeForTest,
  __resetForTest,
  __getTargetForTest,
} from "../src/command";
import { __setTestModeForTest as micTestMode, resetTestRecords as micReset } from "../src/mic";
import type { AgentEntry } from "../src/events";

const agent: AgentEntry = {
  id: "term_x",
  name: "heartwood",
  pane: "wM:p1",
  status: "idle",
  title: "",
  cwd: "",
};

beforeEach(() => {
  __setTestModeForTest(true);
  __resetForTest();
  micReset(); // sets mic testMode so toggleTalk takes the local path
});

test("T1: 沒有 target 時 onUtterance 放行（回 false，不攔截 Q&A）", async () => {
  expect(await onUtterance([1, 2, 3])).toBe(false);
  expect(testConfirms.length).toBe(0);
});

test("T2: startTargetedTalk 記 target 並開始收音；utterance 轉寫後出現確認、target 清空", async () => {
  __setInvokeForTest(async (cmd) => {
    expect(cmd).toBe("transcribe_utterance");
    return "檢查 git status";
  });
  await startTargetedTalk(agent);
  expect(__getTargetForTest()).toEqual({ pane: "wM:p1", name: "heartwood" });
  const consumed = await onUtterance([0]);
  expect(consumed).toBe(true);
  expect(testConfirms).toEqual([{ name: "heartwood", text: "檢查 git status" }]);
  expect(__getTargetForTest()).toBeNull(); // one-shot
  micTestMode(false);
});

test("T3: 空轉寫 → 不出確認、仍算已消費", async () => {
  __setInvokeForTest(async () => "   ");
  await startTargetedTalk(agent);
  expect(await onUtterance([0])).toBe(true);
  expect(testConfirms.length).toBe(0);
  micTestMode(false);
});

test("T4: 轉寫失敗 → 吞掉錯誤、已消費、不出確認", async () => {
  __setInvokeForTest(async () => {
    throw new Error("stt: boom");
  });
  await startTargetedTalk(agent);
  expect(await onUtterance([0])).toBe(true);
  expect(testConfirms.length).toBe(0);
  expect(testInjects.length).toBe(0);
  micTestMode(false);
});
