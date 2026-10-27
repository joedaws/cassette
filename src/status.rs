//! Cassette statuses: a small, derived label per open cassette, shown as a
//! glyph in its separator.
//!
//! A **status set** is one way of labelling cassettes. There is exactly one
//! today, `Turn` — whose move it is — but the shape is a set of sets on
//! purpose: a later manual set will store its own frontmatter field and
//! resolve from that, and nothing here should assume `Turn` is the only one.
//! That is why the resolver is a method on the set rather than a free
//! function. The names config spells sets and statuses with (`name`,
//! `statuses`, `key`) arrive with `[status.<set>]` config, their first
//! caller — landing them before it would be dead code.
//!
//! Pure, like `App` and `Cassette`: no I/O, no store types, no ratatui. What
//! the resolver needs from the store has already been folded onto the
//! `Cassette` as plain scalars (`last_writer_human`, `claimed_until`) by
//! whoever built it, and what is not on the cassette at all — whether this
//! process holds it, and the time — arrives in `StatusContext`. `ui.rs` only
//! maps the resulting `StatusId` to a glyph and a theme color.

use crate::cassette::Cassette;

/// A family of statuses with its own resolution rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusSet {
    /// Whose move it is: the human's, an agent's, or nobody's right now
    /// because someone is in the middle of one.
    Turn,
}

/// One status within a set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusId {
    /// The human's move.
    Human,
    /// An agent's move.
    Agent,
    /// Someone is on it right now — hands off.
    Busy,
}

/// What the resolver needs that is not on the cassette.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatusContext {
    /// This cassette's lock is held while the TUI looks at it: either this
    /// process holds it (the focused cassette) or the TUI already knows
    /// another writer does (the focused cassette opened read-only `Busy`).
    pub held: bool,
    /// Unix seconds, for claim expiry. Passed in rather than read here, so
    /// the resolver stays pure and a claim lapses on the tick after
    /// `claimed_until` with no file change needed.
    pub now: i64,
}

impl StatusSet {
    /// The status of `c`, or `None` for a closed cassette — closed is
    /// history, not work, and has no move left to make.
    pub fn resolve(self, c: &Cassette, cx: &StatusContext) -> Option<StatusId> {
        if c.closed {
            return None;
        }
        match self {
            StatusSet::Turn => Some(resolve_turn(c, cx)),
        }
    }
}

/// The turn set's rules, in precedence order (spec, "How the status is
/// worked out"). Derived every time, never stored.
fn resolve_turn(c: &Cassette, cx: &StatusContext) -> StatusId {
    // 1. Someone is in it. Held beats everything: the human's focused
    //    cassette is exactly what an agent sees as `busy: true`.
    if cx.held {
        return StatusId::Busy;
    }
    // A live claim by another writer. `claimed_until` is only ever set for
    // a claim that is not the TUI's own (`annotate_from_store` drops that
    // one), so "is it live" is the whole question here.
    if c.claimed_until.is_some_and(|until| cx.now < until) {
        return StatusId::Busy;
    }
    // 2. Blank is a prompt waiting for words, whoever made it. Without this
    //    every cassette the human creates (which stamps them `last_writer`)
    //    would read as the agent's move before a character was typed.
    if c.side_a_text().trim().is_empty() && c.side_b_text().trim().is_empty() {
        return StatusId::Human;
    }
    // 3. A human ended the last turn, so it is an agent's move.
    if c.last_writer_human == Some(true) {
        return StatusId::Agent;
    }
    // 4. An agent wrote last — or nobody can tell who did. When in doubt,
    //    show it as the human's move rather than hide it.
    StatusId::Human
}

impl StatusId {
    /// The glyph shown when config does not override it.
    pub fn default_glyph(self) -> &'static str {
        match self {
            StatusId::Human => "✎",
            StatusId::Agent => "⚙",
            StatusId::Busy => "●",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn written(last_writer_human: Option<bool>) -> Cassette {
        let mut c = Cassette::from_sides("some words".to_string(), String::new(), None);
        c.last_writer_human = last_writer_human;
        c
    }

    fn cx(held: bool) -> StatusContext {
        StatusContext { held, now: NOW }
    }

    fn turn(c: &Cassette, cx: &StatusContext) -> Option<StatusId> {
        StatusSet::Turn.resolve(c, cx)
    }

    #[test]
    fn held_is_busy_even_if_agent_wrote_last() {
        assert_eq!(turn(&written(Some(false)), &cx(true)), Some(StatusId::Busy));
    }

    #[test]
    fn live_claim_is_busy() {
        let mut c = written(Some(true));
        c.claimed_until = Some(NOW + 60);
        assert_eq!(turn(&c, &cx(false)), Some(StatusId::Busy));
    }

    #[test]
    fn expired_claim_falls_through() {
        // Expiry is evaluated against `now`, not swept off disk: a claim
        // whose moment has passed (including exactly now) is no claim, and
        // the cassette shows its derived turn again.
        let mut c = written(Some(true));
        c.claimed_until = Some(NOW);
        assert_eq!(turn(&c, &cx(false)), Some(StatusId::Agent));
    }

    #[test]
    fn blank_is_human_even_if_human_wrote_last() {
        let mut c = Cassette::new();
        c.last_writer_human = Some(true);
        assert_eq!(turn(&c, &cx(false)), Some(StatusId::Human));
    }

    #[test]
    fn whitespace_only_both_sides_is_human_with_agent_last_writer() {
        let mut c = Cassette::from_sides("  \n ".to_string(), "\t\n".to_string(), None);
        c.last_writer_human = Some(false);
        assert_eq!(turn(&c, &cx(false)), Some(StatusId::Human));
    }

    #[test]
    fn human_last_writer_is_agent_turn() {
        assert_eq!(
            turn(&written(Some(true)), &cx(false)),
            Some(StatusId::Agent)
        );
    }

    #[test]
    fn agent_last_writer_is_human_turn() {
        assert_eq!(
            turn(&written(Some(false)), &cx(false)),
            Some(StatusId::Human)
        );
    }

    #[test]
    fn unknown_last_writer_is_human_turn() {
        assert_eq!(turn(&written(None), &cx(false)), Some(StatusId::Human));
    }

    #[test]
    fn closed_has_no_status() {
        // Not even when held: a closed cassette has no move left to make.
        let mut c = written(Some(true));
        c.closed = true;
        assert_eq!(turn(&c, &cx(true)), None);
        assert_eq!(turn(&c, &cx(false)), None);
    }

    #[test]
    fn default_glyphs() {
        let glyphs =
            [StatusId::Human, StatusId::Agent, StatusId::Busy].map(StatusId::default_glyph);
        assert_eq!(glyphs, ["✎", "⚙", "●"]);
    }
}
