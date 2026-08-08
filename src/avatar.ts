// Radial-blob avatar, ported from SumVox prototype/orb-canvas.html
// (48-point Catmull-Rom rim, cold radial gradient).
// setLevel() drives wobble+swell; setSpeaking() switches 20fps-dim / 60fps-bright.
// setBusy() / setPending() carry the two states that used to need a bubble:
// busy = 收緊、褪色、快（系統在動，你不用動）；pending = 舒張、鑲琥珀環、回到
// 正常速度（系統停下來了，該你動）。刻意互斥，見 DESIGN.md §4。

const SIZE = 160,
  CX = 80,
  CY = 80,
  BASE_R = 54,
  SEG = 48;

// Gradient stops, idle and busy. Interpolating between the two tables
// reproduces §4's numbers exactly at both ends — cheaper and more faithful
// than re-deriving the desaturation that produced the busy column.
const STOPS_IDLE = [[214, 240, 248], [64, 206, 232], [22, 84, 100]];
const STOPS_BUSY = [[176, 196, 206], [104, 150, 166], [44, 64, 72]];
const AMBER = "244,178,60"; // --ui-pending #f4b23c

// All line widths below are canvas units. The canvas is 160 and CSS shows it
// at 120, so 顯示線寬 × (160/120) = 顯示線寬 × 1.3333. Filling the spec's
// display values straight in would give three quarters of the intended weight.
const RIM_W = 1.33; // 顯示 1
const PENDING_RIM_W = 2.13; // 顯示 1.6
const PENDING_GLOW_W = 4.67; // 顯示 3.5

let cv: HTMLCanvasElement;
let ctx: CanvasRenderingContext2D;

let lvl = 0,
  lvlTarget = 0,
  active = false,
  busy = false,
  pending = false;
let breathe = 0,
  idle = 0,
  wobA = 0,
  wobB = 0;
// busy coefficients ease toward their target instead of snapping
let kSpeed = 1,
  kRadius = 1,
  kWobble = 1,
  kFade = 0;
let last = 0,
  tick = 0;

export function setLevel(v: number) {
  lvlTarget = Math.min(1, Math.max(0, v));
}

export function setSpeaking(on: boolean) {
  active = on;
  if (!on) lvlTarget = 0;
  applyOpacity();
}

// The system is working on something of its own — no bubble, the creature
// just holds its breath. A confirm is up? Then it is the user's move: pending
// wins and busy is dropped by the caller.
export function setBusy(on: boolean) {
  busy = on;
  applyOpacity();
}

export function setPending(on: boolean) {
  pending = on;
  applyOpacity();
}

function applyOpacity() {
  if (!cv) return;
  cv.style.opacity = String(active ? 1 : pending ? 0.92 : busy ? 0.78 : 0.55);
}

// sample blob radius at angle θ — stacked sines (idle drift + level-driven wobble)
function radius(theta: number, l: number) {
  const b = 1 + 0.04 * Math.sin(breathe);
  const swell = 1 + 0.15 * l;
  const idleW =
    kWobble *
    (0.05 * Math.sin(2 * theta + idle) + 0.03 * Math.sin(3 * theta + idle * 0.7));
  // kWobble only touches the two terms that are NOT volume-driven. The
  // 0.12*l*sin(3θ) term is the mouth; damping it would flatten speech while busy.
  const driven =
    0.12 * l * Math.sin(3 * theta + wobA) +
    kWobble * 0.06 * Math.sin(5 * theta - wobB);
  return BASE_R * kRadius * b * swell * (1 + idleW + driven);
}

// Catmull-Rom → cubic bezier closed path through 48 rim points
function blobPath(l: number) {
  const pts: [number, number][] = [];
  for (let i = 0; i < SEG; i++) {
    const th = (i / SEG) * Math.PI * 2;
    const r = radius(th, l);
    pts.push([CX + r * Math.cos(th), CY + r * Math.sin(th)]);
  }
  ctx.beginPath();
  for (let i = 0; i < SEG; i++) {
    const p0 = pts[(i - 1 + SEG) % SEG],
      p1 = pts[i],
      p2 = pts[(i + 1) % SEG],
      p3 = pts[(i + 2) % SEG];
    const c1 = [p1[0] + (p2[0] - p0[0]) / 6, p1[1] + (p2[1] - p0[1]) / 6];
    const c2 = [p2[0] - (p3[0] - p1[0]) / 6, p2[1] - (p3[1] - p1[1]) / 6];
    if (i === 0) ctx.moveTo(p1[0], p1[1]);
    ctx.bezierCurveTo(c1[0], c1[1], c2[0], c2[1], p2[0], p2[1]);
  }
  ctx.closePath();
}

function paint() {
  ctx.clearRect(0, 0, SIZE, SIZE);
  const lvlP = Math.pow(lvl, 1.4);
  blobPath(lvlP);

  // fill: clip to blob, radial gradient from upper-left highlight
  ctx.save();
  ctx.clip();
  const g = ctx.createRadialGradient(CX - 9, CY + 9, 0, CX, CY, BASE_R * 1.15);
  [0, 0.55, 1].forEach((at, i) => {
    const rgb = STOPS_IDLE[i].map((c, j) =>
      Math.round(c + (STOPS_BUSY[i][j] - c) * kFade),
    );
    g.addColorStop(at, `rgb(${rgb.join(",")})`);
  });
  ctx.fillStyle = g;
  ctx.fillRect(0, 0, SIZE, SIZE);
  ctx.restore();

  // rim — hairline, or the amber ring that says the next move is yours
  blobPath(lvlP);
  if (pending) {
    ctx.strokeStyle = `rgba(${AMBER},0.35)`;
    ctx.lineWidth = PENDING_GLOW_W;
    ctx.stroke();
    ctx.strokeStyle = `rgba(${AMBER},0.9)`;
    ctx.lineWidth = PENDING_RIM_W;
  } else {
    ctx.strokeStyle = "rgba(255,255,255,0.25)";
    ctx.lineWidth = RIM_W;
  }
  ctx.stroke();
}

function loop(t: number) {
  const now = t / 1000;
  const dt = last ? Math.min(now - last, 0.1) : 1 / 60;
  last = now;
  // phases advance every tick (real-time motion even when we skip redraw)
  lvl += (lvlTarget - lvl) * (1 - Math.exp(-12 * dt));
  // busy: 呼吸 2.4 rad/s · 半徑 ×0.90 · 輪廓抖動 ×0.45 · 漸層褪色，全部趨近不硬切
  const k = 1 - Math.exp(-6 * dt);
  kSpeed += ((busy ? 2.4 / 1.1 : 1) - kSpeed) * k;
  kRadius += ((busy ? 0.9 : 1) - kRadius) * k;
  kWobble += ((busy ? 0.45 : 1) - kWobble) * k;
  kFade += ((busy ? 1 : 0) - kFade) * k;
  breathe += 1.1 * kSpeed * dt;
  idle += 0.7 * dt;
  wobA += 2.2 * dt;
  wobB += 1.4 * dt;
  // adaptive fps: idle 20fps (stride 3 @60), active/busy 60fps (stride 1) —
  // a doubled breath rate at 20fps reads as stutter, not as urgency
  tick++;
  const stride = active || busy ? 1 : 3;
  if (tick % stride === 0) paint();
  requestAnimationFrame(loop);
}

export function initAvatar(canvas: HTMLCanvasElement) {
  cv = canvas;
  const dpr = window.devicePixelRatio || 1;
  cv.width = SIZE * dpr;
  cv.height = SIZE * dpr;
  ctx = cv.getContext("2d")!;
  ctx.scale(dpr, dpr);
  setSpeaking(false);
  requestAnimationFrame(loop);
}
