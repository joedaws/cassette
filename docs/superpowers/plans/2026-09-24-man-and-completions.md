# Man Page and Completions Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `cassette completions <SHELL>` and `cassette man [--out-dir DIR]` generate completions and man pages from the live clap tree, and the release tarball ships them.

**Architecture:** `cli.rs` exposes `command()` (the clap `Command` that `parse()` uses). Two new `Command` variants lower into `Args` fields that `main()` handles first, before config, store or terminal. The release workflow runs the built binary to fill `dist/`. `docs/distribution.md` is created to hold the packaging record.

**Tech Stack:** Rust, clap 4 derive, `clap_complete`, `clap_mangen`; GitHub Actions.

**Spec:** `docs/superpowers/specs/2026-09-24-man-and-completions-design.md`

## Global Constraints

- Both generators read `cli::command()` and nothing else. No hand-written completion or roff.
- Neither command loads config, touches the store, or enters raw mode. A broken `config.toml` must not break `completions`.
- New modes are subcommands (CLAUDE.md `cli.rs` rule). No top-level flags.
- Binary at the tarball root, as today.
- Record release binary size before/after in the Task 2 commit message. Flag it in the PR description if the growth exceeds ~500 KB.
- Clippy `-D warnings` green after every task.

## Review Focus

- **Stale help text becomes a published man page.** `Command`'s doc comments still say "note" (`new`: "start a session in a named note", `today`: "open today's note", `resume`: "load a saved note…", `find`: "list recent notes…", `-o`: "…instead of writing a file"). Task 1 fixes the wording before anything renders it.
- **A broken `config.toml`.** `completions` must still work. Task 2 CLI test.
- **`CASSETTE_DATA_DIR` pointing nowhere.** It must not be created. Task 2 CLI test.
- **`--out-dir` that doesn't exist.** `generate_to` needs the directory, so create it (`create_dir_all`). An unwritable one exits 1 with the path in the message. Task 2 test for the create case.
- **`--json` given with `man`.** Ignored, not an error. Covered by Task 2 dispatching before any JSON path.

---

### Task 1: Help text says "session", not "note"

**Files:**
- Modify: `src/cli.rs` (doc comments on `Command` variants ~lines 193–207; `-o` ~line 176; `Args.resume` comment ~27)
- Test: `tests/cli.rs` (`help_flag_exits_zero_and_documents_the_surface` ~line 76, if it asserts any of these strings)

- [ ] **Step 1: Rewrite the doc comments.** These become `--help` and man text:

```rust
    /// start a new session with this alias (never resumes an existing one)
    New { .. }
    /// open today's session (aliased by date), creating it if needed
    Today,
    /// reopen a session: the newest, the newest with this alias, or this id
    Resume { #[arg(value_name = "NAME_OR_ID")] file: Option<String> },
    /// streak, weekly/monthly sessions and words, totals
    Stats,
    /// list recent sessions newest-first; TEXT filters by id, alias, topic or content
    Find { .. },
```
`-o`: `/// print the session to stdout on quit instead of saving it`.
`Args.resume` comment: "`resume` with an optional alias or session id: `Some(None)` resumes the newest session."

Check each claim against `main.rs` `resolve_session` before committing. The phrasing above follows CLAUDE.md's description of it. Keep the field name `file` (renaming touches `into_args` and tests for no user-visible gain); only `value_name` changes.

- [ ] **Step 2: Verify** with `cargo test && cargo clippy --all-targets -- -D warnings`. If a `tests/cli.rs` help test asserts old wording, update it to the new wording.

- [ ] **Step 3: Commit**

```bash
git add src/cli.rs tests/cli.rs
git commit -m "docs: CLI help says session where it still said note"
```

---

### Task 2: `completions` and `man` subcommands

**Files:**
- Modify: `Cargo.toml` (deps)
- Modify: `src/cli.rs` (`Command::Completions`, `Command::Man`; `Args` fields; `into_args`; `pub fn command()`)
- Modify: `src/main.rs` (dispatch at the top of `main`, before `config::load_config`)
- Test: `tests/cli.rs`, `src/cli.rs` tests

**Interfaces:**
- Produces: `pub fn cli::command() -> clap::Command`; `Args.completions: Option<clap_complete::Shell>`; `Args.man: Option<Option<PathBuf>>` (`Some(None)` = stdout, `Some(Some(dir))` = full set).

- [ ] **Step 1: Record the baseline size**

Run: `cargo build --release && stat -c %s target/release/cassette`
Write down the number for the commit message.

- [ ] **Step 2: Add the dependencies**

Run: `cargo add clap_complete clap_mangen`
Expected: both added at their current versions, compatible with clap 4.

- [ ] **Step 3: Failing CLI tests** (`tests/cli.rs`; `bin()`, `stderr()` exist)

```rust
#[test]
fn completions_cover_the_queue_surface_for_each_shell() {
    for shell in ["bash", "zsh", "fish"] {
        let out = Command::new(bin()).args(["completions", shell]).output().expect("spawn");
        assert_eq!(out.status.code(), Some(0), "{shell}: {}", stderr(&out));
        let text = String::from_utf8_lossy(&out.stdout);
        assert!(text.contains("queue") && text.contains("write"), "{shell}");
    }
}

#[test]
fn completions_reject_an_unknown_shell() {
    let out = Command::new(bin()).args(["completions", "nope"]).output().expect("spawn");
    assert_eq!(out.status.code(), Some(2));
}

#[test]
fn generators_ignore_config_and_never_create_the_store() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cfg = dir.path().join("cfg");
    std::fs::create_dir_all(cfg.join("cassette")).expect("mkdir");
    std::fs::write(cfg.join("cassette/config.toml"), "this is = = not toml").expect("write");
    let store = dir.path().join("never");
    for args in [&["completions", "zsh"][..], &["man"][..]] {
        let out = Command::new(bin())
            .args(args)
            .env("XDG_CONFIG_HOME", &cfg)
            .env("CASSETTE_DATA_DIR", &store)
            .output()
            .expect("spawn");
        assert_eq!(out.status.code(), Some(0), "{args:?}: {}", stderr(&out));
    }
    assert!(!store.exists(), "no data dir is created");
}

#[test]
fn man_prints_the_top_page_and_out_dir_writes_one_per_subcommand() {
    let out = Command::new(bin()).arg("man").output().expect("spawn");
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let page = String::from_utf8_lossy(&out.stdout);
    assert!(page.lines().any(|l| l.starts_with(".TH") && l.contains("cassette")), "{page}");

    let dir = tempfile::tempdir().expect("tempdir");
    let target = dir.path().join("man1");
    let out = Command::new(bin())
        .args(["man", "--out-dir"])
        .arg(&target)
        .output()
        .expect("spawn");
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(target.join("cassette.1").is_file());
    assert!(target.join("cassette-queue-write.1").is_file());
}
```
Check that `config::config_path` honors `XDG_CONFIG_HOME` as CLAUDE.md says. It does per `config.rs`'s doc, so the broken-config fixture takes effect.

- [ ] **Step 4: Run** `cargo test --test cli completions man_ generators_`. Expected: FAIL, unrecognized subcommand (exit 2).

- [ ] **Step 5: Implement**

`src/cli.rs`, in `Command` after `Export`:
```rust
    /// print shell completions to stdout
    Completions {
        #[arg(value_name = "SHELL")]
        shell: clap_complete::Shell,
    },
    /// print the man page to stdout, or write every page to a directory
    Man {
        /// write cassette.1 and one page per subcommand here instead
        #[arg(long, value_name = "DIR")]
        out_dir: Option<PathBuf>,
    },
```
`Args`:
```rust
    /// `completions <SHELL>`.
    pub completions: Option<clap_complete::Shell>,
    /// `man`: `Some(None)` prints cassette.1; `Some(Some(dir))` writes the set.
    pub man: Option<Option<PathBuf>>,
```
`into_args`:
```rust
            Some(Command::Completions { shell }) => args.completions = Some(shell),
            Some(Command::Man { out_dir }) => args.man = Some(out_dir),
```
Below `parse()`:
```rust
/// The clap command `parse()` parses. The one source the man page and the
/// completions are generated from, so neither can drift from the real
/// surface.
pub fn command() -> clap::Command {
    <Cli as clap::CommandFactory>::command()
}
```
If `Args` derives `PartialEq` and `clap_complete::Shell` does not implement it, check with `cargo check`. `Shell` derives `PartialEq, Eq` in current clap_complete. If not, lower it to a local `ShellArg` enum in the `WriterKindArg` style.

`src/main.rs`, first lines of `main()` after `cli::parse()` and **before** `config::load_config()`:
```rust
    // Generators first: they describe the CLI, not a store, so neither a
    // broken config.toml nor a missing data dir may stop them.
    if let Some(shell) = args.completions {
        clap_complete::generate(shell, &mut cli::command(), "cassette", &mut io::stdout());
        return Ok(());
    }
    if let Some(out_dir) = args.man.take() {
        let cmd = cli::command();
        match out_dir {
            None => clap_mangen::Man::new(cmd).render(&mut io::stdout())?,
            Some(dir) => {
                if let Err(e) = std::fs::create_dir_all(&dir)
                    .and_then(|()| clap_mangen::generate_to(cmd, &dir))
                {
                    die_with(1, &format!("cannot write man pages to {}: {e}", dir.display()));
                }
            }
        }
        return Ok(());
    }
```
`args` is already `let mut`. Confirm `clap_mangen::generate_to`'s signature with `cargo doc -p clap_mangen --open` or the source; it exists in 0.2.x. If it is missing in the resolved version, loop `cmd.get_subcommands()` recursively, rendering each with `Man::new(sub.clone().name(format!("cassette-{..}")))`, and say so in the commit.

- [ ] **Step 6: Verify** with `cargo test && cargo clippy --all-targets -- -D warnings`. Expect green.

- [ ] **Step 7: Measure** with `cargo build --release && stat -c %s target/release/cassette`.

- [ ] **Step 8: Commit** (fill in the sizes)

```bash
git add Cargo.toml Cargo.lock src/cli.rs src/main.rs tests/cli.rs
git commit -m "feat: cassette completions and cassette man generate from the clap tree

Release binary: <before> -> <after> bytes."
```

---

### Task 3: Release packaging and `docs/distribution.md`

**Files:**
- Modify: `.github/workflows/release.yml` (replace the "Package artifact" step)
- Create: `docs/distribution.md`
- Modify: `README.md` ("Build from source" and "For maintainers → Releasing")
- Modify: `CLAUDE.md` (`cli.rs` paragraph; Commands block)
- Modify: `docs/follow-up.md` (remove the entry), `CHANGELOG.md` (Unreleased → Added)

- [ ] **Step 1: Workflow.** Replace the `Package artifact` step with:

```yaml
      - name: Package artifact
        run: |
          set -euo pipefail
          BIN=target/release/cassette
          mkdir -p dist/man/man1 dist/completions
          cp "$BIN" dist/
          "$BIN" man --out-dir dist/man/man1
          "$BIN" completions bash > dist/completions/cassette.bash
          "$BIN" completions zsh  > dist/completions/_cassette
          "$BIN" completions fish > dist/completions/cassette.fish
          tar -czf "${{ matrix.artifact }}" -C dist .
```
No new third-party actions, so no new pins.

- [ ] **Step 2: Reproduce locally.** Run the same script body with `BIN=target/release/cassette` and `${{ matrix.artifact }}` replaced by `/tmp/…/cassette-test.tar.gz`, then `tar -tzf` it. Expect `./cassette`, `./man/man1/cassette.1`, `./man/man1/cassette-queue-write.1`, `./completions/_cassette`, and the rest. Don't commit the output.

- [ ] **Step 3: `docs/distribution.md`.** Sections:
  1. *What ships:* the tarball tree from Step 2.
  2. *Installing from the tarball:* `install -m755 cassette ~/.local/bin/`; `install -m644 man/man1/*.1 ~/.local/share/man/man1/`; bash → `~/.local/share/bash-completion/completions/cassette`; zsh → a dir on `$fpath` as `_cassette`; fish → `~/.config/fish/completions/cassette.fish`.
  3. *Installing with `cargo install`:* the same, but generated: `cassette completions zsh > …`, `cassette man --out-dir ~/.local/share/man/man1`.
  4. *Why runtime generation:* two sentences from the spec's Decision 1.
  5. *Releasing:* move the README's "Releasing a new version" steps and the "CI security: pinned Actions" section here verbatim.
  6. *Human step:* the first release after this change is the workflow's only real test. Check the attached tarball's listing.

- [ ] **Step 4: README.** Under "Build from source" add: "Shell completions and man pages: `cassette completions <shell>`, `cassette man`. See [docs/distribution.md](docs/distribution.md)." Replace the "For maintainers" body with a pointer to `docs/distribution.md`. Remove "man page and shell completions" from the "Ideas" bullet.

- [ ] **Step 5: CLAUDE.md.** In the `cli.rs` paragraph, add `completions <SHELL>` and `man [--out-dir]` to the subcommand list and a sentence: "`command()` is the clap `Command` `parse()` uses; `main()` hands it to `clap_complete`/`clap_mangen` before loading config, so generated docs cannot drift and a broken config cannot block them." Add both commands to the Commands block.

- [ ] **Step 6:** Remove the "Man page, shell completions, release packaging" section from `docs/follow-up.md`. Add a CHANGELOG `Unreleased → Added` bullet.

- [ ] **Step 7: Commit**

```bash
git add .github/workflows/release.yml docs/distribution.md README.md CLAUDE.md docs/follow-up.md CHANGELOG.md
git commit -m "build: ship man pages and completions in the release tarball"
```

- [ ] **Step 8: Tell the user** the remaining human step: publish a release (or a pre-release tag) and check the attached tarball's contents.
