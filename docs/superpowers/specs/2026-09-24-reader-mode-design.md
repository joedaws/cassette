# Reader mode — focus without holding the lock

> Written unattended overnight on 2026-09-24 from `docs/follow-up.md`'s "Reader mode" entry.
> Judgement calls are under **Decisions**; the big one (toggle vs. lock-on-intent) is answered
> in **The larger question** and flagged for review.

## Purpose

Today **focus means held** (`session_writer.rs`): focusing a cassette takes its lock and keeps it
until focus moves. So nobody can watch a cassette being written. To let an agent write one, the
human must Tab away, and it collapses to its last line. Under the distillation model the Stage 2
trial arrived at (each writer rewrites the shared nugget), watching the nugget being refined is
the whole point.

## What already exists (verified against `follow-up-specs` @ `a8d0573`)

The follow-up's sketch holds up. Checked:

- `sync_external_writes` skips only `SessionWriter::held_id()`. A focused but unheld cassette is
  merged like any other, full-height, today. That is exactly what a **Busy** cassette already
  does. Reading is Busy, except the human chose it.
- `merge_external`'s follow-if-at-end cursor rule applies unmodified.
- `retry_lock` retries only `ReadOnly::Busy`, so a new variant is not retried by accident.
- `flush_held`/`finish`/`suspend_session`/autosave are all no-ops with no guard.

Two things the sketch missed:

1. **Cursor motion is gated too.** `App::modify_focused` is the read-only gate, and every
   normal-mode motion (`h j k l w b 0 $ gg G`) goes through it. A reader could not scroll.
   Worse, `modify_focused` marks the cassette `dirty`. A dirty unheld cassette would trip
   `refresh_from_disk`'s "keep unwritten words" belt on the next acquire and **skip the
   re-read**, which is the lost-update bug 5b's invariant exists to prevent.
2. **There is no release path.** The follow-up says "what `Tab` already does, minus the focus
   move". But `acquire` only drops the old guard on the way to taking a new one, and `finish`
   also removes sessions. A plain `release` has to be added.

## The larger question: a toggle, or lock-on-intent?

The follow-up asks whether reading should be a mode you toggle, or whether focus should stop
implying the lock altogether: take it on first keystroke, release on idle.

**Recommendation: lock-on-intent is the right end state, and the toggle is how to get there.**
The automatic version is the toggle plus two triggers:

- *intent → acquire*: entering insert mode (`i a I A o O`) from Reading takes the lock.
- *idle → release*: sitting in **normal** mode for N seconds releases it.

So the toggle has to exist either way, and it can ship and be used first. Phase 2 adds the idle
trigger behind a config key, **off by default**. The default flips only after the user has
lived with it (open question).

Why not jump straight to automatic:

- **"First keystroke" is the wrong trigger.** `handle_key` is pure and the lock is taken
  afterwards in `follow_focus`. A keystroke that has to *win* the lock before it can land either
  gets dropped or needs replay. Entering insert mode is a keystroke with no text effect, so it
  is a safe place to win the lock. Typing is not.
- **Idle release in *insert* mode would drop the next character.** The human pauses
  mid-sentence, an agent grabs the cassette, and the next letter goes nowhere. So the idle
  release only applies in normal mode. `Esc` means "I've stopped writing", which is the signal
  the trial showed (the user escaped out and waited for the agent).

## Design

### Phase 1: the toggle

**States.** `ReadOnly` gains `Reading { id: String }`:

```rust
pub enum ReadOnly { No, Busy { holder: Option<String> }, Closed, Reading { id: String } }
```

The id pins Reading to the cassette it was entered on, so `follow_focus` can tell "focus moved"
without a held guard to compare against.

**Motion is not an edit.** A new `App::view_focused(f)` applies a closure to the focused cassette
**bypassing the read-only gate and never setting `dirty`**. All normal-mode motions, and the
Ctrl+B flip, route through it. This also means Busy and Closed cassettes can now be scrolled,
which they should always have allowed. The `snapshot()` calls stay on `modify_focused`: they are
undo-stack writes, not views.

**Keys** (normal mode):

- `r`: **release to read.** When the focused cassette is held: flush through the guard, drop
  it, set `Reading { id }`. When it is already Reading: re-acquire (the toggle). A no-op when
  Busy or Closed (there is nothing of ours to release).
- `i a I A o O` while Reading: **intent to write.** The key is stashed in
  `App::replay_after_acquire` and the mode stays Normal. `follow_focus` tries the lock. On
  success it replays the key through `handle_normal_key`, so `a` still moves right and `o`
  still opens a line, now against the freshly re-read text. On failure the state becomes
  `Busy`, as for any contended focus.

The pure/IO split holds: `handle_normal_key` only sets `app.release_requested` (one-shot, like
`suspend`) or `replay_after_acquire`. `follow_focus` does the store work.

**`SessionWriter::release(&mut self, app: &mut App) -> Result<(), LockError>`**: `flush_held`,
then `self.guard = None`. That is "flush first, drop second", the same order `acquire` uses.

**`follow_focus` changes**, in order:

1. `release_requested` → if held, `w.release(app)`; `read_only = Reading { id }`.
2. Reading and the focused id still equals `id` and no `replay_after_acquire` → **return**
   (don't acquire; this is the whole point).
3. Reading and focus moved → fall through to today's acquire path. **Reading ends when you
   leave the cassette**: it describes a cassette, not a session-wide mode.
4. `replay_after_acquire` → `try_acquire`; on `ReadOnly::No`, replay.

**Screen.** `info_text`: `-- READING (open to writers) --`. `help_text`:
`reading: others may write here  i/a/o:write  r:hold again  hjkl:scroll  Tab:next  ^C:quit`. The
wording differs from BUSY (someone took it, wait) and CLOSED (needs `queue reopen`). This one is
voluntary and undone by a key.

**Record mode** (`-R`) has no normal mode, so it can never enter Reading. Nothing to guard.

### Phase 2: idle release (config, off by default)

`config.toml`: `release_idle_secs = 20` (unset = never). On a tick where the mode is Normal, the
focused cassette is held and not dirty after the autosave flush, and `app.idle_secs >=
release_idle_secs`, do exactly what `r` does. `idle_secs` already exists (`tick_idle`, reset by
any keypress in `handle_key`) and is reused as-is.

With the key set, reading *is* the default whenever the human isn't actively writing, and
`i` claims the cassette back. That is the "take it when you write, release it when you stop"
model the follow-up proposes.

## Decisions

1. **Toggle first, automatic behind config.** See "The larger question". *Open question: after
   living with phase 2, should `release_idle_secs` default on, and at what value?*
2. **Intent = entering insert mode, with replay.** Not "first edit keystroke", which would drop
   or reorder text.
3. **Idle release only from normal mode.** Never from insert mode.
4. **Reading is per-cassette** (`Reading { id }`) and ends on focus change. Tab back re-acquires
   as today. A session-wide "read everything" mode is not asked for, and it would make
   Tab-to-write surprising.
5. **Motion works on every read-only cassette.** `view_focused` lifts the gate for Busy and
   Closed too. There was no reason to forbid scrolling a cassette you cannot write.
6. **`r` as the key.** Free in normal mode, mnemonic, and vim's `r` (replace char) isn't
   implemented here. *If you'd rather keep `r` for a future vim replace, `R` or `Ctrl+R` are
   the fallbacks.*
7. **No new sync code.** Proven by test rather than argued (plan Task 2).

## Out of scope

- Agents seeing "a human is reading this". A `reading` marker in the lock anchor or frontmatter
  would let `queue next` prefer other cassettes. Not needed: the trial's complaint was the
  opposite, too much exclusion.
- Changing `queue write`'s behavior when it contends with a human who is *about to* type.
  Contention is still settled by the `flock`.

## Acceptance criteria

- `r` on a held cassette: the file is flushed, `Store::holds` is false, the banner reads
  `-- READING (open to writers) --`, and `queue write` from another process exits 0.
- While Reading, an external `queue write` shows up full-height within one tick. With the cursor
  at the end it follows the new end; parked mid-text it stays.
- `hjkl`/`gg`/`G` move the cursor while Reading, Busy and Closed. None of them set `dirty`.
- `i` while Reading with the lock free: the lock is held, mode is Insert, the next typed char
  lands and autosaves. With the lock busy: `Busy` banner, mode stays Normal.
- `a` while Reading replays as `a`: the cursor is one right of where it was, over the re-read
  text.
- Tab away from a Reading cassette and back: it is held again (not Reading).
- Phase 2: with `release_idle_secs = 2`, sitting in normal mode for 2 s releases the lock.
  Sitting in insert mode never does.
- The `verify` skill drives the TUI through `r` → external write → `i` → type, and the captured
  screens show the banner and the merged text.
- CLAUDE.md's "Focus means held" wording is amended in `session_writer.rs`'s and `app.rs`'s
  paragraphs, and Key bindings gains `r`.
- `cargo test`, `cargo clippy --all-targets -- -D warnings` green after each phase.
