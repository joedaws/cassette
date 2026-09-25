//! A cassette file's YAML frontmatter.
//!
//! Hand-rolled rather than serde_yaml: the repo already hand-parses
//! frontmatter in `output.rs` and `find.rs`, the field set is fixed and
//! small, and it keeps a YAML crate out of the dependency tree.

/// Whether a cassette is still in the queue or has been retired to the
/// collapsed closed row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Open,
    Closed,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Open => "open",
            Status::Closed => "closed",
        }
    }

    /// Anything unrecognized reads as `Open`: a hand-edited typo should leave
    /// the cassette visible in the queue rather than silently hiding it.
    pub fn parse(s: &str) -> Status {
        match s.trim() {
            "closed" => Status::Closed,
            _ => Status::Open,
        }
    }
}

/// Everything about a cassette except its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CassetteMeta {
    pub id: String,
    /// Display name, and the source of truth for it — the file's slug is
    /// frozen at creation and may disagree after a retopic.
    pub topic: Option<String>,
    /// Sparse queue position (10, 20, 30 …). The only ordering mechanism.
    pub priority: i64,
    pub status: Status,
    /// Sticky lock: the writer id holding it, or `None`.
    pub locked_by: Option<String>,
    pub created_by: String,
    pub last_writer: String,
    /// RFC3339 UTC, seconds precision.
    pub updated_at: String,
}

/// Frontmatter is line-oriented: a value containing a line break would end the
/// block early, truncating later fields and letting a topic inject ones it has
/// no business setting. Collapse every line break into a space.
///
/// `pub(crate)` because `lock::Attribution::render` reuses it: the anchor
/// file is line-oriented in exactly the same way, and `name` is free text
/// from `$USER`.
pub(crate) fn one_line(value: &str) -> String {
    value.replace("\r\n", " ").replace(['\n', '\r'], " ")
}

/// `---` block in the spec's field order, always ending with a newline so a
/// body can be concatenated straight onto it.
pub fn build_frontmatter(m: &CassetteMeta) -> String {
    let mut s = String::with_capacity(256);
    s.push_str("---\n");
    s.push_str(&format!("id: {}\n", one_line(&m.id)));
    if let Some(topic) = &m.topic {
        s.push_str(&format!("topic: {}\n", one_line(topic)));
    } else {
        s.push_str("topic:\n");
    }
    s.push_str(&format!("priority: {}\n", m.priority));
    s.push_str(&format!("status: {}\n", m.status.as_str()));
    s.push_str(&format!(
        "locked_by:{}\n",
        m.locked_by
            .as_deref()
            .map(|w| format!(" {}", one_line(w)))
            .unwrap_or_default()
    ));
    s.push_str(&format!("created_by: {}\n", one_line(&m.created_by)));
    s.push_str(&format!("last_writer: {}\n", one_line(&m.last_writer)));
    s.push_str(&format!("updated_at: {}\n", one_line(&m.updated_at)));
    s.push_str("---\n");
    s
}

/// Locate the frontmatter block and the body. `None` when the file has no
/// well-formed frontmatter: the opening fence must be the first line, and the
/// closing fence must be a line of exactly `---`.
///
/// One finder for both callers. When these were two searches with different
/// patterns, a malformed closing fence (a trailing space, say) parsed as valid
/// frontmatter but yielded an empty body — so a cassette's words were silently
/// discarded instead of the file being skipped.
fn split_parts(content: &str) -> Option<(&str, &str)> {
    let rest = content.strip_prefix("---\n")?;
    let (block, after) = match rest.split_once("\n---\n") {
        Some((block, after)) => (block, after),
        // A file that ends immediately after the closing fence.
        None => (rest.strip_suffix("\n---")?, ""),
    };
    // Writers put one blank line between the block and the body.
    Some((block, after.strip_prefix('\n').unwrap_or(after)))
}

/// Parse the fields out of an already-located frontmatter block. `None` when
/// `id` is missing — an unidentifiable file must not become a cassette with an
/// empty id that collides with the next one.
fn parse_block(block: &str) -> Option<CassetteMeta> {
    let mut id = None;
    let mut topic = None;
    let mut priority = 0i64;
    let mut status = Status::Open;
    let mut locked_by = None;
    let mut created_by = String::new();
    let mut last_writer = String::new();
    let mut updated_at = String::new();

    for line in block.lines() {
        // Split on the FIRST colon only: topics are free text and may contain
        // their own.
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            // A cassette's own id must say it is one: a bare ULID or another
            // kind's id is frontmatter this store did not write.
            "id" => {
                id = crate::store::ids::check(crate::store::ids::IdKind::Cassette, value)
                    .is_ok()
                    .then(|| value.to_string())
            }
            "topic" => topic = (!value.is_empty()).then(|| value.to_string()),
            "priority" => priority = value.parse().unwrap_or(0),
            "status" => status = Status::parse(value),
            "locked_by" => locked_by = (!value.is_empty()).then(|| value.to_string()),
            "created_by" => created_by = value.to_string(),
            "last_writer" => last_writer = value.to_string(),
            "updated_at" => updated_at = value.to_string(),
            _ => {}
        }
    }

    // Every writer reference must say it is one. A missing or bare value is
    // frontmatter this store did not write, and would otherwise resolve to
    // nobody — or, for `locked_by`, to a claim no writer can clear by name.
    let is_writer =
        |v: &str| crate::store::ids::check(crate::store::ids::IdKind::Writer, v).is_ok();
    if !is_writer(&created_by)
        || !is_writer(&last_writer)
        || locked_by.as_deref().is_some_and(|v| !is_writer(v))
    {
        return None;
    }

    Some(CassetteMeta {
        id: id?,
        topic,
        priority,
        status,
        locked_by,
        created_by,
        last_writer,
        updated_at,
    })
}

/// Frontmatter plus the body after it, with the body byte-for-byte intact.
/// `build_frontmatter` ends in `---\n` and writers add one blank line, so that
/// blank line is consumed here and re-added on write. When the frontmatter is
/// malformed or missing, the whole content is returned as body rather than
/// risking an empty one.
pub fn split(content: &str) -> (Option<CassetteMeta>, &str) {
    let Some((block, body)) = split_parts(content) else {
        return (None, content);
    };
    let Some(meta) = parse_block(block) else {
        return (None, content);
    };
    (Some(meta), body)
}

/// `2026-09-13T14:02:11Z` — RFC3339, UTC, seconds precision.
pub fn now_utc() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta() -> CassetteMeta {
        CassetteMeta {
            id: "cas_01K5GR7T2M9WPD0000000000AB".to_string(),
            topic: Some("gratitude".to_string()),
            priority: 20,
            status: Status::Open,
            locked_by: None,
            created_by: crate::store::ids::TEST_WRITER.to_string(),
            last_writer: crate::store::ids::TEST_WRITER.to_string(),
            updated_at: "2026-09-13T14:02:11Z".to_string(),
        }
    }

    #[test]
    fn frontmatter_round_trips() {
        let m = meta();
        let parsed = split(&build_frontmatter(&m)).0.expect("parses");
        assert_eq!(parsed, m);
    }

    #[test]
    fn frontmatter_is_delimited_and_ordered() {
        let text = build_frontmatter(&meta());
        assert!(text.starts_with("---\n"), "{text}");
        assert!(text.ends_with("---\n"), "{text}");
        assert!(
            text.contains("\nid: cas_01K5GR7T2M9WPD0000000000AB\n"),
            "{text}"
        );
        assert!(text.contains("\npriority: 20\n"), "{text}");
        assert!(text.contains("\nstatus: open\n"), "{text}");
    }

    #[test]
    fn an_empty_lock_round_trips_as_none() {
        // The spec writes `locked_by:` with nothing after it when unlocked.
        let m = meta();
        let text = build_frontmatter(&m);
        assert!(text.contains("\nlocked_by:\n"), "{text}");
        assert_eq!(split(&text).0.unwrap().locked_by, None);
    }

    #[test]
    fn a_held_lock_round_trips() {
        let mut m = meta();
        m.locked_by = Some("wri_01K5GQ00000000000000000002".to_string());
        let parsed = split(&build_frontmatter(&m)).0.unwrap();
        assert_eq!(
            parsed.locked_by.as_deref(),
            Some("wri_01K5GQ00000000000000000002")
        );
    }

    #[test]
    fn a_missing_topic_round_trips_as_none() {
        let mut m = meta();
        m.topic = None;
        let parsed = split(&build_frontmatter(&m)).0.unwrap();
        assert_eq!(parsed.topic, None);
    }

    #[test]
    fn a_topic_with_a_colon_survives() {
        // Topics are free user text; everything after the first `topic:` is
        // the value, so an embedded colon must not truncate it.
        let mut m = meta();
        m.topic = Some("re: yesterday".to_string());
        let parsed = split(&build_frontmatter(&m)).0.unwrap();
        assert_eq!(parsed.topic.as_deref(), Some("re: yesterday"));
    }

    #[test]
    fn closed_status_round_trips_and_unknown_reads_open() {
        let mut m = meta();
        m.status = Status::Closed;
        assert_eq!(
            split(&build_frontmatter(&m)).0.unwrap().status,
            Status::Closed
        );
        // A hand-edited file must not vanish from the queue.
        assert_eq!(Status::parse("nonsense"), Status::Open);
    }

    #[test]
    fn split_returns_meta_and_the_untouched_body() {
        let body = "## Side A\n\nhello\n\n## Side B\n\nscratch\n";
        let content = format!("{}\n{}", build_frontmatter(&meta()), body);
        let (parsed, rest) = split(&content);
        assert_eq!(parsed.unwrap().id, meta().id);
        assert_eq!(rest, body, "body must survive byte-for-byte");
    }

    #[test]
    fn content_without_frontmatter_is_all_body() {
        let (parsed, rest) = split("## Side A\n\nhello\n");
        assert!(parsed.is_none());
        assert_eq!(rest, "## Side A\n\nhello\n");
    }

    #[test]
    fn a_topic_containing_a_newline_cannot_break_out_of_the_block() {
        let mut m = meta();
        m.topic = Some("x\n---\nid: EVIL".to_string());
        let parsed = split(&build_frontmatter(&m)).0.expect("parses");
        assert_eq!(parsed.id, m.id, "a topic must not be able to set the id");
        assert_eq!(parsed.priority, m.priority, "later fields must survive");
        assert_eq!(parsed.updated_at, m.updated_at, "later fields must survive");
        assert!(
            !parsed.topic.as_deref().unwrap_or_default().contains('\n'),
            "the stored topic must be single-line"
        );
    }

    #[test]
    fn a_malformed_closing_fence_is_not_a_cassette_rather_than_an_empty_body() {
        // A trailing space after the fence used to parse as valid frontmatter
        // while yielding an empty body — which an autosave would then persist
        // over the real words. Skipping the file is the safe failure.
        let content = "---\nid: abc\npriority: 20\n--- \n\n## Side A\n\nmy words\n";
        let (meta, body) = split(content);
        assert!(
            meta.is_none(),
            "a malformed fence must not scan as a cassette"
        );
        assert_eq!(body, content, "and the content must be returned untouched");
        assert!(split(content).0.is_none(), "both entry points must agree");
    }

    #[test]
    fn writer_fields_must_be_writer_ids() {
        let good = |locked_by: &str, created_by: &str, last_writer: &str| {
            format!(
                "---\nid: cas_01K5GR7T2M9WPD0000000000AB\ntopic: x\npriority: 10\nstatus: open\n\
                 locked_by: {locked_by}\ncreated_by: {created_by}\nlast_writer: {last_writer}\n\
                 updated_at: 2026-09-24T09:00:00Z\n---\n\n"
            )
        };
        let w = "wri_01K5GQ00000000000000000001";
        assert!(
            split(&good("", w, w)).0.is_some(),
            "the valid baseline parses"
        );
        assert!(
            split(&good(w, w, w)).0.is_some(),
            "a writer-held sticky lock parses"
        );
        for text in [
            good("", "w", w),
            good("", w, "01K5GQ00000000000000000001"),
            good("cas_01K5GQ00000000000000000001", w, w),
            good("", "", w),
        ] {
            assert!(split(&text).0.is_none(), "must not parse:\n{text}");
        }
    }

    #[test]
    fn frontmatter_ending_the_file_still_parses() {
        // No body at all: the closing fence is the last line.
        let content = "---\nid: cas_01K5GR7T2M9WPD0000000000AB\npriority: 20\ncreated_by: wri_01K5GQ00000000000000000001\nlast_writer: wri_01K5GQ00000000000000000001\n---";
        let (meta, body) = split(content);
        assert_eq!(meta.expect("parses").id, "cas_01K5GR7T2M9WPD0000000000AB");
        assert_eq!(body, "");
    }

    #[test]
    fn frontmatter_missing_required_fields_is_rejected() {
        // A file we cannot identify must not silently become a cassette
        // with an empty id that later collides.
        assert!(split("---\ntopic: x\n---\n").0.is_none());
    }

    #[test]
    fn now_utc_is_rfc3339_zulu_seconds() {
        let t = now_utc();
        assert_eq!(t.len(), 20, "YYYY-MM-DDTHH:MM:SSZ — got {t}");
        assert!(t.ends_with('Z'), "{t}");
    }
}
