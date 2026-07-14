import { test, expect, beforeEach } from "bun:test";
import { orbGesture, __resetOrbGestureForTest } from "../src/orb-gesture";

beforeEach(() => __resetOrbGestureForTest());

// The bug @claude found: a double-click that dismisses an open menu must not
// also start talk. First mousedown sees menu open (detail=1); the dismiss flips
// menuOpen false, so the second mousedown (detail=2) sees it closed.
test("double-click while menu open -> dismiss only, never talk", () => {
  expect(orbGesture(1, true)).toBe("none"); // 1st press: menu open -> dismiss
  expect(orbGesture(2, false)).toBe("none"); // 2nd press: still suppressed
});

test("double-click with menu closed -> drag then talk", () => {
  expect(orbGesture(1, false)).toBe("drag");
  expect(orbGesture(2, false)).toBe("talk");
});

test("single press with menu closed -> drag", () => {
  expect(orbGesture(1, false)).toBe("drag");
});

test("single press dismissing the menu -> none", () => {
  expect(orbGesture(1, true)).toBe("none");
});

test("fresh sequence re-reads menu state (no stale latch)", () => {
  orbGesture(1, true); // dismiss sequence
  expect(orbGesture(1, false)).toBe("drag"); // next sequence, menu now closed
});
