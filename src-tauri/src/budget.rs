//! Every number that bounds what reaches the model in one turn, in one place.
//!
//! The four layers, and which of these each one spends (`r-brain-tool-loop`):
//!
//! - **fixed** — persona + tool declarations. No budget: it is the same every turn.
//! - **rebuilt each turn** — roster + precise hook signal. `PRECISE_MAX_*` per agent,
//!   `DEPTH_MAX_CHARS` across all of them.
//! - **rolling** — `HISTORY_DEPTH` turns, each fitted to `TURN_MAX_*`, plus the
//!   curated file capped at `CURATED_MAX_CHARS`.
//! - **on demand** — what a tool fetched: `SCREEN_MAX_*` per `read_pane`. It lives
//!   for the turn that fetched it and is never written back into the rolling layer.
//!
//! `ROTATE_MAX_BYTES` is the odd one out — it bounds the file on disk, not the
//! prompt. It sits here because it is the same kind of decision, deliberately soft:
//! a failed rotation still appends, because memory continuity beats a hard bound.
//!
//! Loop bounds (`MAX_STEPS`, `TURN_BUDGET`, `MIN_STEP_BUDGET`) are **not** here.
//! They bound how long a turn may take, not how much context it carries.

/// Rolling layer: how many past turns reach the prompt.
pub const HISTORY_DEPTH: usize = 6;
/// Rolling layer: one turn row, before it is stored or replayed.
pub const TURN_MAX_LINES: usize = 8;
pub const TURN_MAX_CHARS: usize = 400;
/// Curated layer: `MEMORY.md`. Advisory — over this we warn and never truncate,
/// because silently dropping the user's own words is worse than a long prompt.
pub const CURATED_MAX_CHARS: usize = 4000;

/// Rebuilt each turn: one agent's precise hook signal (label and detail each).
pub const PRECISE_MAX_LINES: usize = 6;
pub const PRECISE_MAX_CHARS: usize = 200;
/// Rebuilt each turn: the ceiling across every agent's depth together, so a big
/// roster cannot crowd out the question itself.
pub const DEPTH_MAX_CHARS: usize = 2000;

/// On demand: one `read_pane` excerpt. Same shape the prefetched screen excerpt
/// used before it moved to a tool, so the model pays the same either way.
pub const SCREEN_MAX_LINES: usize = 12;
pub const SCREEN_MAX_CHARS: usize = 600;

/// On disk, not in the prompt: rotate `memory.jsonl` past this, keeping one
/// generation.
pub const ROTATE_MAX_BYTES: u64 = 1_048_576;

/// In the spool, not in the prompt: one hook entry's detail is truncated to this
/// on the way into `HOOK_DEPTHS`, keeping the tail, so a runaway hook line cannot
/// hold megabytes in memory. `PRECISE_MAX_CHARS` is what actually bounds the
/// prompt — this only bounds what we are willing to remember long enough to
/// normalize.
pub const HOOK_DETAIL_MAX_CHARS: usize = 2000;
