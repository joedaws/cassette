//! Queue ordering and sparse priority insertion.
//!
//! Priorities are sparse (10, 20, 30 …) so most insertions are a single
//! frontmatter edit rather than a renumbering of the whole queue. `first` and
//! `between` return `None` when a run has no integer gap left; the caller
//! renumbers just that run, taking just those locks. There is no
//! normalize-on-open pass — it would rewrite every cassette.

use crate::store::meta::{CassetteMeta, Status};

/// The gap left between adjacent priorities.
pub const STEP: i64 = 10;

/// Tail placement — the default for a new cassette. An agent adding work
/// cannot jump the human's line. `None` when there is no room left above the
/// maximum, which means this run must be renumbered.
///
/// Returns `Option` rather than a bare `i64` so an unrepresentable result is a
/// value the caller must handle, not a debug-build panic. Priorities come from
/// frontmatter, which `parse_frontmatter` will accept as any i64-parseable
/// string, so `i64::MAX` is reachable by hand-editing a file.
pub fn last(existing: &[i64]) -> Option<i64> {
    existing
        .iter()
        .copied()
        .max()
        .unwrap_or(0)
        .checked_add(STEP)
}

/// Head placement. `min - STEP` normally; when that would reach zero, half
/// the minimum instead. `None` when even that leaves no room.
pub fn first(existing: &[i64]) -> Option<i64> {
    let Some(min) = existing.iter().copied().min() else {
        return Some(STEP);
    };
    if min - STEP > 0 {
        return Some(min - STEP);
    }
    let halved = min / 2;
    (halved > 0).then_some(halved)
}

/// Midpoint of two neighbours. `None` when they are adjacent or equal, when
/// they are reversed, or when the span between them is not representable —
/// all of which mean the same thing to a caller: this run must be renumbered.
pub fn between(lo: i64, hi: i64) -> Option<i64> {
    let span = hi.checked_sub(lo)?;
    let mid = lo.checked_add(span / 2)?;
    (mid > lo && mid < hi).then_some(mid)
}

/// Fresh sparse priorities for a run that ran out of gaps.
pub fn renumber(count: usize) -> Vec<i64> {
    (1..=count as i64).map(|i| i * STEP).collect()
}

/// Queue order: open cassettes by priority, closed ones last, ties by id.
///
/// The id tiebreak is for stability, not truth — two cassettes minted in the
/// same millisecond sort arbitrarily with respect to each other, but they
/// sort the *same way* on every read, so the queue does not jitter.
pub fn queue_order(metas: &mut [CassetteMeta]) {
    metas.sort_by(|a, b| {
        let closed = |m: &CassetteMeta| m.status == Status::Closed;
        closed(a)
            .cmp(&closed(b))
            .then(a.priority.cmp(&b.priority))
            .then_with(|| a.id.cmp(&b.id))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::meta::{CassetteMeta, Status};

    fn m(id: &str, priority: i64, status: Status) -> CassetteMeta {
        CassetteMeta {
            id: id.to_string(),
            topic: None,
            priority,
            status,
            locked_by: None,
            created_by: String::new(),
            last_writer: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn tail_placement_is_max_plus_a_step() {
        assert_eq!(last(&[10, 20, 30]), Some(40));
        // Out-of-order input must still append after the true maximum.
        assert_eq!(last(&[30, 10, 20]), Some(40));
    }

    #[test]
    fn tail_placement_on_an_empty_queue_is_the_first_step() {
        assert_eq!(last(&[]), Some(STEP));
    }

    #[test]
    fn tail_placement_refuses_to_overflow() {
        // Reachable from a hand-edited frontmatter priority: `parse_frontmatter`
        // accepts any i64-parseable string. A debug build panicked here.
        assert_eq!(last(&[i64::MAX]), None, "no room above i64::MAX");
        assert_eq!(last(&[i64::MAX - 1]), None, "nor within one STEP of it");
        assert_eq!(
            last(&[i64::MAX - STEP]),
            Some(i64::MAX),
            "exactly one step fits"
        );
    }

    #[test]
    fn head_placement_is_min_minus_a_step() {
        assert_eq!(first(&[20, 30]), Some(10));
        assert_eq!(first(&[]), Some(STEP));
    }

    #[test]
    fn head_placement_halves_instead_of_reaching_zero() {
        // min - STEP would be <= 0, so halve the minimum instead.
        assert_eq!(first(&[10]), Some(5));
        assert_eq!(first(&[4]), Some(2));
        assert_eq!(first(&[3]), Some(1));
    }

    #[test]
    fn head_placement_gives_up_when_there_is_no_room_below() {
        assert_eq!(first(&[1]), None, "nothing fits below 1");
    }

    #[test]
    fn between_takes_the_midpoint() {
        assert_eq!(between(10, 20), Some(15));
        assert_eq!(between(15, 20), Some(17));
    }

    #[test]
    fn between_gives_up_on_adjacent_values() {
        assert_eq!(between(15, 16), None);
        assert_eq!(between(15, 15), None);
    }

    #[test]
    fn between_refuses_to_overflow() {
        // `hi - lo` overflows before the midpoint guard ever runs.
        assert_eq!(
            between(i64::MIN, i64::MAX),
            None,
            "the span is not representable"
        );
        // Also unrepresentable: `0 - i64::MIN` is `i64::MAX + 1`. Returning
        // None here is correct — it reads as "renumber this run", and real
        // priorities are positive by construction anyway.
        assert_eq!(between(i64::MIN, 0), None, "this span overflows too");
        // A span that IS representable must still produce a midpoint.
        assert_eq!(
            between(-10, 10),
            Some(0),
            "a representable span still works"
        );
    }

    #[test]
    fn renumber_produces_a_sparse_run() {
        assert_eq!(renumber(3), vec![10, 20, 30]);
        assert_eq!(renumber(0), Vec::<i64>::new());
    }

    #[test]
    fn queue_order_puts_open_before_closed() {
        let mut v = vec![m("b", 10, Status::Closed), m("a", 20, Status::Open)];
        queue_order(&mut v);
        assert_eq!(
            v.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            vec!["a", "b"],
            "an open cassette outranks a closed one with a better priority"
        );
    }

    #[test]
    fn queue_order_sorts_by_priority_then_id() {
        let mut v = vec![
            m("z", 20, Status::Open),
            m("a", 20, Status::Open),
            m("m", 10, Status::Open),
        ];
        queue_order(&mut v);
        assert_eq!(
            v.iter().map(|c| c.id.as_str()).collect::<Vec<_>>(),
            vec!["m", "a", "z"],
            "priority first; the id only settles the tie"
        );
    }

    #[test]
    fn queue_order_is_stable_across_repeated_calls() {
        // The queue must not jitter between reads.
        let mut v = vec![m("b", 10, Status::Open), m("a", 10, Status::Open)];
        queue_order(&mut v);
        let once: Vec<String> = v.iter().map(|c| c.id.clone()).collect();
        queue_order(&mut v);
        let twice: Vec<String> = v.iter().map(|c| c.id.clone()).collect();
        assert_eq!(once, twice);
    }
}
