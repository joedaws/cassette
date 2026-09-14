// Nothing in the non-test build reaches the store until Phase 4 wires the CLI
// to it, so every item here is dead code to clippy until then. Lint attributes
// are inherited by nested modules, so this one covers the whole subtree.
// Remove it when Phase 4 lands.
#![allow(dead_code)]

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
pub mod meta;
pub mod priority;
