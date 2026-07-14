// Pure router for the orb's left-mousedown gesture.
// double-click = talk, single press+hold = drag, menu-dismiss = none.
// The menu-open guard is latched on the FIRST mousedown of a sequence
// (detail===1), not re-read each press: the 1st click dismisses the menu
// (isMenuOpen flips false), so a per-press read would let the 2nd click of the
// same double-click slip through and fire talk. We remember the opening state.

export type OrbAction = "talk" | "drag" | "none";

let seqMenuOpen = false;

export function orbGesture(detail: number, menuOpen: boolean): OrbAction {
  if (detail === 1) seqMenuOpen = menuOpen;
  if (seqMenuOpen) return "none"; // whole sequence just dismisses the menu
  return detail === 2 ? "talk" : "drag";
}

export function __resetOrbGestureForTest() {
  seqMenuOpen = false;
}
