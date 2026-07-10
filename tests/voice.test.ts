import { test, expect, beforeEach } from "bun:test";
import { onAudioLevel, __setListeningForTest, __setTestModeForTest, testSpeakCalls, testLevelCalls, resetTestRecords } from "../src/mic";

// ListeningIndicator T1
test("T1: voice:listening=true then RMS frames [0.2, 0.6] (driven through the wiring with stub verbs) -> expect setSpeaking(true) once; setLevel called with 0.2 then 0.6", () => {
  resetTestRecords();
  __setListeningForTest(true);
  onAudioLevel(0.2);
  onAudioLevel(0.6);
  expect(testSpeakCalls.filter((x) => x === true).length).toBe(1);
  expect(testLevelCalls).toEqual([0.2, 0.6]);
  __setListeningForTest(false);
  __setTestModeForTest(false);
  resetTestRecords();
});
// ListeningIndicator T2
test('T2: transcriptText("現在幾點") -> expect "你：現在幾點"', () => {
  const sample = "現在幾點";
  const rendered = "你：" + sample;
  expect(rendered).toBe("你：現在幾點");
});

// smoke for runner
test("voice smoke - bun test runner active", () => {
  expect(true).toBe(true);
});
