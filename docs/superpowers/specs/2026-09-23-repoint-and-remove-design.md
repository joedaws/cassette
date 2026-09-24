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
| `config::notes_dir`, `default_notes_dir` | 0 calls | Delete — see below; the compatibility argument for keeping the field was wrong |

Two things the parent spec lists as Phase 6 work are **already done** and this phase does not
redo them: `stats` and `find` moved to the store in 5a, and `today`/`resume` have resolved
against store sessions since 5a's `resolve_session`. The spec's phase list predates that move.

`clear_draft_flag` and the `[y/N]` crash-recovery prompt named in the spec's Deletions do not
exist in the tree at all — removed earlier or never built. Nothing to do.

### `notes_dir` — the compatibility argument was wrong, and the field is deleted

**Corrected 2026-09-23, after review.** This section originally argued that `Config.notes_dir`
had to survive: deleting the field would make an existing `config.toml` that still set it fail
to parse, and `load_config` exits 2 on a parse error, so an upgrade would break every command
for anyone who had not edited their config since 5a.

**That premise is false.** `Config` derives `Deserialize` with no `#[serde(deny_unknown_fields)]`,
and serde's derive ignores unknown fields by default. Verified against the real binary: a
config containing `this_key_does_not_exist = "hello"` parses fine and every command exits 0.
Removing the field breaks nobody.

Worse, the "regression test" this section justified passed identically with the field deleted —
it guarded nothing while being described in three places as the one thing standing between an
upgrade and a broken config. A test that cannot fail is worse than no test, because it is a
false assurance resting on a wrong belief.

So `notes_dir` is deleted along with the rest, and the test is replaced by one that pins what
is actually true and load-bearing: an unknown key is ignored rather than erroring. Adding
`deny_unknown_fields` makes that test fail, which the old one would not have noticed.

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

## The data directory is created `0700` — already done

**Corrected 2026-09-23, during implementation.** This section originally listed the `0700`
store root as Phase 6 work and specified that existing directories should be left alone. Both
were wrong, and the audit that caught it is the reason this phase checks the tree before
deleting or building anything.

`store::ensure_private_dir` has done this since `ee29073`, and does it *better* than this spec
proposed: the mode is baked into `mkdir(2)` rather than chmod'd afterwards, so the directory is
never briefly world-readable, and parents keep their own permissions. Four tests cover it
(`the_data_dir_is_private`, `session_subdirectories_are_private_too`,
`registering_a_writer_creates_a_private_root`, `the_data_dirs_parent_keeps_its_own_permissions`).

It also **does** tighten an existing directory that is looser than `0700`, which this spec had
said it should not. Bounded after review: it now clears only the group and other bits
(`mode & !0o077`) rather than calling `set_mode(0o700)` wholesale, which had silently stripped
setgid from a deliberately shared `2770` directory on every run. A test pins that `2770`
becomes `2700`, not `700`. On reflection the shipped behaviour is right: a store that is already
world-readable holds private writing that stays exposed for as long as nobody notices, and
"don't surprise the user" is a weaker argument than "don't leave their journal readable". The
spec is corrected to the code, not the code to the spec — changing tested security behaviour to
match an assumption written a day earlier would be the exact failure this phase guards against.

Nothing to implement. What remains of this section is the sync-root warning below.

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

## Man page and shell completions — deferred to a packaging phase

**Deferred 2026-09-23, at the user's direction, during implementation.**

The intent was `clap_mangen` and `clap_complete` as build-dependencies generating into
`OUT_DIR` from the `cli::Cli` derive tree, so they could not drift from the real command
surface. That needs `build.rs` to reach the clap tree, and the only way to do that without a
lib target is `include!("src/cli.rs")` — which does not compile, because `cli.rs` uses nine
`crate::` paths (`queue::Placement`, `queue::Side`, `store::writers::Kind` and others) directly
in its derive.

Three ways out were weighed: generating at runtime from the live `Command` (zero drift, but the
crates become runtime dependencies in the shipped binary); making `cli.rs` self-contained by
moving those domain enums behind local arg types and converting in `into_args`, the pattern
`WriterKindArg` already establishes (keeps the deps build-only and arguably improves the
layering, but touches the queue and writer command surface); or adding a lib target (standard,
but this crate has deliberately had no `lib.rs` — CLAUDE.md cites its absence as why there is
no public escape hatch to the store).

None is obviously right, and each is a structural decision about the crate rather than the
packaging step it looks like. So the redesign's six phases close without it, and man page,
completions and release packaging get their own design alongside `docs/distribution.md`.

**Phase 6 therefore adds no dependencies at all.**

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
- **Config compatibility** — an *unknown* key is ignored rather than erroring, which is what
  actually lets an older config keep working. Pinned by a test that fails if
  `deny_unknown_fields` is ever added.

## Out of scope

- Migrating the legacy `notes/` directory into the store. The files are untouched on disk and
  a migration is its own phase if it is ever wanted; the user accepted in 5a, with the cost
  stated, that those 45 notes stopped being counted.
- Detecting network filesystems.
- The man page, shell completions, and packaging them into releases — deferred to their own
  phase, above.

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
