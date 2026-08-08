import { test, expect, beforeEach } from "bun:test";
import {
  acquireLarge,
  releaseLarge,
  currentCenter,
  __isLargeForTest,
  __refsForTest,
  __resetFrameForTest,
} from "../src/window-frame";

// No Tauri runtime under bun → applySize catches and just flips the flag,
// which is exactly the ref-counting + center behavior we want to pin.
beforeEach(() => __resetFrameForTest());

test("idle center is the small-window center 88/88", () => {
  expect(currentCenter()).toEqual({ x: 88, y: 88 });
  expect(__isLargeForTest()).toBe(false);
});

test("acquireLarge grows to large center 160/296, ref 1", async () => {
  await acquireLarge();
  expect(__isLargeForTest()).toBe(true);
  expect(__refsForTest()).toBe(1);
  expect(currentCenter()).toEqual({ x: 160, y: 296 });
});

test("nested acquire holds large until refs return to 0", async () => {
  await acquireLarge();
  await acquireLarge();
  expect(__refsForTest()).toBe(2);
  await releaseLarge();
  expect(__isLargeForTest()).toBe(true); // still held by the second acquire
  await releaseLarge();
  expect(__refsForTest()).toBe(0);
  expect(__isLargeForTest()).toBe(false);
  expect(currentCenter()).toEqual({ x: 88, y: 88 });
});

test("release below zero clamps at 0 and stays small", async () => {
  await releaseLarge();
  expect(__refsForTest()).toBe(0);
  expect(__isLargeForTest()).toBe(false);
});

test("onFrameChange fires after the flag flips (grow and shrink)", async () => {
  const { onFrameChange, currentCenter: cc } = await import("../src/window-frame");
  const seen: number[] = [];
  onFrameChange(() => seen.push(cc().x)); // record the center AT notify time
  await acquireLarge();
  await releaseLarge();
  expect(seen).toEqual([160, 88]); // large center, then small — never stale
});
