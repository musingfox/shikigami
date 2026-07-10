// Radial-blob avatar, ported from SumVox prototype/orb-canvas.html
// (48-point Catmull-Rom rim, emerald→cyan radial gradient).
// setLevel() drives wobble+swell; setSpeaking() switches 20fps-dim / 60fps-bright.

const SIZE = 160,
  CX = 80,
  CY = 80,
  BASE_R = 54,
  SEG = 48;

// palette — emerald → cyan (sRGB). ctr = emerald mixed toward white (highlight core)
const FR = 0.204,
  FG = 0.827,
  FB = 0.6; // #34d399
const TR = 0.133,
  TG = 0.827,
  TB = 0.933; // #22d3ee
const CTR = [FR * 0.6 + 0.4, FG * 0.6 + 0.4, FB * 0.6 + 0.4];
const MID = [TR, TG, TB];
const RIM = [TR * 0.45, TG * 0.45, TB * 0.45];

let cv: HTMLCanvasElement;
let ctx: CanvasRenderingContext2D;

let lvl = 0,
  lvlTarget = 0,
  active = false;
let breathe = 0,
  idle = 0,
  wobA = 0,
  wobB = 0;
let last = 0,
  tick = 0;

export function setLevel(v: number) {
  lvlTarget = Math.min(1, Math.max(0, v));
}

export function setSpeaking(on: boolean) {
  active = on;
  if (!on) lvlTarget = 0;
  cv.style.opacity = on ? "1" : "0.55";
}

// sample blob radius at angle θ — stacked sines (idle drift + level-driven wobble)
function radius(theta: number, l: number) {
  const b = 1 + 0.04 * Math.sin(breathe);
  const swell = 1 + 0.15 * l;
  const idleW =
    0.05 * Math.sin(2 * theta + idle) + 0.03 * Math.sin(3 * theta + idle * 0.7);
  const driven =
    0.12 * l * Math.sin(3 * theta + wobA) + 0.06 * Math.sin(5 * theta - wobB);
  return BASE_R * b * swell * (1 + idleW + driven);
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
  g.addColorStop(0, `rgb(${CTR.map((c) => Math.round(c * 255)).join(",")})`);
  g.addColorStop(0.55, `rgb(${MID.map((c) => Math.round(c * 255)).join(",")})`);
  g.addColorStop(1, `rgb(${RIM.map((c) => Math.round(c * 255)).join(",")})`);
  ctx.fillStyle = g;
  ctx.fillRect(0, 0, SIZE, SIZE);
  ctx.restore();

  // hairline rim
  blobPath(lvlP);
  ctx.strokeStyle = "rgba(255,255,255,0.25)";
  ctx.lineWidth = 1;
  ctx.stroke();
}

function loop(t: number) {
  const now = t / 1000;
  const dt = last ? Math.min(now - last, 0.1) : 1 / 60;
  last = now;
  // phases advance every tick (real-time motion even when we skip redraw)
  lvl += (lvlTarget - lvl) * (1 - Math.exp(-12 * dt));
  breathe += 1.1 * dt;
  idle += 0.7 * dt;
  wobA += 2.2 * dt;
  wobB += 1.4 * dt;
  // adaptive fps: idle 20fps (stride 3 @60), active 60fps (stride 1)
  tick++;
  const stride = active ? 1 : 3;
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
