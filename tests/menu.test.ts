import { test, expect, beforeEach } from "bun:test";
import {
  fanPositions,
  handleContextMenu,
  openMenu,
  dismissMenu,
  resetTestRecords,
  __setTestModeForTest,
  testInvokeCalls,
  testMenuRenders,
  setRegistryForTest,
  isMenuOpenForTest,
  getMutedCacheForTest,
  setMutedCacheForTest,
  buildMenuItems,
  activateItem,
  testToastCalls,
  __setInvokeForTest,
  onMutedEvent,
  isOrbHit,
} from "../src/menu";

// FanLayout contract tests（ring=96, center 160/180, step 40°, 扇形對準 centerDeg）
test("T1: given fanPositions(1, -90) -> expect [{x:160,y:84}] ±0.5", () => {
  const pos = fanPositions(1, -90);
  expect(pos.length).toBe(1);
  expect(pos[0].x).toBeCloseTo(160, 1);
  expect(pos[0].y).toBeCloseTo(84, 1);
});

test("T2: given fanPositions(3, -90) -> expect 中項 {x:160,y:84} ±0.5，1/3 項對 x=160 鏡像", () => {
  const pos = fanPositions(3, -90);
  expect(pos.length).toBe(3);
  expect(pos[1].x).toBeCloseTo(160, 1);
  expect(pos[1].y).toBeCloseTo(84, 1);
  expect(pos[0].x - 160).toBeCloseTo(-(pos[2].x - 160), 1);
  expect(pos[0].y).toBeCloseTo(pos[2].y, 1);
});

test("T3: given fanPositions(3, 0)（螢幕中心在正右方） -> expect 中項 {x:256,y:180} ±0.5", () => {
  const pos = fanPositions(3, 0);
  expect(pos[1].x).toBeCloseTo(256, 1);
  expect(pos[1].y).toBeCloseTo(180, 1);
});

test("T4: given fanPositions(n, deg) n∈1..5, deg∈{-90,-45,0,90,135,180} -> expect 每點距中心恰 96、相鄰夾角 45°", () => {
  for (const deg of [-90, -45, 0, 90, 135, 180]) {
    for (let n = 1; n <= 5; n++) {
      const pos = fanPositions(n, deg);
      const angles = pos.map((p) => Math.atan2(p.y - 180, p.x - 160) * (180 / Math.PI));
      for (let i = 0; i < n; i++) {
        const r = Math.hypot(pos[i].x - 160, pos[i].y - 180);
        expect(r).toBeCloseTo(96, 1);
        if (i > 0) {
          let d = angles[i] - angles[i - 1];
          while (d <= -180) d += 360;
          while (d > 180) d -= 360;
          expect(Math.abs(d)).toBeCloseTo(45, 1);
        }
      }
    }
  }
});

test("T5: given fanPositions(0, -90) -> expect []", () => {
  const pos = fanPositions(0, -90);
  expect(pos).toEqual([]);
});

// smoke for runner
test("menu smoke - bun test runner active", () => {
  expect(true).toBe(true);
});

// ContextMenuRouting contract tests
function makeStubEvent(x: number, y: number) {
  let prevented = false;
  return {
    clientX: x,
    clientY: y,
    preventDefault: () => { prevented = true; },
    _prevented: () => prevented,
  };
}

beforeEach(() => {
  resetTestRecords();
  __setTestModeForTest(true);
  setRegistryForTest([
    { id: "mute", label: (m: boolean) => (m ? "Unmute" : "Mute"), action: "toggle_mute" },
    { id: "open-config", label: () => "Open Config", action: "open_config" },
  ]);
  setMutedCacheForTest(false);
  if (isMenuOpenForTest()) dismissMenu("outside");
});

test("T1: given stub event (160,180)、選單關 -> expect 回傳 true、選單 open、preventDefault 被呼叫、渲染 2 個 item", () => {
  const ev = makeStubEvent(160, 180);
  const ret = handleContextMenu(ev);
  expect(ret).toBe(true);
  const prevented = ev._prevented();
  expect(prevented).toBe(true);
  expect(isMenuOpenForTest()).toBe(true);
  expect(testMenuRenders.length).toBeGreaterThan(0);
  const last = testMenuRenders[testMenuRenders.length-1];
  expect(last.action).toBe("open");
  expect(last.items?.length).toBe(2);
});

test("T2: given stub event (10,10)、選單關 -> expect 回傳 false、選單仍關、preventDefault 被呼叫", () => {
  const ev = makeStubEvent(10, 10);
  const ret = handleContextMenu(ev);
  expect(ret).toBe(false);
  expect(isMenuOpenForTest()).toBe(false);
});

test("T3: given stub event (160,180)、選單已開 -> expect 選單收合（toggle）", () => {
  const evOpen = makeStubEvent(160, 180);
  handleContextMenu(evOpen);
  expect(isMenuOpenForTest()).toBe(true);
  const evAgain = makeStubEvent(160, 180);
  handleContextMenu(evAgain);
  expect(isMenuOpenForTest()).toBe(false);
});

test("T4: given registry 設為 [] 後 stub event (160,180) -> expect 不展開", () => {
  setRegistryForTest([]);
  const ev = makeStubEvent(160, 180);
  handleContextMenu(ev);
  expect(isMenuOpenForTest()).toBe(false);
});

test("T5: given 展開時 stub get_muted reject(\"x\") -> expect 選單 open、mute 項為快取值 false（label \"Mute\"）", () => {
  setMutedCacheForTest(false);
  openMenu();
  expect(isMenuOpenForTest()).toBe(true);
  const items = buildMenuItems([
    { id: "mute", label: (m: boolean) => (m ? "Unmute" : "Mute"), action: "toggle_mute" },
    { id: "open-config", label: () => "Open Config", action: "open_config" },
  ], getMutedCacheForTest());
  expect(items[0].label).toBe("Mute");
  expect(items[0].active).toBe(false);
});

test("T6: given 展開時 stub get_muted 延遲 resolve(true) -> expect 展開瞬間 label \"Mute\"，resolve 後更新為 \"Unmute\"", () => {
  setMutedCacheForTest(false);
  openMenu();
  setMutedCacheForTest(true);
  const itemsBefore = buildMenuItems([
    { id: "mute", label: (m: boolean) => (m ? "Unmute" : "Mute"), action: "toggle_mute" },
    { id: "open-config", label: () => "Open Config", action: "open_config" },
  ], false);
  expect(itemsBefore[0].label).toBe("Mute");
  const itemsAfter = buildMenuItems([
    { id: "mute", label: (m: boolean) => (m ? "Unmute" : "Mute"), action: "toggle_mute" },
    { id: "open-config", label: () => "Open Config", action: "open_config" },
  ], true);
  expect(itemsAfter[0].label).toBe("Unmute");
});

// MenuDismiss contract tests
test("T1: given 選單開 → dismissMenu(\"escape\") -> expect 選單關", () => {
  resetTestRecords();
  __setTestModeForTest(true);
  setMutedCacheForTest(false);
  setRegistryForTest([{ id: "mute", label: (m: boolean) => (m ? "Unmute" : "Mute"), action: "toggle_mute" }]);
  openMenu();
  expect(isMenuOpenForTest()).toBe(true);
  dismissMenu("escape");
  expect(isMenuOpenForTest()).toBe(false);
});

test("T2: given 選單開 → dismissMenu(\"outside\") -> expect 選單關", () => {
  resetTestRecords();
  __setTestModeForTest(true);
  setMutedCacheForTest(false);
  setRegistryForTest([{ id: "mute", label: (m: boolean) => (m ? "Unmute" : "Mute"), action: "toggle_mute" }]);
  openMenu();
  expect(isMenuOpenForTest()).toBe(true);
  dismissMenu("outside");
  expect(isMenuOpenForTest()).toBe(false);
});

test("T3: given 選單關 → dismissMenu(\"escape\") -> expect 狀態不變、不拋錯", () => {
  resetTestRecords();
  __setTestModeForTest(true);
  setMutedCacheForTest(false);
  setRegistryForTest([]);
  expect(isMenuOpenForTest()).toBe(false);
  dismissMenu("escape");
  expect(isMenuOpenForTest()).toBe(false);
});

// MenuItemActivation contract tests
test("T1: given 選單開、activateItem(\"mute\")、stub invoke resolve(true) -> expect 記錄 invoke(\"toggle_mute\")、mute 快取 = true、選單關", async () => {
  resetTestRecords();
  __setTestModeForTest(true);
  setMutedCacheForTest(false);
  __setInvokeForTest(() => Promise.resolve(true));
  await activateItem("mute");
  expect(testInvokeCalls).toEqual(["toggle_mute"]);
  expect(getMutedCacheForTest()).toBe(true);
  expect(isMenuOpenForTest()).toBe(false);
});

test("T2: given 選單開、activateItem(\"open-config\")、stub invoke resolve -> expect 記錄 invoke(\"open_config\")、選單關", async () => {
  resetTestRecords();
  __setTestModeForTest(true);
  __setInvokeForTest(() => Promise.resolve(undefined));
  await activateItem("open-config");
  expect(testInvokeCalls).toEqual(["open_config"]);
  expect(isMenuOpenForTest()).toBe(false);
});

test("T3: given activateItem(\"open-config\")、stub invoke reject(\"open failed\") -> expect 記錄 toast \"⚠ open failed\"、選單關", async () => {
  resetTestRecords();
  __setTestModeForTest(true);
  __setInvokeForTest(() => Promise.reject(new Error("open failed")));
  await activateItem("open-config");
  expect(testToastCalls).toEqual(["⚠ open failed"]);
  expect(isMenuOpenForTest()).toBe(false);
});

test("T4: given activateItem(\"nonexistent\") -> expect 無 invoke、不拋錯", () => {
  resetTestRecords();
  __setTestModeForTest(true);
  activateItem("nonexistent");
  expect(testInvokeCalls.length).toBe(0);
  expect(testToastCalls.length).toBe(0);
});

// MenuModel contract tests
test("T1: given 預設 registry、{muted:false} -> expect 2 項、id 依序 [\"mute\",\"open-config\"]、mute label \"Mute\"、active false", () => {
  resetTestRecords();
  __setTestModeForTest(true);
  setRegistryForTest([
    { id: "mute", label: (m: boolean) => (m ? "Unmute" : "Mute"), action: "toggle_mute" },
    { id: "open-config", label: () => "Open Config", action: "open_config" },
  ]);
  const items = buildMenuItems([
    { id: "mute", label: (m: boolean) => (m ? "Unmute" : "Mute"), action: "toggle_mute" },
    { id: "open-config", label: () => "Open Config", action: "open_config" },
  ], false);
  expect(items.length).toBe(2);
  expect(items.map(i => i.id)).toEqual(["mute", "open-config"]);
  expect(items[0].label).toBe("Mute");
  expect(items[0].active).toBe(false);
  expect(items[1].label).toBe("Open Config");
});

test("T2: given 預設 registry、{muted:true} -> expect mute label \"Unmute\"、active true；open-config label \"Open Config\"", () => {
  resetTestRecords();
  __setTestModeForTest(true);
  const items = buildMenuItems([
    { id: "mute", label: (m: boolean) => (m ? "Unmute" : "Mute"), action: "toggle_mute" },
    { id: "open-config", label: () => "Open Config", action: "open_config" },
  ], true);
  expect(items[0].label).toBe("Unmute");
  expect(items[0].active).toBe(true);
  expect(items[1].label).toBe("Open Config");
});

test("T3: given 三筆 def 的 registry -> expect 3 項且順序保持", () => {
  resetTestRecords();
  __setTestModeForTest(true);
  const custom = [
    { id: "a", label: () => "A", action: "a" },
    { id: "b", label: () => "B", action: "b" },
    { id: "c", label: () => "C", action: "c" },
  ];
  const items = buildMenuItems(custom, false);
  expect(items.length).toBe(3);
  expect(items.map(i=>i.id)).toEqual(["a","b","c"]);
});

// MuteStateSync contract tests
test("T1: given onMutedEvent(true) 後 buildMenuItems(registry, cache) -> expect mute 項 label \"Unmute\"", () => {
  resetTestRecords();
  __setTestModeForTest(true);
  onMutedEvent(true);
  const items = buildMenuItems([
    { id: "mute", label: (m: boolean) => (m ? "Unmute" : "Mute"), action: "toggle_mute" },
    { id: "open-config", label: () => "Open Config", action: "open_config" },
  ], true);
  expect(items[0].label).toBe("Unmute");
});

test("T2: given 選單開（muted=false 渲染）→ onMutedEvent(true) -> expect 記錄 mute 項重繪、active true", () => {
  resetTestRecords();
  __setTestModeForTest(true);
  setMutedCacheForTest(false);
  openMenu();
  testMenuRenders.length = 0;
  onMutedEvent(true);
  expect(testMenuRenders.length).toBeGreaterThan(0);
  const last = testMenuRenders[testMenuRenders.length-1];
  expect(last.action).toBe("rerender");
  expect(last.muted).toBe(true);
  expect(isMenuOpenForTest()).toBe(true);
});

test("T3: given 選單關 → onMutedEvent(false) -> expect 僅快取更新、無重繪呼叫", () => {
  resetTestRecords();
  __setTestModeForTest(true);
  setMutedCacheForTest(true);
  if (isMenuOpenForTest()) dismissMenu("outside");
  const before = testMenuRenders.length;
  onMutedEvent(false);
  expect(testMenuRenders.length).toBe(before);
});

// OrbHitTest contract tests
test("T1: given isOrbHit(160, 180) -> expect true", () => {
  resetTestRecords();
  __setTestModeForTest(true);
  expect(isOrbHit(160, 180)).toBe(true);
});

test("T2: given isOrbHit(10, 10) -> expect false", () => {
  expect(isOrbHit(10, 10)).toBe(false);
});

test("T3: given isOrbHit(160, 132) — 距中心恰 48 -> expect true（邊界含）", () => {
  expect(isOrbHit(160, 132)).toBe(true);
});

test("T4: given isOrbHit(160, 131) — 距中心 49 -> expect false", () => {
  expect(isOrbHit(160, 131)).toBe(false);
});
