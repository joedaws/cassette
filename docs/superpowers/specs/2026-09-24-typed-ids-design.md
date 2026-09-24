# Typed ids: `ses_`, `cas_`, `wri_`

## Purpose

Every identity in the store is a bare 26-character ULID today: sessions, cassettes and writers
all look alike. Collisions are not the problem (80 random bits per millisecond, and each kind
lives in its own namespace). The problem is **legibility**:

- A cassette id passed to `--session` gets "no session '…'", which is true but unhelpful. An
  agent juggling 26-character strings makes this mistake routinely.
- In a cassette file, `id`, `created_by`, `last_writer` and `locked_by` are indistinguishable,
  and three of the four are writer ids.
- Lock anchors, log lines and pasted output don't say what they name.

Every id therefore carries its kind as a three-letter prefix.

## Decisions (made with the user, 2026-09-24)

1. **Prefixes are `ses_`, `cas_`, `wri_`**, for session, cassette and writer. They are the first
   three letters of each noun, matching the CLI's own words (`--session`, cassette, `--writer`).
2. **No legacy support.** A bare ULID is rejected everywhere. Old stores become unreadable until
   migrated by hand (see "Migrating an existing store"), and the user accepts that. The codebase
   does not carry a compatibility path.
3. **Prefixed strings validated at every entry point**, not Rust newtypes. Ids stay `String`
   internally. Types (`SessionId`/`CassetteId`/`WriterId`) would add compile-time checking but
   touch hundreds of sites. They remain possible later as a separate refactor.
4. **One trace of the old world:** listings report how many session directories they skipped,
   so an unmigrated store doesn't look silently empty.

## Format

```
<prefix>_<ULID>        e.g.  ses_01K5GQ2R8VXM3T0000000000AB
```

- `prefix` is exactly `ses`, `cas` or `wri`, lowercase.
- The separator is `_`. It must not be `-`: cassette files are named `<slug>-<id>.md`, and
  `ids::id_from_file_name` splits on the **last** dash because slugs contain dashes. `_` never
  appears in a slug (`ids::slug` maps every non-alphanumeric run to `-`).
- The ULID part keeps today's rule: 26 Crockford base32 characters, case-insensitive.
- Total length is 30.

`atomic_write`'s temp name (`.tmp-<ulid>`) is not an identity and stays a bare ULID.

## The `ids` module

`src/store/ids.rs` gains:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdKind { Session, Cassette, Writer }

impl IdKind {
    pub fn prefix(self) -> &'static str;   // "ses" | "cas" | "wri"
    pub fn noun(self) -> &'static str;     // "session" | "cassette" | "writer"
}

/// A fresh id of `kind`: `<prefix>_<ULID>`.
pub fn new(kind: IdKind) -> String;

/// Why a string is not an id of the expected kind.
pub enum IdError {
    /// Well-formed, but a different kind: carries what it actually is.
    WrongKind { expected: IdKind, found: IdKind },
    /// Not `<known prefix>_<ULID>` at all — including a bare ULID.
    Malformed { expected: IdKind },
}

/// `Ok(())` when `s` is a well-formed id of `kind`.
pub fn check(kind: IdKind, s: &str) -> Result<(), IdError>;

/// The kind of a well-formed id, if it is one.
pub fn kind_of(s: &str) -> Option<IdKind>;
```

`new_id()` and `is_valid_id()` are deleted, and so is `ID_LEN`'s role as the whole id's length.
The ULID length becomes a private detail of `check`. `IdError` implements `Display` with the
messages under "Errors" below, so every entry point words it the same way.

## Minting

| Kind | Site |
|---|---|
| session | `Store::create_session` |
| cassette | `queue::edit::new`, `SessionWriter::create_cassette` |
| writer | `store::writers::ensure`, `store::writers::resolve` |

## Entry points: where outside ids are checked

| Entry point | Kind | On failure |
|---|---|---|
| `--session <ID>` (every `queue` command), `session alias <ID>` via `Store::require_session` | session | exit 2, `IdError` message. This replaces today's "expected a 26-character ULID". |
| `export <SESSION>` (same gate) | session | exit 2 |
| queue `<ID>` positional: `show`, `write`, `close`, `reopen`, `topic`, `move`, `lock`, `unlock` | cassette | exit 2, checked before any lock is taken |
| `queue move --before/--after <ID>` | cassette | exit 2 |
| `resume <NAME_OR_ID>` | session | alias match first, as today. Otherwise, if `check(Session)` passes, open it as an id. Otherwise the existing "no session named …" |
| cassette frontmatter `id` | cassette | the cassette is **damaged** (`BadFrontmatter`) |
| frontmatter `created_by`, `last_writer`; `locked_by` when non-empty | writer | damaged (`BadFrontmatter`) |
| frontmatter `id` disagreeing with the file name's id | — | damaged (`BadFrontmatter`) |
| cassette file name `<slug>-<id>.md` | cassette | `id_from_file_name` returns `None` unless the suffix is a well-formed `cas_` id, so the file is damaged/unrecognised as today |
| session directory names under `sessions/` | session | skipped by `list_sessions`, counted (see "Skipped directories") |
| `writers.toml` table keys | writer | **hard error**: `writers::read` fails with `InvalidData` naming the key, which is exit 1 wherever the registry is read |

Why `writers.toml` fails hard rather than skipping: silently dropping a writer would make every
cassette crediting it resolve to a raw id, and would make sticky-lock and authority checks
quietly treat real writers as unknown. A registry the store cannot trust is an I/O-class
failure, the same stance `writers::read` already takes on a file that doesn't parse.

The frontmatter id/file-name agreement check is new. It closes a gap the prefix makes cheap:
with kinds in the name, a hand-migrated file whose frontmatter and name drifted apart is caught
at scan time instead of producing two identities for one cassette.

## Errors

`IdError`'s `Display`, with the flag or argument name supplied by the caller:

- WrongKind: `` `cas_01K5…` is a cassette id; --session takes a session id (ses_…) ``
- Malformed: `` malformed session id '01K5…': expected ses_ followed by a 26-character ULID ``

A bare ULID is `Malformed`, with no special wording. The code does not know old ids existed.

## Skipped directories

`Store::list_sessions` returns the directories it skipped alongside the sessions it found:

```rust
pub struct SessionListing {
    pub sessions: Vec<(String, SessionMeta)>,
    /// Directories under `sessions/` whose name is not a well-formed `ses_` id.
    pub skipped: usize,
}
```

`session list`, `find`, `stats` and the `sessions` picker print one trailing line when
`skipped > 0`:

```
2 session directories skipped: names are not ses_ ids
```

This follows the same pattern as the existing `N unreadable` count: work that exists on disk
must not vanish from every view silently. The wording describes the directory, not "old
format", because the code has no concept of an old format. A directory with a well-formed
name but an unreadable `session.toml` is skipped exactly as today and is not counted here.

## JSON contract

Every `id`-carrying field in `--json` output (`session.id`, cassette `id`) carries the prefix.
`WriterRef` is unchanged: it emits names, not ids. **This breaks agents** that parse or store
ids, so it gets its own CHANGELOG entry under "Changed", and the `cassette-session` skill's
examples are updated.

## Migrating an existing store (by hand)

For the user's existing data, with every `cassette` process stopped:

1. `writers.toml`: prefix each table key with `wri_`.
2. Each `sessions/<ULID>/` → `sessions/ses_<ULID>/`.
3. Inside each session, each `cassettes/<slug>-<ULID>.md` → `<slug>-cas_<ULID>.md`, and in its
   frontmatter: `id:` gets `cas_`, and `created_by:`, `last_writer:` and a non-empty
   `locked_by:` get `wri_`.
4. Delete every `.locks/` directory (the per-session ones and the registry's). Anchors are
   recreated on the next lock. Renaming them is pointless, since a stale anchor holds no lock
   once its process is gone.

This procedure goes into `README.md` under a short "Upgrading from bare ULIDs" note, with the
CHANGELOG entry pointing to it. The codebase contains no migration code.

## Tests

- `ids`: `new(kind)` round-trips through `check(kind)` and `kind_of`; wrong kind →
  `WrongKind`; bare ULID, unknown prefix, `-` separator, and a short ULID → `Malformed`.
- CLI (`tests/cli.rs`): a cassette id passed to `--session` gives exit 2 and a message naming
  both kinds; a session id as a queue `<ID>` gives exit 2; a bare ULID to `--session` gives
  exit 2 with the Malformed message; `resume ses_…` opens by id.
- Store: a bare-ULID session directory is skipped and counted; a cassette with a bare-ULID
  frontmatter `id`, a writer field missing `wri_`, or an id disagreeing with its file name is
  damaged; a bare key in `writers.toml` makes `writers::read` fail.
- Every existing fixture literal (~130 hard-coded ULIDs across `src/` tests and
  `tests/`) gains its prefix. Helpers that hand-write frontmatter (`tests/cli.rs`'s sticky-lock
  fixtures) are updated.

## Docs

- CLAUDE.md: the command examples use `ses_`/`cas_` ids. The `store` and `session.rs`
  paragraphs ("Sessions are named by ULID only") become "by `ses_` id only". The planning
  section needs no change.
- README: examples and the frontmatter sample carry prefixes, and the upgrade note is added.
- `.claude/skills/cassette-session/SKILL.md` and `cassette-writeup`'s `fixture.sh` comments
  are updated.
- `docs/follow-up.md`: no entry; this was never on it.

## Out of scope

- Rust newtypes for ids (Decision 3).
- Any migration tooling (Decision 2).
- Changing writer *names*. `--writer` still takes a name, never an id.

## Acceptance criteria

- `grep -rn "new_id\|is_valid_id" src` finds nothing.
- `cassette session new` prints `ses_…`, `queue new` prints `cas_…`, and `writers.toml` keys
  are `wri_…`.
- Every CLI entry point in the table rejects a wrong-kind id and a bare ULID with exit 2 and
  the specified wording.
- A store with a bare-ULID session directory lists nothing from it and prints the skipped line
  in `session list`, `find`, `stats` and the picker.
- `cargo test` and `cargo clippy --all-targets -- -D warnings` are green.
- The hand-migration procedure, applied to a copy of a pre-change store, produces a store the
  new binary reads in full. Verified once on a scratch store built by the pre-change binary.
