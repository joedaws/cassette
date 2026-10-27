# Cassette Turn Status Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Every open cassette shows a glyph for whose move it is — ✎ human, ⚙ agent, ● busy — and agents can claim a cassette so their thinking time shows as busy.

**Architecture:** The turn status is derived, never stored: a pure resolver in a new `src/status.rs` reads plain scalars on `Cassette` (last writer's kind, claim expiry) plus a small context (held? now?). Claims are two new frontmatter fields written only by new `queue claim`/`queue release` commands and cleared by the holder's `queue write`. Non-turn queue commands stop stamping `last_writer` so the derivation stays honest.

**Tech Stack:** Rust, ratatui/crossterm, clap v4 derive, chrono, serde/toml.

**Spec:** `docs/superpowers/specs/2026-10-26-cassette-turn-status-design.md`

## Global Constraints

- Default glyphs: human `✎`, agent `⚙`, busy `●`. Config table `[status.turn]` with keys `human`, `agent`, `busy`, `pulse`.
- Default claim duration: **120 s** (`--for <SECS>` overrides).
- Exit codes: `Usage` 2, `Busy` 3, `Sticky` 4, `Empty` 5 — existing `QueueError` variants only.
- `last_writer` is stamped only by `queue write`, the TUI flush, and creation.
- `App`, `Cassette`, `status.rs` stay pure: no I/O, no ratatui. `cassette.rs` imports nothing.
- All colors through `Theme`; never hardcode in `ui.rs`.
- `cargo clippy -- -D warnings` green after every task: fold each API's consumer into the same task, never `#[allow(dead_code)]`.
- Run `/verify` before any commit that changes TUI behaviour (Tasks 4, 5).

## Review Focus

1. A claim whose `claimed_until` is in the past (agent crashed): must read as no claim everywhere — glyph, `next`, `claim` by another agent, JSON `null`. → tests in Task 2 and Task 3.
2. A cassette with only whitespace on both sides but an agent `last_writer` (agent created it with an empty template body): must be ✎, not ✎-by-accident through the agent rule only. → resolver test in Task 4.
3. An agent that runs `queue claim` with no `--writer` in the human's shell: exit 2 naming `--writer`, nothing written. → Task 2.
4. The TUI's own autosave on a claimed focused-then-left cassette must not drop the claim; the next sync must still show ●. → Task 4 (session_writer test).
5. A user glyph two cells wide (e.g. `🤖`) must not overflow the separator row. → Task 5 render test.

---

## Phase 1 — store and queue commands (CLI-testable on its own)

### Task 1: Only turns stamp `last_writer`

**Files:**
- Modify: `src/queue/edit.rs` — `close` (~291), `reopen` (~407), `lock` (~471), `unlock` (~522), `renumber_all` (~579), `move_cassette` (~813); their doc comments
- Test: `src/queue/edit.rs` tests (update the one at ~1150 asserting on `last_writer`)

**Interfaces:** Consumes nothing new. Produces: invariant "only `queue write`, `queue new`, TUI flush/create set `last_writer`" that Task 4's resolver relies on.

- [ ] **Step 1: Write failing tests** — one per command, each seeding a cassette with `last_writer = A`, running the command as writer `B`, and asserting `last_writer == A` and `updated_at` changed:
  `close_does_not_stamp_last_writer`, `reopen_does_not_stamp_last_writer`, `lock_does_not_stamp_last_writer`, `unlock_does_not_stamp_last_writer`, `move_does_not_stamp_last_writer`, `renumber_all_does_not_stamp_last_writer` (assert on every cassette in the session).
- [ ] **Step 2: Run** `cargo test --bin cassette does_not_stamp_last_writer` — expect 6 FAIL.
- [ ] **Step 3: Remove the `m.last_writer = …` lines** in the six functions. Rewrite each doc comment's ordering list that names `last_writer` (e.g. close's step 5) and point at `retopic`'s existing "a retitle is not a turn" note as the shared rule. Update the existing test at ~1150 to the new expectation. Drop now-unused `writer` bindings rather than prefixing `_`.
- [ ] **Step 4: Run** `cargo test && cargo clippy --all-targets -- -D warnings` — PASS.
- [ ] **Step 5: Commit** `fix(queue): only turns stamp last_writer`

### Task 2: Claim fields, `queue claim`, `queue release`

**Files:**
- Modify: `src/store/meta.rs` (fields, build, parse, `claim_live`)
- Modify: every `CassetteMeta { … }` literal (29 across `src/` and `tests/`) — add `claimed_by: None, claimed_until: None`
- Modify: `src/queue/edit.rs` (`claim`, `release`)
- Modify: `src/cli.rs` (`QueueAction::Claim/Release`, `QueueCmd::Claim/Release`, both exhaustive matches)
- Modify: `src/main.rs` (dispatch, next to `QueueCmd::Lock` at ~384)
- Test: `src/store/meta.rs`, `src/queue/edit.rs`, `tests/cli.rs`

**Interfaces:**
- Produces:
  - `CassetteMeta.claimed_by: Option<String>` (writer id), `CassetteMeta.claimed_until: Option<String>` (RFC3339 UTC, seconds)
  - `pub fn claim_live(m: &CassetteMeta, now: chrono::DateTime<chrono::Utc>) -> Option<&str>` in `meta.rs` — the claimant id iff `claimed_by` is set and `claimed_until` parses and is `> now`
  - `pub fn claim(store: &Store, session: &str, id: &str, who_name: &str, source: WriterSource, secs: u64) -> Result<(), QueueError>`
  - `pub fn release(store: &Store, session: &str, id: &str, who_name: &str, source: WriterSource) -> Result<(), QueueError>`
  - `pub const DEFAULT_CLAIM_SECS: u64 = 120;` in `edit.rs`

- [ ] **Step 1: Write failing meta tests**
  - `frontmatter_round_trips_a_claim`: build → split preserves both fields; lines appear right after `locked_by:` as `claimed_by: wri_…` / `claimed_until: 2026-10-26T14:05:00Z`, and as bare `claimed_by:` / `claimed_until:` when `None`.
  - `missing_claim_keys_parse_as_unclaimed` (an existing-store file).
  - `non_writer_claimed_by_is_damaged`: `claimed_by: bot` → `split` returns `(None, _)`.
  - `unparseable_claimed_until_is_not_live`: `claim_live(..) == None`.
  - `claim_live_respects_expiry`: `now = until - 1s` → `Some(id)`; `now = until` → `None`.
- [ ] **Step 2: Run** `cargo test --bin cassette claim` — FAIL (no fields).
- [ ] **Step 3: Implement the meta changes.** Follow `locked_by`'s exact build/parse shape; validate `claimed_by` with `ids::check(IdKind::Writer, …)` in the same `is_writer` block. Keep the raw `claimed_until` string (garbage tolerated, read as not live by `claim_live`). Fix all struct literals.
- [ ] **Step 4: Write failing command tests** in `edit.rs`:
  - `claim_sets_claimant_and_expiry` (agent via `Flag`; `claimed_until ≈ now+120`; `last_writer` and `updated_at` untouched)
  - `claim_by_same_agent_extends`
  - `claim_over_other_agents_live_claim_is_sticky` (4) / `claim_over_expired_claim_succeeds`
  - `claim_from_env_source_is_usage` (2; message contains `--writer`) / `claim_by_human_is_usage` (2)
  - `claim_on_held_cassette_is_busy` (3; hold the lock in-process via `store.lock`)
  - `claim_on_closed_cassette_is_usage` (2)
  - `claim_on_sticky_locked_cassette_is_sticky` (4, `locked_by` another writer)
  - `release_clears_own_claim`, `release_unclaimed_is_noop_no_write` (mtime unchanged), `release_other_agents_live_claim_is_sticky` (4), `human_release_clears_any_claim`
- [ ] **Step 5: Implement `claim`/`release`** in `edit.rs` using `lock`'s order of operations: `resolve_acting` → kind/source refusal before acquiring → `store.lock` (`lock_error_to_queue_error`) → read through guard → table → write. `claim` refuses with `Usage` when `acting.source == Env || acting.authority != Agent`, using `acting.hint(who_name)` plus an explicit "pass --writer <agent>" when the hint is empty. `release` allows agents (own claim only, else `Sticky`) and humans (any). Neither touches `last_writer`/`updated_at`.
- [ ] **Step 6: CLI + dispatch.** `queue claim <ID> --session <SES> [--for <SECS>]` (default `DEFAULT_CLAIM_SECS`) and `queue release <ID> --session <SES>`, both taking the same writer option the other mutating commands take. Add to `cassette_ids()` (`<ID>`) and `session()`. Help text: claim — "mark a cassette picked up by an agent; shows as busy until written, released, or expired".
- [ ] **Step 7: Add `tests/cli.rs`** `claim_without_writer_exits_2` and `claim_then_release_round_trip` (store under `CASSETTE_DATA_DIR`, register `bot` as agent, `queue new`, `claim --writer bot`, check `queue show` frontmatter, `release`, check cleared).
- [ ] **Step 8: Run** `cargo test && cargo clippy --all-targets -- -D warnings` — PASS.
- [ ] **Step 9: Commit** `feat(queue): agent claims — queue claim and queue release`

### Task 3: Writes, `next` and JSON respect claims

**Files:**
- Modify: `src/queue/write.rs` (`write_permitted`, `read_check_and_write`)
- Modify: `src/queue/view.rs` (`open_candidates`/`first_free`, `build_view`)
- Modify: `src/queue/json.rs` (`CassetteView`)
- Test: same files

**Interfaces:**
- Consumes: `meta::claim_live` (Task 2).
- Produces: `CassetteView.claimed_by: Option<WriterRef>`, `CassetteView.claimed_until: Option<String>` — both `None` unless the claim is live.

- [ ] **Step 1: Failing tests**
  - write.rs: `holder_write_clears_claim`; `agent_write_over_other_agents_live_claim_is_sticky` (4, body unchanged); `agent_write_over_expired_claim_succeeds_and_clears_it`; `human_write_over_claim_keeps_claim`.
  - view.rs: `next_skips_cassette_claimed_by_another`; `next_returns_busy_when_only_claimed_remain` (exit-3 `Busy`, not 5); `next_ignores_expired_claim`; `view_reports_live_claim` / `view_hides_expired_claim` (`null`s).
  - json.rs: extend the serialization test to assert `claimed_by`/`claimed_until` keys exist and are `null` when unclaimed.
- [ ] **Step 2: Run** — FAIL.
- [ ] **Step 3: Implement.** `write_permitted` gains a `claimed_by: Option<&str>` (already filtered through `claim_live`) and refuses an agent whose `acting.id` differs. In `read_check_and_write`, when building the new meta: if the live claimant is `acting.id`, or the claim is expired, set both fields `None`; a human write leaves another's live claim as is. `next` treats a live claim by anyone as unavailable — placed alongside the `is_free` probe so "all remaining are claimed or held" lands in `busy_err()`, while "no open, sticky-unlocked cassettes" stays `Empty`. `next` takes no `--writer`, so it skips every live claim (an agent re-finding its own claim via `next` is not a case worth a flag). The spec's optional `next --claim` is **not** built this round (YAGNI; one agent per session is the current use) — record it in follow-up.md in Task 6.
- [ ] **Step 4: Run** `cargo test && cargo clippy --all-targets -- -D warnings` — PASS.
- [ ] **Step 5: Commit** `feat(queue): writes, next and --json honour claims`

---

## Phase 2 — what the TUI shows

### Task 4: Turn status resolver, store→cassette plumbing, glyph in the separator

**Files:**
- Create: `src/status.rs` (+ `mod status;` in `main.rs`)
- Modify: `src/cassette.rs` (two fields; hand-written `Default`)
- Modify: `src/session_writer.rs` (shared annotate helper; `refresh_from_disk`; `flush_held`; writer kind)
- Modify: `src/main.rs` (`load_session_cassettes` ~870, `sync_external_writes` ~1430, `resolve_tui_writer` returns kind, tick sets `app.now`)
- Modify: `src/app.rs` (`now: i64`; `merge_external` both branches + short circuit; `turn_status(idx)`)
- Modify: `src/theme.rs` (`status_human`/`status_agent`/`status_busy` on `Theme`, `ThemeSpec`, `apply_spec`, every builtin)
- Modify: `src/ui.rs` (`separator_pieces`, `render_separator`, call sites)
- Test: `src/status.rs`, `src/app.rs`, `src/session_writer.rs`, `src/ui.rs`, `src/theme.rs`

**Interfaces:**
- Consumes: `meta::claim_live` (Task 2); Task 1's `last_writer` invariant.
- Produces:
  - `Cassette.last_writer_human: Option<bool>` — `None` when unresolvable / no store
  - `Cassette.claimed_until: Option<i64>` — unix secs of a claim by **another** writer; `None` otherwise
  - `status.rs`:
    ```rust
    pub enum StatusSet { Turn }
    pub enum StatusId { Human, Agent, Busy }
    pub struct StatusContext { pub held: bool, pub now: i64 }
    impl StatusSet {
        pub fn name(self) -> &'static str;               // "turn"
        pub fn statuses(self) -> &'static [StatusId];
        pub fn resolve(self, c: &Cassette, cx: &StatusContext) -> Option<StatusId>; // None iff c.closed
    }
    impl StatusId {
        pub fn key(self) -> &'static str;                // "human" | "agent" | "busy"
        pub fn default_glyph(self) -> &'static str;      // "✎" | "⚙" | "●"
    }
    ```
  - `App::turn_status(&self, idx: usize) -> Option<StatusId>` — `None` when `self.session.is_empty()` (`-o` mode has no other writer; no glyphs) or the cassette is closed. `held` = `idx == focus_idx && read_only != ReadOnly::Closed` (covers both our own lock and `ReadOnly::Busy`).
  - `pub(crate) fn annotate_from_store(c: &mut Cassette, meta: &CassetteMeta, writers: &Writers, own_writer: &str, now: DateTime<Utc>)` in `session_writer.rs` — sets `locked_by`, `priority`, `closed`, `last_writer_human`, `claimed_until` (dropped when the claimant is `own_writer`). The three construction sites call it instead of setting fields inline.

- [ ] **Step 1: Failing resolver tests** in `status.rs`, one per rule, in precedence order:
  `held_is_busy_even_if_agent_wrote_last`, `live_claim_is_busy`, `expired_claim_falls_through`, `blank_is_human_even_if_human_wrote_last`, `whitespace_only_both_sides_is_human_with_agent_last_writer`, `human_last_writer_is_agent_turn`, `agent_last_writer_is_human_turn`, `unknown_last_writer_is_human_turn`, `closed_has_no_status`.
- [ ] **Step 2: Run** — FAIL. **Step 3: Implement** `status.rs` and the two `Cassette` fields (`Default`: `None`, `None`; `from_sides` leaves them `None`).
- [ ] **Step 4: Failing plumbing tests**
  - app.rs: `merge_external_carries_turn_fields_through_a_full_rebuild`; `merge_external_updates_turn_fields_in_the_no_op_short_circuit` (mirror the two `locked_by` tests at ~1361/~1377); `turn_status_is_none_without_a_session`; `focused_cassette_is_busy`.
  - session_writer.rs: `flush_sets_last_writer_human_from_tui_writer_kind`; `autosave_preserves_a_claim` (seed claim on disk, dirty + flush, re-read: claim intact); `annotate_drops_own_claim`.
- [ ] **Step 5: Implement plumbing.** `resolve_tui_writer` returns `(id, name, Kind)`; `SessionWriter` stores `writer_is_human: bool` and `flush_held` sets `app.cassettes[idx].last_writer_human = Some(writer_is_human)` after a successful write. Replace the inline field-setting at the three construction sites with `annotate_from_store`. In `refresh_from_disk`'s short circuit, apply `last_writer_human`/`claimed_until` in place like `locked_by`. `merge_external` treats them like `locked_by` (in place in the short circuit; copied in the rebuild). `main.rs`'s 1 s tick sets `app.now = Utc::now().timestamp()` before drawing; initialize at startup too.
- [ ] **Step 6: Failing render tests** in ui.rs: `separator_shows_status_glyph_after_side_tag` (`separator_text` for an agent-written unfocused cassette contains `"╡ SIDE A ╞ ⚙"` ordering: side tag, glyph, topic); `closed_separator_has_no_glyph`; `no_glyph_in_offline_mode`.
- [ ] **Step 7: Implement display.** `separator_pieces(cassette, short, glyph: Option<&str>)` inserts the glyph as piece index 1, wrapped as `" ✎ "` (spaces part of the piece, no `╡╞` frame — my call, keep it light). `render_separator` styles that piece with the status's theme color; compute `used` with `cassette::char_width` summed over chars for **every** piece (replacing `chars().count()`). Theme defaults: `Theme::default()` → human `Color::Cyan`, agent `Color::DarkGray`, busy `Color::Red`; for each builtin pick from its own palette (dracula cyan `0x8be9fd` / comment `0x6272a4` / red `0xff5555`; gruvbox aqua `0x8ec07c` / gray `0x928374` / red `0xfb4934`; nord `0x88c0d0` / `0x4c566a` / `0xbf616a`; solarized `0x2aa198` / `0x586e75` / `0xdc322f`). `ThemeSpec` gains `status_human`/`status_agent`/`status_busy`; `apply_spec` overrides them like the rest.
- [ ] **Step 8: Run** `cargo test && cargo clippy --all-targets -- -D warnings` — PASS.
- [ ] **Step 9: `/verify`** — a session with agent-written, human-written, empty-agent, empty-Ctrl+N cassettes shows ✎, ⚙, ✎, ✎ respectively; Tab in shows ●; Tab out without typing restores; type + Tab out shows ⚙; `queue claim --writer bot` on an unfocused cassette shows ● within ~1 s; `queue claim --for 3` reverts within ~1 s of expiry; `queue write --writer bot` turns it ✎.
- [ ] **Step 10: Commit** `feat(tui): turn status glyph in each cassette's separator`

### Task 5: Pulse and `[status.turn]` config

**Files:**
- Modify: `src/config.rs` (`status` field), `src/status.rs` (`Glyphs`, `glyphs_from_config`), `src/app.rs` (`anim_frame`), `src/main.rs` (load + exit 2, tick, pass glyphs to render), `src/ui.rs` (`render` takes `&Glyphs`; pulse style)
- Test: `src/status.rs`, `src/config.rs`, `src/ui.rs`

**Interfaces:**
- Consumes: Task 4's `StatusSet`, `StatusId`, `App::turn_status`.
- Produces:
  - `config::Config.status: HashMap<String, StatusSpec>` (`#[serde(default)]`), `pub struct StatusSpec { pub pulse: Option<bool>, #[serde(flatten)] pub glyphs: HashMap<String, String> }`
  - `pub struct Glyphs { /* per StatusId */ pub pulse: bool }` with `pub fn glyph(&self, id: StatusId) -> &str` and `Default` = built-in glyphs, `pulse: true`
  - `pub fn glyphs_from_config(specs: &HashMap<String, StatusSpec>) -> Result<Glyphs, String>`
  - `App.anim_frame: u64`, `App::tick_anim(&mut self)`
  - `ui::render(frame, app, theme, glyphs: &Glyphs)`

- [ ] **Step 1: Failing tests**
  - status.rs: `empty_config_gives_defaults`; `overrides_one_glyph_keeps_others`; `unknown_set_name_is_error` (`[status.trun]`, message names `turn`); `unknown_status_key_is_error` (`humna`, message lists `human, agent, busy`); `empty_glyph_is_error`; `pulse_false_is_honoured`.
  - config.rs: `parses_status_table` from a TOML string with `[status.turn]` incl. `pulse = false`; keep the existing unknown-top-level-key test passing.
  - ui.rs: `pulse_alternates_human_glyph_style` (same glyph text both frames; style differs between even/odd `anim_frame`); `pulse_off_keeps_style_constant`; `two_cell_glyph_does_not_overflow_separator` (render into a `TestBackend` of width 40 with glyph `"🤖"`, assert row width ≤ 40 and the fill ends at the edge).
- [ ] **Step 2: Run** — FAIL.
- [ ] **Step 3: Implement.** Validation walks the map: set name must equal a `StatusSet::name()`, each glyph key a `StatusId::key()` of that set, each value non-empty after trim. `main.rs` calls it right after the theme resolution at ~417 and exits 2 with `config: [status.turn] …` on `Err`, the same path as an unknown `--theme`. Pulse: the 1 s tick calls `app.tick_anim()`; in `render_separator` a `Human` glyph gets `BOLD` on even frames and `DIM` on odd frames when `glyphs.pulse`, otherwise always `BOLD`. Other statuses never pulse.
- [ ] **Step 4: Run** `cargo test && cargo clippy --all-targets -- -D warnings` — PASS.
- [ ] **Step 5: `/verify`** — the ✎ glyph visibly alternates across two captures a second apart; with `pulse = false` and `agent = "🤖"` in a temp `XDG_CONFIG_HOME` config, the glyph changes, doesn't pulse, and the separator fill is intact.
- [ ] **Step 6: Commit** `feat(tui): pulsing human-turn glyph and [status.turn] config`

---

## Phase 3 — agent skill and docs

### Task 6: Skill, CLAUDE.md, README, follow-up

**Files:**
- Modify: `.claude/skills/cassette-session/SKILL.md`, `CLAUDE.md`, `README.md`, `docs/follow-up.md`

- [ ] **Step 1: SKILL.md** — in "What to act on": claim before reading/thinking (`queue claim <id> --session <ID> --writer bot [--for N]`), `queue write` releases it, `queue release` if you decide not to answer; pick `--for` longer than your think time (expiry is a crash net); never work a cassette whose `claimed_by` is someone else. Add the glyph table so the agent can refer to what the human sees. Note that `waiting_on` now only changes on writes (moves, locks, closes no longer flip it).
- [ ] **Step 2: CLAUDE.md** — command list gains `queue claim`/`queue release`; add a `src/status.rs` paragraph (StatusSet, derived-not-stored, precedence); `cassette.rs` paragraph mentions the two new scalars; `meta.rs`/`session_writer.rs` ownership notes add `claimed_by`/`claimed_until` to the queue-owned fields; `edit.rs` paragraph records the "only turns stamp `last_writer`" rule; `config.rs` paragraph documents `[status.turn]`; `ui.rs` paragraph mentions the glyph piece and `char_width` fill.
- [ ] **Step 3: README** — a "Cassette status" section with the glyph table, the `[status.turn]` example from the spec, and one line on claims.
- [ ] **Step 4: follow-up.md** — three entries: manual status sets (CLI + TUI); the stale-read overwrite hazard (agent write landing after the human added words); second-TUI busy detection on unfocused cassettes.
- [ ] **Step 5: Verify** `cargo test` still green (docs only; no `/verify` needed). Mark the spec's acceptance list as checked in the final report, noting anything unverified.
- [ ] **Step 6: Commit** `docs: turn status, claims, and the status config`
