# Phase 6 — Repoint and remove

## Purpose

The last phase of the session-store redesign. Phases 1–5d built the store, the lock
protocol, the queue commands and the TUI that reads and writes them; the flat-note machinery
they replaced is still in the tree, dead but compiling. This phase removes it, adds the one
command the parent spec promised and never built (`cassette export`), and finishes the
packaging items the redesign deferred.

Unlike every phase before it, this one is mostly deletion. That changes the risk: the danger
is not a subtle bug in new logic but removing something that still has a caller, or keeping
something because a stale doc comment mentions it.

## What the audit found

Every deletion below was checked against the tree rather than taken from the parent spec's
list. `src/output.rs` carries a module-wide `#![allow(dead_code)]` added in 5a precisely so
this phase could remove its contents:

| Item | External refs | Verdict |
|---|---|---|
| `output::cassette_body` | 5 (`session_writer.rs`) | **Keep** — the live store-body builder |
| `output::write_markdown` | 0 | Delete |
| `output::write_markdown_appended` | 0 | Delete |
| `output::parse_append_base`, `AppendBase` | 0 | Delete |
| `output::is_draft` | 0 | Delete |
| `output::frontmatter_date` | 0 | Delete |
| `output::parse_markdown` | 0 calls (1 doc-comment mention) | Delete, and fix the comment |
| `config::find_available_path` | 0 | Delete |
| `config::newest_note` | 0 | Delete |
| `config::notes_dir`, `default_notes_dir` | 0 calls | See below — **not** a simple delete |

Two things the parent spec lists as Phase 6 work are **already done** and this phase does not
redo them: `stats` and `find` moved to the store in 5a, and `today`/`resume` have resolved
against store sessions since 5a's `resolve_session`. The spec's phase list predates that move.

`clear_draft_flag` and the `[y/N]` crash-recovery prompt named in the spec's Deletions do not
exist in the tree at all — removed earlier or never built. Nothing to do.

### `notes_dir` is a compatibility surface, not dead code

`Config.notes_dir` has no callers, but deleting the **field** would make an existing
`config.toml` that still sets `notes_dir` fail to parse — and `config::load_config` exits 2 on
a parse error, so a user who has not touched their config since before 5a would find every
command broken by an upgrade.

So: the field stays and keeps parsing; `default_notes_dir` and the resolution helpers go. The
field is marked deprecated in its doc comment and in the README's config table, and reading it
does nothing. Removing it outright belongs to a release that can state a breaking change,
which this is not.

## `cassette export`

The parent spec promises it and it does not exist:

> `cassette export <session>` renders a session to a single flat markdown file (stdout or a
> path), reusing `build_body`. Not a second store — a one-way projection. It matters because
> the reason to use this tool is that the writing is plain text you own, and without it the
> words live in a directory tree with sidecar lock files.

**Shape.** `cassette export <SESSION> [--out <PATH>]`. `<SESSION>` is a session id, matching
every other id-taking command (4b's ruling: ULID only, an alias never resolves in place of an
id — except at the human entry points `resume`/`today`, and `export` is a scripting surface,
not one of those). Default output is stdout, so it pipes; `--out` writes a file.

**Content.** One `# Cassette N — topic` heading per cassette in queue order, then that
cassette's body exactly as `queue::write::build_body` produces it. Closed cassettes are
included — an export is the archive, and excluding them would make it a lossy one — marked
with a `(closed)` suffix on the heading. Damaged cassettes are **not** silently dropped: each
becomes a `# Cassette N — unreadable (<reason>)` heading, for the same reason 5c gave them
rows in the TUI.

**It reads without locking.** Export is a read-only projection, and `atomic_write`'s
rename means a concurrent writer can never present a torn file. Taking locks would make an
export of a busy session fail for no benefit, which is the `queue list` precedent.

## The data directory is created `0700`

Freewriting content is private by nature and the default `0755` would make every session
world-readable on a shared machine. Applied where the store root is created, Unix-only
(`std::os::unix::fs::PermissionsExt`), behind `#[cfg(unix)]` — Windows is already out of CI by
the Phase 3 scope decision, and there is no portable equivalent worth faking.

**Existing directories are not re-chmodded.** Changing permissions on a directory the user
already has is a surprise they did not ask for, and it would fight anyone who set them
deliberately. New stores get `0700`; old ones are left alone.

## The cloud-sync warning

The parent spec: "Where cheap, warn at startup when `data_dir` resolves under a known sync
root." Cheap is the operative word — this is a path-prefix check against a small list
(`Dropbox`, `iCloud Drive` / `Mobile Documents`, `OneDrive`, `Google Drive`, `Sync.com`), not
a filesystem probe.

It **warns and continues** rather than refusing. The user may have a good reason, the check is
a heuristic that can produce false positives on a directory that merely has one of those
words in its name, and a tool that refuses to start over a substring match would be worse than
the risk it names. One line to stderr, once, at startup.

Network filesystems stay undetected, as the parent spec's own analysis concludes — there is no
cheap userspace way to tell whether a given mount's `flock` is cross-client safe, and a
mount-type guess would give false confidence rather than protection.

## Man page and shell completions

`clap_mangen` and `clap_complete` as **build-dependencies**, generating into `OUT_DIR` from the
existing `cli::Cli` derive tree — so they cannot drift from the real command surface, which is
the whole reason to generate rather than write them.

A `build.rs` emits `cassette.1` and completions for bash, zsh and fish. Packaging them into a
release tarball is a distribution concern and belongs in `docs/distribution.md`, not here; this
phase's job is that the artifacts exist and regenerate from the source of truth.

**This is the one part of the phase that adds dependencies**, and both are build-only, so they
do not enter the shipped binary.

## Testing

- **Deletion is proved by the compiler**, and by the `#![allow(dead_code)]` at the top of
  `output.rs` being **removed** in the same change. If anything left in that module is still
  dead, the build fails — which is the point, and is why the allow cannot simply be narrowed.
- **`export`** — a session with two open cassettes, one closed and one damaged renders all
  four, in queue order, with the closed and damaged ones marked; `--out` writes the same bytes
  stdout produces; an unknown session id is a usage error (exit 2), matching every other
  id-taking command.
- **Body fidelity** — an exported cassette's body is byte-identical to what `queue write`
  wrote, asserted against `build_body` rather than a hand-written string, so the export cannot
  drift from the writer.
- **`0700`** — a store created fresh has mode `0700`; a pre-existing directory with different
  permissions is left unchanged. `#[cfg(unix)]`.
- **The sync warning** — a `data_dir` under a `Dropbox` path warns, an ordinary path does not,
  and in both cases the command still runs.
- **Config compatibility** — a `config.toml` setting `notes_dir` still parses and does not
  exit 2. This is the one test standing between an upgrade and a user's broken config.
- **Generated artifacts** — `build.rs` produces the man page and three completion files.

## Out of scope

- Migrating the legacy `notes/` directory into the store. The files are untouched on disk and
  a migration is its own phase if it is ever wanted; the user accepted in 5a, with the cost
  stated, that those 45 notes stopped being counted.
- Removing `Config.notes_dir` outright — see above.
- Detecting network filesystems.
- Packaging the generated artifacts into releases (`docs/distribution.md`).

## Open items carried in

Recorded; none blocking. These have survived several phases and none belongs to this one's
work, so they are noted rather than swept in:

- `assert!(after >= before)` in `tests/cli.rs` passes when the mtime did not move.
- After a side-B merge, `cursor_for_a` is hardcoded to 0 while the mirror case keeps side B's
  stored cursor.
- `queue::write::write_permitted` keeps an inline id→name lookup beside the shared
  `store::writers::display_name`.
- The startup `find::scan_store` for `cassette sessions` reads every cassette of every session
  before the first frame. Pre-existing (`find` pays it too) but now on an interactive path;
  worth measuring before the store reaches four digits of sessions.
