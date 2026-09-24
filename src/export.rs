//! `cassette export <SESSION>` — a session rendered to one flat markdown
//! file.
//!
//! A one-way projection, not a second store. The reason to use this tool is
//! that the writing is plain text you own, and without this the words live
//! in a directory tree beside sidecar lock files.
//!
//! **It takes no locks.** Export is read-only, and `store::atomic_write`
//! writes to a temp file and renames, so a reader can never see a torn
//! cassette. Locking would make exporting a session an agent happens to be
//! writing fail for no benefit — the same reasoning `queue list` follows.

use crate::store::SessionScan;

/// Render a scanned session as flat markdown.
///
/// Cassettes come out in queue order, which `Store::scan_session` has
/// already applied. Closed cassettes are **included** and marked: an export
/// is the archive, and one that silently dropped everything finished would
/// be a lossy archive, which is worse than a noisy one. Damaged cassettes
/// get a heading of their own for the same reason Phase 5c gave them rows in
/// the TUI — a file that vanishes from every view is invisible work.
pub fn render(scan: &SessionScan) -> String {
    let mut out = String::new();
    for (i, c) in scan.cassettes.iter().enumerate() {
        let n = i + 1;
        match (
            &c.meta.topic,
            c.meta.status == crate::store::meta::Status::Closed,
        ) {
            (Some(t), false) => out.push_str(&format!("# Cassette {n} — {t}\n\n")),
            (Some(t), true) => out.push_str(&format!("# Cassette {n} — {t} (closed)\n\n")),
            (None, false) => out.push_str(&format!("# Cassette {n}\n\n")),
            (None, true) => out.push_str(&format!("# Cassette {n} (closed)\n\n")),
        }
        // The stored body verbatim, so an export cannot drift from what the
        // writer produced. Trimmed and re-terminated only so the spacing
        // between cassettes is even regardless of how each file ended.
        out.push_str(c.body.trim());
        out.push_str("\n\n");
    }

    for (i, d) in scan.damaged.iter().enumerate() {
        let n = scan.cassettes.len() + i + 1;
        out.push_str(&format!(
            "# Cassette {n} — {} (unreadable: {})\n\n",
            d.label(),
            d.reason
        ));
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::meta::{CassetteMeta, Status};
    use crate::store::{ids, meta, session::SessionMeta, Store};

    fn session_meta() -> SessionMeta {
        SessionMeta {
            alias: None,
            created: meta::now_utc(),
            timer_secs: None,
            word_goal: None,
        }
    }

    fn add(
        store: &Store,
        session: &str,
        priority: i64,
        status: Status,
        topic: Option<&str>,
        body: &str,
    ) {
        let m = CassetteMeta {
            id: ids::new_id(),
            topic: topic.map(|t| t.to_string()),
            priority,
            status,
            locked_by: None,
            created_by: "w".to_string(),
            last_writer: "w".to_string(),
            updated_at: meta::now_utc(),
        };
        store.add_cassette(session, &m, body).expect("add cassette");
    }

    /// An export is the archive, so it is not lossy: closed cassettes are
    /// included and marked, and a damaged one becomes a visible heading
    /// rather than a silent omission.
    #[test]
    fn export_renders_every_cassette_in_queue_order_and_marks_the_unusual_ones() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let session = store.create_session(&session_meta()).expect("session");
        add(
            &store,
            &session,
            10,
            Status::Open,
            Some("morning"),
            "## Side A\n\nfirst words\n",
        );
        add(
            &store,
            &session,
            20,
            Status::Closed,
            Some("done"),
            "## Side A\n\narchived words\n",
        );
        std::fs::write(
            store
                .cassettes_dir(&session)
                .join("01M3600000000000000000BAD.md"),
            "no frontmatter\n",
        )
        .expect("write");

        let scan = store.scan_session(&session).expect("scan");
        let out = render(&scan);

        let first = out.find("first words").expect("open cassette");
        let second = out.find("archived words").expect("closed cassette");
        assert!(first < second, "queue order: open before closed\n{out}");
        assert!(out.contains("# Cassette 1 — morning\n"), "{out}");
        // A blank line between cassettes, so consecutive ones do not run
        // together into one block when the file is read as markdown.
        assert!(
            out.contains("first words\n\n# Cassette 2"),
            "cassettes are separated by a blank line: {out:?}"
        );
        assert!(
            out.contains("# Cassette 2 — done (closed)"),
            "closed cassettes are marked: {out}"
        );
        // The number, the filename and the reason in one assertion: each
        // was independently deletable with the suite green.
        assert!(
            out.contains("# Cassette 3 — 01M3600000000000000000BAD.md (unreadable: frontmatter is unparseable)"),
            "a damaged cassette is named and numbered after the real ones: {out}"
        );
    }

    /// The export must not drift from what the writer produced: asserted
    /// against the stored body rather than a hand-written string, so a
    /// change to the writer shows up here instead of diverging silently.
    #[test]
    fn an_exported_body_is_what_the_writer_wrote() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let session = store.create_session(&session_meta()).expect("session");
        let body = crate::queue::write::build_body("side a text", "side b text");
        add(&store, &session, 10, Status::Open, None, &body);

        let scan = store.scan_session(&session).expect("scan");
        let out = render(&scan);

        assert!(
            out.contains(body.trim()),
            "the stored body appears verbatim:\n{out}"
        );
        assert!(out.contains("# Cassette 1\n"), "no topic, no dash: {out}");
    }

    /// An empty session is not an error — it renders to nothing, which pipes
    /// and diffs the way an empty file should.
    #[test]
    fn an_empty_session_exports_to_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::new(dir.path().to_path_buf());
        let session = store.create_session(&session_meta()).expect("session");
        let scan = store.scan_session(&session).expect("scan");
        assert_eq!(render(&scan), "");
    }
}
