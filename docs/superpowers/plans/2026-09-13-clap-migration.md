# Phase 1: clap Migration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the hand-rolled 112-line `parse_args` with clap v4 derive, holding the
current CLI surface constant, so later phases can add subcommands without parser changes
being confused for storage bugs.

**Architecture:** Two safety nets are built first — unit tests over a pure parse function
for every successful invocation shape, and binary-level tests for every path that exits
(help, version, errors). Only then is clap swapped in underneath. Both suites must pass
**unchanged** after the swap; that is the entire proof that behavior held constant. The
parser moves out of `main.rs` (1279 lines) into a focused `src/cli.rs`.

**Tech Stack:** Rust 2021, clap 4 (derive feature), std test harness. No other new deps.

**Spec:** `docs/superpowers/specs/2026-09-13-session-store-design.md` (see "CLI surface"
and "Implementation phases" — this plan is Phase 1 only)

## Global Constraints

- **Behavior is frozen.** Every flag, action word, exit code, and `Args` field keeps its
  current meaning. This phase changes *how* args are parsed, never *what* they mean.
- **One deliberate deviation:** `--help` output becomes clap-generated and will not match
  the current `USAGE` string byte-for-byte. Content is preserved via `about` and doc
  comments. Tests must assert on exit code and key substrings, never on exact help text.
- **`-V` output must stay exactly `cassette 0.9.0`** (clap's `version` attribute produces
  `<name> <version>`, which already matches).
- **`die()` stays in `main.rs`.** It has 17 call sites and only 5 are in the parser.
- **Do not touch** `app.rs`, `cassette.rs`, `ui.rs`, `output.rs`, `config.rs`, `stats.rs`,
  `find.rs`, or `theme.rs`.
- Rust edition 2021; clap `4` with `features = ["derive"]`.
- Work happens on branch `feat/clap-migration` (already created).

## File Structure

| File | Responsibility |
|---|---|
| `src/cli.rs` | **Create.** The entire CLI surface: `Args` (the parsed result `main` consumes), the clap `Cli`/`Action` derive types, and `Cli::into_args()`. Owns its own tests. |
| `src/main.rs` | **Modify.** Delete `USAGE`, `positive()`, `parse_args()`, `parse_args_from()`, and `struct Args`. Add `mod cli;`. `main()` changes by one line. |
| `tests/cli.rs` | **Create.** Binary-level tests for paths that exit: `-h`, `-V`, and every error case. These cannot be unit tests because `die()` calls `process::exit`. |
| `Cargo.toml` | **Modify.** Add the clap dependency. |

**Why the split:** success-path parsing is testable in-process and belongs beside the
parser; error paths call `process::exit` and would kill the test runner, so they must
spawn the real binary. Two suites, two mechanisms, one behavior contract.

---

### Task 1: Make the current parser testable and characterize it

Extracts a pure function from `parse_args()` and pins every successful parse shape. These
tests pass the moment they are written — that is expected. Their job is to fail in Task 3
if clap parses anything differently.

**Files:**
- Modify: `src/main.rs:943` (`parse_args`)
- Test: `src/main.rs` (existing `#[cfg(test)] mod tests` at line ~1057)

**Interfaces:**
- Consumes: nothing.
- Produces: `fn parse_args_from(args: &[String]) -> Args` — same logic as `parse_args`,
  taking the argument list instead of reading `std::env::args()`. `Args` gains
  `#[derive(Debug, Default, PartialEq)]` so tests can compare whole structs.

- [ ] **Step 1: Add derives to `Args` so tests can compare it**

In `src/main.rs`, find `struct Args {` (line ~860) and add the derive above it:

```rust
#[derive(Debug, Default, PartialEq)]
struct Args {
```

- [ ] **Step 2: Split `parse_args` into a pure function plus a thin wrapper**

Replace the signature line `fn parse_args() -> Args {` and its first statement:

```rust
fn parse_args() -> Args {
    let args: Vec<String> = std::env::args().skip(1).collect();
```

with:

```rust
fn parse_args() -> Args {
    let args: Vec<String> = std::env::args().skip(1).collect();
    parse_args_from(&args)
}

fn parse_args_from(args: &[String]) -> Args {
```

Leave the entire rest of the function body unchanged. It already refers to `args`, which
is now the parameter.

- [ ] **Step 3: Run the build to confirm the refactor compiles**

Run: `cargo build`
Expected: compiles with no errors. Warnings about unused `Default` are acceptable.

- [ ] **Step 4: Write the characterization tests**

Add to the existing `#[cfg(test)] mod tests` block at the bottom of `src/main.rs`:

```rust
/// Build the argument slice the parser expects from string literals.
fn argv(args: &[&str]) -> Vec<String> {
    args.iter().map(|s| s.to_string()).collect()
}

/// Every successful invocation shape, pinned. These pass against the
/// hand-rolled parser and must keep passing against clap.
#[test]
fn parses_bare_invocation() {
    assert_eq!(parse_args_from(&argv(&[])), Args::default());
}

#[test]
fn parses_positional_note_name() {
    let a = parse_args_from(&argv(&["mynote"]));
    assert_eq!(a.note_name, Some("mynote".to_string()));
    assert!(a.resume.is_none());
}

#[test]
fn timer_is_converted_from_minutes_to_seconds() {
    assert_eq!(parse_args_from(&argv(&["-t", "10"])).timer_secs, Some(600));
}

#[test]
fn parses_word_goal_and_visible_lines() {
    let a = parse_args_from(&argv(&["-w", "500", "-l", "8"]));
    assert_eq!(a.word_goal, Some(500));
    assert_eq!(a.visible_lines, Some(8));
}

#[test]
fn parses_template_and_theme() {
    let a = parse_args_from(&argv(&["-T", "morning", "--theme", "gruvbox"]));
    assert_eq!(a.template, Some("morning".to_string()));
    assert_eq!(a.theme, Some("gruvbox".to_string()));
}

#[test]
fn parses_record_and_output_in_both_spellings() {
    assert!(parse_args_from(&argv(&["-R"])).record);
    assert!(parse_args_from(&argv(&["--record"])).record);
    assert!(parse_args_from(&argv(&["-o"])).print_stdout);
    assert!(parse_args_from(&argv(&["--output"])).print_stdout);
}

#[test]
fn bare_resume_means_newest_note() {
    assert_eq!(parse_args_from(&argv(&["--resume"])).resume, Some(None));
}

#[test]
fn resume_takes_an_optional_file_name() {
    assert_eq!(
        parse_args_from(&argv(&["--resume", "note.md"])).resume,
        Some(Some("note.md".to_string()))
    );
}

#[test]
fn resume_does_not_swallow_a_following_flag() {
    let a = parse_args_from(&argv(&["--resume", "-R"]));
    assert_eq!(a.resume, Some(None));
    assert!(a.record);
}

#[test]
fn parses_action_words() {
    assert!(parse_args_from(&argv(&["today"])).daily);
    assert!(parse_args_from(&argv(&["stats"])).stats);
    assert!(parse_args_from(&argv(&["+themes"])).list_themes);
}

#[test]
fn find_collects_trailing_words_as_one_query() {
    assert_eq!(
        parse_args_from(&argv(&["find", "some", "words"])).find,
        Some(vec!["some".to_string(), "words".to_string()])
    );
}

#[test]
fn bare_find_lists_everything() {
    assert_eq!(parse_args_from(&argv(&["find"])).find, Some(Vec::new()));
}

#[test]
fn flags_work_before_and_after_an_action_word() {
    assert_eq!(parse_args_from(&argv(&["-t", "10", "today"])).timer_secs, Some(600));
    let a = parse_args_from(&argv(&["today", "-t", "10"]));
    assert_eq!(a.timer_secs, Some(600));
    assert!(a.daily);
}
```

- [ ] **Step 5: Run the tests and confirm they all pass**

Run: `cargo test`
Expected: all tests PASS, including the 13 new ones. If any fail, the hand-rolled parser
does something different from what this plan assumed — stop and report which test and what
it actually returned, rather than editing the test to match.

- [ ] **Step 6: Commit**

```bash
git add src/main.rs
git commit -m "test: characterize the hand-rolled CLI parser before migrating

Extracts parse_args_from() so the parser can be exercised without reading
process argv, and pins every successful invocation shape. These pass now;
their job is to fail if the clap migration changes any behavior."
```

---

### Task 2: Pin the exiting paths at the binary level

`die()` and `--help` call `process::exit`, so they cannot be unit-tested — the test runner
would exit with them. These spawn the real binary instead. Like Task 1, they pass
immediately; they exist to catch a Task 3 regression.

**Files:**
- Create: `tests/cli.rs`

**Interfaces:**
- Consumes: the compiled binary via `env!("CARGO_BIN_EXE_cassette")`, which Cargo defines
  automatically for integration tests. No dev-dependency is needed.
- Produces: nothing consumed by later tasks.

- [ ] **Step 1: Write the integration tests**

Create `tests/cli.rs`:

```rust
use std::process::{Command, Output};

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cassette"))
        .args(args)
        .output()
        .expect("failed to run the cassette binary")
}

fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).to_string()
}

#[test]
fn version_flag_prints_name_and_version() {
    let out = run(&["-V"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        format!("cassette {}", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn help_flag_exits_zero_and_documents_the_surface() {
    let out = run(&["--help"]);
    assert_eq!(out.status.code(), Some(0));
    let help = String::from_utf8_lossy(&out.stdout);
    // Assert on content that survives the move to clap-generated help,
    // never on exact formatting.
    for expected in ["cassette", "--resume", "--theme", "today", "stats", "find"] {
        assert!(help.contains(expected), "help is missing {expected:?}:\n{help}");
    }
}

#[test]
fn short_help_flag_also_exits_zero() {
    assert_eq!(run(&["-h"]).status.code(), Some(0));
}

#[test]
fn zero_is_rejected_for_numeric_options() {
    for flag in ["-t", "-w", "-l"] {
        let out = run(&[flag, "0"]);
        assert_eq!(out.status.code(), Some(2), "{flag} 0 should exit 2");
    }
}

#[test]
fn non_numeric_values_are_rejected() {
    let out = run(&["-t", "abc"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(!stderr(&out).is_empty(), "an error message should reach stderr");
}

#[test]
fn unknown_option_exits_two() {
    let out = run(&["-x"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(!stderr(&out).is_empty());
}

#[test]
fn extra_positional_after_an_action_exits_two() {
    assert_eq!(run(&["today", "extra"]).status.code(), Some(2));
}

#[test]
fn missing_value_for_an_option_exits_two() {
    assert_eq!(run(&["-T"]).status.code(), Some(2));
    assert_eq!(run(&["--theme"]).status.code(), Some(2));
}
```

- [ ] **Step 2: Run the integration tests**

Run: `cargo test --test cli`
Expected: all 8 tests PASS against the current hand-rolled parser.

- [ ] **Step 3: Commit**

```bash
git add tests/cli.rs
git commit -m "test: pin CLI exit codes and help/version at the binary level

Covers the paths that call process::exit and so cannot be unit-tested.
Asserts exit codes and key help substrings, never exact help formatting,
so clap-generated help will not break them."
```

---

### Task 3: Swap in clap

The only risky task. Both suites from Tasks 1 and 2 must pass **unchanged** afterwards.
The clap structure below was verified against all twelve invocation shapes before this
plan was written; it is known to parse them identically.

**Files:**
- Modify: `Cargo.toml`
- Create: `src/cli.rs`
- Modify: `src/main.rs` (delete `USAGE`, `positive`, `parse_args`, `parse_args_from`,
  `struct Args`; add `mod cli;`; change one line in `main()`)
- Modify: `src/main.rs` tests (move the 13 characterization tests to `src/cli.rs`)

**Interfaces:**
- Consumes: `Args` and the 13 characterization tests from Task 1; `tests/cli.rs` from Task 2.
- Produces: `cli::Args` (same fields, now `pub`), `cli::Cli`, `cli::Action`, and
  `fn cli::parse() -> Args` — the single entry point `main()` calls.

- [ ] **Step 1: Add clap to Cargo.toml**

In `[dependencies]`, after `chrono`:

```toml
clap = { version = "4", features = ["derive"] }
```

- [ ] **Step 2: Create `src/cli.rs` with the clap surface**

```rust
use clap::{Parser, Subcommand};

/// The parsed CLI, in the shape `main()` consumes.
#[derive(Debug, Default, PartialEq)]
pub struct Args {
    pub timer_secs: Option<u32>,
    pub word_goal: Option<usize>,
    pub note_name: Option<String>,
    pub print_stdout: bool,
    pub visible_lines: Option<usize>,
    pub template: Option<String>,
    pub theme: Option<String>,
    pub list_themes: bool,
    pub record: bool,
    pub daily: bool,
    pub stats: bool,
    /// `find` with the query words that followed it; empty = list all.
    pub find: Option<Vec<String>>,
    /// `--resume` with an optional note name: `Some(None)` resumes the most
    /// recently modified note.
    pub resume: Option<Option<String>>,
}

#[derive(Parser, Debug)]
#[command(
    name = "cassette",
    version,
    about = "cassette — a freewriting TUI",
    disable_help_subcommand = true
)]
struct Cli {
    /// output note name or path; an existing note is resumed
    #[arg(value_name = "NAME")]
    name: Option<String>,

    /// countdown timer in minutes
    #[arg(short = 't', value_name = "MINUTES", global = true,
          value_parser = clap::value_parser!(u32).range(1..))]
    timer: Option<u32>,

    /// word goal (winds the tape reel)
    #[arg(short = 'w', value_name = "WORDS", global = true,
          value_parser = clap::value_parser!(u32).range(1..))]
    word_goal: Option<u32>,

    /// visible text rows per cassette (2-40)
    #[arg(short = 'l', value_name = "LINES", global = true,
          value_parser = clap::value_parser!(u32).range(1..))]
    visible_lines: Option<u32>,

    /// start with one cassette per topic from the named [templates] entry
    #[arg(short = 'T', value_name = "TEMPLATE", global = true)]
    template: Option<String>,

    /// color theme for this session (overrides config)
    #[arg(long, value_name = "NAME", global = true)]
    theme: Option<String>,

    /// record mode: no deletions, the tape only rolls forward
    #[arg(short = 'R', long, global = true)]
    record: bool,

    /// print to stdout on quit instead of writing a file
    #[arg(short = 'o', long = "output", global = true)]
    print_stdout: bool,

    /// load a saved note back into the TUI and keep writing
    #[arg(long, value_name = "FILE", num_args = 0..=1, global = true)]
    resume: Option<Option<String>>,

    #[command(subcommand)]
    action: Option<Action>,
}

#[derive(Subcommand, Debug)]
enum Action {
    /// open today's note (named by date)
    Today,
    /// streak, weekly/monthly notes and words, totals
    Stats,
    /// list recent notes newest-first; TEXT filters by name, topic, or content
    Find {
        #[arg(value_name = "TEXT", trailing_var_arg = true)]
        query: Vec<String>,
    },
    /// list available themes (built-in and from config.toml)
    #[command(name = "+themes")]
    Themes,
}

impl Cli {
    fn into_args(self) -> Args {
        let (daily, stats, list_themes, find) = match self.action {
            Some(Action::Today) => (true, false, false, None),
            Some(Action::Stats) => (false, true, false, None),
            Some(Action::Themes) => (false, false, true, None),
            Some(Action::Find { query }) => (false, false, false, Some(query)),
            None => (false, false, false, None),
        };
        Args {
            // The CLI takes minutes; the app works in seconds.
            timer_secs: self.timer.map(|m| m * 60),
            word_goal: self.word_goal.map(|w| w as usize),
            note_name: self.name,
            print_stdout: self.print_stdout,
            visible_lines: self.visible_lines.map(|l| l as usize),
            template: self.template,
            theme: self.theme,
            list_themes,
            record: self.record,
            daily,
            stats,
            find,
            resume: self.resume,
        }
    }
}

/// Parse the process arguments, exiting with clap's usage error (code 2) on
/// bad input.
pub fn parse() -> Args {
    Cli::parse().into_args()
}

#[cfg(test)]
fn parse_args_from(args: &[String]) -> Args {
    let mut argv = vec!["cassette".to_string()];
    argv.extend_from_slice(args);
    Cli::try_parse_from(argv)
        .expect("test invocation should parse")
        .into_args()
}
```

- [ ] **Step 3: Move the characterization tests into `src/cli.rs`**

Cut the `argv` helper and all 13 `#[test]` functions added in Task 1 out of
`src/main.rs`'s test module, and paste them into a new test module at the bottom of
`src/cli.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    /// clap's own structural validation: catches conflicting arg
    /// definitions that would otherwise only surface at runtime.
    #[test]
    fn cli_definition_is_valid() {
        Cli::command().debug_assert();
    }

    // Paste the `argv` helper and all 13 tests from src/main.rs here,
    // unchanged. They are, in order:
    //   parses_bare_invocation
    //   parses_positional_note_name
    //   timer_is_converted_from_minutes_to_seconds
    //   parses_word_goal_and_visible_lines
    //   parses_template_and_theme
    //   parses_record_and_output_in_both_spellings
    //   bare_resume_means_newest_note
    //   resume_takes_an_optional_file_name
    //   resume_does_not_swallow_a_following_flag
    //   parses_action_words
    //   find_collects_trailing_words_as_one_query
    //   bare_find_lists_everything
    //   flags_work_before_and_after_an_action_word
}
```

Confirm all 13 names above are present in `src/cli.rs` and absent from `src/main.rs`
before moving on. Do not modify the pasted tests. If one fails, the clap definition is wrong — fix
`src/cli.rs`, never the test.

- [ ] **Step 4: Wire `main.rs` to the new module**

In `src/main.rs`, add to the module list (after `mod cassette;`, keeping alphabetical
order):

```rust
mod cli;
```

Change the first line of `main()` (line ~121) from:

```rust
    let args = parse_args();
```

to:

```rust
    let args = cli::parse();
```

- [ ] **Step 5: Delete the hand-rolled parser**

From `src/main.rs`, delete these four items entirely:
- `struct Args { ... }` (line ~860, including the derives added in Task 1)
- `const USAGE: &str = "..."` (line ~893)
- `fn positive<T>(...)` (line ~930)
- `fn parse_args()` and `fn parse_args_from()` (line ~943 through the end of the function)

**Keep `fn die(msg: &str) -> !`** — it has 12 remaining call sites in `main()`.

- [ ] **Step 6: Build and fix fallout**

Run: `cargo build`
Expected: compiles. If `Args` is unresolved, add `use cli::Args;` near the other `use`
statements. If `die` is reported unused, that means a call site was deleted by mistake —
restore it rather than deleting `die`.

- [ ] **Step 7: Run the full suite — the proof that behavior held**

Run: `cargo test`
Expected: **all tests pass, including all 13 characterization tests and all 8 integration
tests, none of them modified.** A failure here is a real behavior change; report which
test failed and what clap produced instead. Do not adjust a test to make it pass.

- [ ] **Step 8: Verify the observable surface by hand**

Run each and confirm:

```bash
cargo run -- -V            # exactly: cassette 0.9.0
cargo run -- --help        # clap-formatted; lists -t -w -l -T --theme -R -o --resume
                           # and the today/stats/find/+themes actions
cargo run -- stats         # prints stats and exits 0
cargo run -- +themes       # lists themes and exits 0
cargo run -- -t 0; echo $? # 2
```

- [ ] **Step 9: Run the linters the project already uses**

Run: `cargo clippy -- -D warnings && cargo fmt --check`
Expected: clean. Run `cargo fmt` if formatting is off.

- [ ] **Step 10: Commit**

```bash
git add Cargo.toml Cargo.lock src/cli.rs src/main.rs
git commit -m "refactor: parse the CLI with clap v4 derive

Moves the CLI surface out of main.rs into a focused cli.rs and replaces
the 112-line hand-rolled parse_args with clap derive. Behavior is
unchanged: the characterization tests and binary-level exit-code tests
written against the old parser pass unmodified.

Action words (today/stats/find/+themes) become clap subcommands; option
flags are global so they still work on either side of an action word.
--resume keeps its optional-value semantics via num_args(0..=1).

The one deviation is --help, which is now clap-generated and no longer
byte-identical to the old USAGE string.

Unblocks the man page and shell completions, which clap_mangen and
clap_complete can generate from these same structs in Phase 6."
```

---

### Task 4: Bump the version to 0.10.0

The CLI surface moved to clap and the parser was replaced; that earns a minor bump.
0.9.0 -> 0.10.0 is the next minor — the version is a plain decimal-separated triple, not a
decimal number, so 0.10.0 follows 0.9.0.

**Files:**
- Modify: `Cargo.toml:3`
- Modify: `Cargo.lock` (regenerated by cargo, not edited by hand)

**Interfaces:**
- Consumes: the completed clap migration from Task 3.
- Produces: nothing. This is the last task.

- [ ] **Step 1: Bump the version**

In `Cargo.toml`, change:

```toml
version = "0.9.0"
```

to:

```toml
version = "0.10.0"
```

- [ ] **Step 2: Regenerate the lockfile**

Run: `cargo build`
Expected: compiles, and `Cargo.lock` updates its `name = "cassette"` entry to `0.10.0`.
Do not hand-edit `Cargo.lock`.

- [ ] **Step 3: Confirm the version test still passes**

Run: `cargo test --test cli version_flag_prints_name_and_version`
Expected: PASS. The test reads `env!("CARGO_PKG_VERSION")`, so it tracks the bump
automatically — if it fails, the binary and the manifest disagree and something else is
wrong.

- [ ] **Step 4: Verify the observable output**

Run: `cargo run -- -V`
Expected: exactly `cassette 0.10.0`

- [ ] **Step 5: Run the full suite**

Run: `cargo test`
Expected: all tests pass.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "chore: bump version to 0.10.0

The CLI parser was replaced wholesale and the surface now comes from clap;
that is a minor bump. No behavior change beyond the reported version."
```

---

## Verification

Phase 1 is done when:

- [ ] `cargo test` passes, with the 13 characterization tests and 8 integration tests
      unmodified since the tasks that wrote them
- [ ] `cargo clippy -- -D warnings` and `cargo fmt --check` are clean
- [ ] `src/main.rs` no longer contains `USAGE`, `positive`, `parse_args`, or `struct Args`
- [ ] `cassette -V` prints exactly `cassette <version from Cargo.toml>` (0.10.0 after Task 4)
- [ ] `git log --oneline` on `feat/clap-migration` shows four commits, tests before the swap

## Out of Scope

Deliberately excluded, per the spec's phasing:

- Man page and shell completion generation — Phase 6
- `session`, `queue`, and `writer` subcommands — Phase 4
- `--writer`, `--session`, `--json` global flags — Phase 4
- Renaming `notes_dir` to `data_dir` — Phase 6
- Any storage change whatsoever
