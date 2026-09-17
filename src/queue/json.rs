//! The `--json` contract types and the body splitter.
//!
//! **Pure data. No `Store`, no `std::fs`, no locking.** That is what makes
//! the frozen wire shape testable without building a store — a reviewer can
//! confirm the invariant by reading this file's imports alone. Building a
//! `Listing` from a live store is a later task's job; this module only
//! knows how to shape and serialize one.
//!
//! `status`, `kind`, and `waiting_on` are frozen wire spellings
//! (`"open"`/`"closed"`, `"human"`/`"agent"`) rather than derived from
//! `store::meta::Status` or `store::writers::Kind` via `serde`'s enum
//! serialization: a later rename of either Rust type must not silently
//! change what an agent parses. `&'static str` makes that impossible by
//! construction.

use serde::Serialize;

/// The session a `Listing` belongs to.
#[derive(Debug, Clone, Serialize)]
pub struct SessionRef {
    pub id: String,
    pub alias: Option<String>,
}

/// A writer, as attributed on a cassette (`created_by`, `last_writer`) or
/// holding its sticky lock. `kind` is the frozen wire spelling — see the
/// module doc.
#[derive(Debug, Clone, Serialize)]
pub struct WriterRef {
    pub name: String,
    pub kind: &'static str,
}

/// One cassette, shaped for `--json`.
#[derive(Debug, Clone, Serialize)]
pub struct CassetteView {
    pub id: String,
    pub topic: Option<String>,
    pub priority: i64,
    pub status: &'static str,
    pub words: usize,
    pub busy: bool,
    pub sticky_lock: Option<WriterRef>,
    pub created_by: Option<WriterRef>,
    pub last_writer: Option<WriterRef>,
    pub waiting_on: Option<&'static str>,
    pub updated_at: String,
    pub side_a: String,
    pub side_b: String,
}

/// The top-level `--json` payload for `queue list`.
#[derive(Debug, Clone, Serialize)]
pub struct Listing {
    pub session: SessionRef,
    pub cassettes: Vec<CassetteView>,
}

/// Split a cassette body into `(side_a, side_b)`.
///
/// Looks for lines equal to `## Side A` and `## Side B` after trimming
/// trailing whitespace — the headings `output::build_body` writes (always
/// `## Side A`, and `## Side B` only when side B is non-empty). Text before
/// the first `## Side A` heading, when headings exist, belongs to nothing —
/// `build_body` never emits any — but is folded into `side_a` rather than
/// dropped, matching the "no headings at all" case below.
///
/// When no `## Side A` heading appears anywhere, the whole body is
/// `side_a` and `side_b` is empty. That is the common case today: nothing
/// in the store writes sides yet, `queue write` replaces the whole body
/// with flat text, and this must not lose it.
pub fn split_sides(body: &str) -> (String, String) {
    const HEADING_A: &str = "## Side A";
    const HEADING_B: &str = "## Side B";

    let lines: Vec<&str> = body.lines().collect();

    let Some(a_start) = lines.iter().position(|l| l.trim_end() == HEADING_A) else {
        return (body.to_string(), String::new());
    };

    // Any preamble before the heading is kept, not dropped — it is not part
    // of `build_body`'s output today, but this function must never be the
    // reason text goes missing from an agent's view of a cassette.
    let preamble = &lines[..a_start];
    // Skip the heading line itself, then find where Side B (if any) begins
    // within what follows.
    let after_a = &lines[a_start + 1..];

    let (side_a_lines, side_b_lines): (&[&str], &[&str]) =
        match after_a.iter().position(|l| l.trim_end() == HEADING_B) {
            Some(b_start) => (&after_a[..b_start], &after_a[b_start + 1..]),
            None => (after_a, &[]),
        };

    let side_a = preamble
        .iter()
        .chain(side_a_lines.iter())
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    let side_b = side_b_lines.join("\n");

    (side_a, side_b)
}

/// Word count over both sides — `split_whitespace().count()` summed —
/// matching `Cassette::word_count` in `src/cassette.rs` exactly. The TUI and
/// the JSON contract must never disagree about how long a cassette is.
pub fn count_words(side_a: &str, side_b: &str) -> usize {
    side_a.split_whitespace().count() + side_b.split_whitespace().count()
}

/// Whose turn it is, given who wrote last: the inverse kind. `None` when the
/// last writer cannot be resolved to a `WriterRef` at all — a writer id in a
/// cassette's frontmatter that `writers.toml` does not know, i.e. a damaged
/// store. Emitting `null` rather than guessing a turn is deliberate: telling
/// an agent it is up when nobody knows whose turn it is would be worse than
/// saying nothing.
pub fn waiting_on(last_writer: Option<&WriterRef>) -> Option<&'static str> {
    match last_writer?.kind {
        "human" => Some("agent"),
        "agent" => Some("human"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_body_with_side_headings_splits_on_them() {
        let (a, b) = split_sides("## Side A\n\nfront words\n\n## Side B\n\nback words\n");
        assert_eq!(a.trim(), "front words");
        assert_eq!(b.trim(), "back words");
    }

    #[test]
    fn a_body_with_no_headings_is_all_side_a() {
        // Nothing writes sides yet — `queue write` replaces the whole body
        // with flat text — so this is the common case today, and the one
        // that must not silently lose the text.
        let (a, b) = split_sides("just prose an agent wrote\n");
        assert_eq!(a.trim(), "just prose an agent wrote");
        assert_eq!(b, "");
    }

    #[test]
    fn text_before_the_first_heading_survives_in_side_a() {
        // Not a shape `build_body` ever writes, but this function must never
        // be the reason text disappears from an agent's view of a cassette:
        // silent truncation here is indistinguishable from the human never
        // having written the words.
        let (a, b) = split_sides("stray preamble\n## Side A\n\nwords\n");
        assert!(a.contains("stray preamble"), "{a}");
        assert!(a.contains("words"), "{a}");
        assert_eq!(b, "");
    }

    #[test]
    fn side_a_only_leaves_side_b_empty() {
        let (a, b) = split_sides("## Side A\n\nwords\n");
        assert_eq!(a.trim(), "words");
        assert_eq!(b, "");
    }

    #[test]
    fn words_counts_both_sides() {
        // Matches Cassette::word_count in src/cassette.rs, which is
        // split_whitespace().count() over both sides. The TUI and the JSON
        // must never disagree about how long a cassette is.
        assert_eq!(count_words("one two", "three"), 3);
        assert_eq!(count_words("", ""), 0);
        assert_eq!(count_words("  spaced   out  ", ""), 2);
    }

    #[test]
    fn waiting_on_is_the_inverse_of_who_wrote_last() {
        let human = WriterRef {
            name: "joseph".into(),
            kind: "human",
        };
        let agent = WriterRef {
            name: "bot".into(),
            kind: "agent",
        };
        assert_eq!(waiting_on(Some(&human)), Some("agent"));
        assert_eq!(waiting_on(Some(&agent)), Some("human"));
        // An unresolvable writer yields null rather than a guess: a writer
        // missing from writers.toml is a damaged store, and inventing a turn
        // would tell an agent it is up when nobody knows whose turn it is.
        assert_eq!(waiting_on(None), None);
    }

    #[test]
    fn a_cassette_serializes_to_the_contract_shape() {
        let v = CassetteView {
            id: "01K5GR7T2M9WPD0000000000AB".into(),
            topic: Some("refactor notes".into()),
            priority: 10,
            status: "open",
            words: 2,
            busy: false,
            sticky_lock: None,
            created_by: Some(WriterRef {
                name: "joseph".into(),
                kind: "human",
            }),
            last_writer: Some(WriterRef {
                name: "joseph".into(),
                kind: "human",
            }),
            waiting_on: Some("agent"),
            updated_at: "2026-09-13T14:02:11Z".into(),
            side_a: "two words".into(),
            side_b: String::new(),
        };
        let j: serde_json::Value = serde_json::to_value(&v).expect("serialize");
        assert_eq!(j["id"], "01K5GR7T2M9WPD0000000000AB");
        assert_eq!(j["status"], "open");
        assert_eq!(j["sticky_lock"], serde_json::Value::Null);
        assert_eq!(j["created_by"]["kind"], "human");
        assert_eq!(j["waiting_on"], "agent");
        assert_eq!(j["side_b"], "");
    }

    #[test]
    fn prose_with_quotes_and_newlines_round_trips() {
        // The reason serde_json is a dependency rather than hand-rolled
        // escaping: bodies are arbitrary user prose.
        let body = "she said \"no\"\\ever\n\ttabbed\n";
        let (a, _) = split_sides(body);
        let j = serde_json::to_string(&a).expect("serialize");
        let back: String = serde_json::from_str(&j).expect("round trip");
        assert_eq!(back, a);
    }
}
