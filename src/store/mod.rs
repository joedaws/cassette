//! The session store: session directories of per-cassette markdown files.
//!
//! Layout under the data dir (created `0700`):
//!
//! ```text
//! ~/.local/share/cassette/
//!   writers.toml
//!   active                       # single line: active session id
//!   sessions/
//!     <session ulid>/
//!       session.toml
//!       cassettes/
//!         <slug>-<cassette ulid>.md
//!       .locks/
//!         <cassette ulid>        # empty flock anchor (Phase 3)
//! ```
//!
//! Pure data and math live in `ids`, `meta` and `priority`; the thin I/O layer
//! is `session`, `writers`, and `Store` here. No locking yet — Phase 3 wraps
//! the write calls.

pub mod ids;
