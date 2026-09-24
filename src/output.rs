//! The store-shaped cassette body.
//!
//! This module used to hold the flat-note writer — `write_markdown`,
//! `parse_markdown`, the append/draft/conflict-rename machinery — which
//! Phase 5a moved the TUI off and Phase 6 deleted. What remains is the one
//! item that ever had a live caller.

use crate::cassette::Cassette;

/// A store cassette's body: `## Side A` always, `## Side B` only when it has
/// text, and **no `# Cassette N` heading**.
///
/// The heading `build_body` writes below belongs to the old one-file-per-
/// session format, where it was the only thing separating one cassette from
/// the next. Per file it is redundant, and it is actively harmful:
/// `json::split_sides` folds everything before the first `## Side A` into
/// `side_a`, so a stray wrapper would surface inside the JSON contract an
/// agent reads.
///
/// Delegates to `queue::write::build_body` rather than re-deriving the
/// shape: `queue write` and the TUI write the same files, and a cassette's
/// body must not depend on which of the two touched it last.
pub fn cassette_body(c: &Cassette) -> String {
    crate::queue::write::build_body(&c.side_a_text(), &c.side_b_text())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_store_cassette_body_carries_sides_and_no_cassette_heading() {
        // The `# Cassette N` wrapper is redundant per-file AND harmful:
        // json::split_sides folds pre-heading text into side_a, so a wrapper
        // would surface inside the JSON contract agents read.
        let c = Cassette::from_sides(
            "front words\n".to_string(),
            String::new(),
            Some("topic".into()),
        );
        let body = cassette_body(&c);
        assert!(
            body.starts_with("## Side A"),
            "no preamble before the heading: {body:?}"
        );
        assert!(
            !body.contains("# Cassette"),
            "no per-cassette wrapper: {body:?}"
        );
        assert!(
            !body.contains("## Side B"),
            "side B is empty, so no heading: {body:?}"
        );

        // And it round-trips through the contract's own splitter.
        let (a, b) = crate::queue::json::split_sides(&body);
        assert_eq!(a.trim(), "front words");
        assert_eq!(b, "");
    }

    #[test]
    fn a_store_cassette_body_includes_side_b_when_it_has_text() {
        let c = Cassette::from_sides("front\n".to_string(), "back\n".to_string(), None);
        let body = cassette_body(&c);
        let (a, b) = crate::queue::json::split_sides(&body);
        assert_eq!(a.trim(), "front");
        assert_eq!(b.trim(), "back");
    }

    #[test]
    fn a_store_cassette_body_is_byte_identical_to_what_queue_write_emits() {
        // Two writers touch the same cassette files. If they disagree by so
        // much as a blank line, a body's shape depends on which one wrote it
        // last — and every diff of a human-then-agent session shows churn
        // that is not text. Pinned against `queue write`'s own builder, over
        // the shapes that differ: trailing newlines, an empty side B, and a
        // side B that has to be re-emitted.
        for (a, b) in [
            ("front words", ""),
            ("front words\n", ""),
            ("front\n\n", "back\n\n"),
            ("", "only the back side"),
            ("", ""),
        ] {
            let c = Cassette::from_sides(a.to_string(), b.to_string(), Some("topic".into()));
            assert_eq!(
                cassette_body(&c),
                crate::queue::write::build_body(a, b),
                "diverged for sides {a:?} / {b:?}"
            );
        }
    }
}
