# Man page, shell completions, release packaging

> Written unattended overnight on 2026-09-24 from `docs/follow-up.md`'s "Man page, shell
> completions, release packaging" entry (deferred out of Phase 6). Judgement calls are under
> **Decisions**.

## Purpose

Generate a man page and shell completions from the real `clap` tree, so they cannot drift from
the command surface, and ship them in the release tarball.

## The follow-up's premise, corrected

The entry says `build.rs` cannot reach the clap tree because "`cli.rs` uses nine `crate::` paths
directly in its derive". Verified against `src/cli.rs`: **the derive tree has none.** `Cli`,
`Command`, `QueueAction`, `WriterAction`, `SessionAction` and the three `*Arg` value enums
(lines ~137–450) reference only `clap` and `std`. The `crate::` paths all live in the other half
of the file:

- the lowered types `main()` consumes (`Args`, `QueueCmd`, `WriterCmd`: `queue::Placement`,
  `queue::Side`, `queue::WriteMode`, `queue::StatusFilter`, `queue::edit::MoveAnchor`,
  `store::writers::Kind`),
- the three `From<*Arg>` impls,
- `into_args`.

The `WriterKindArg` pattern the entry proposes as option 2 was already applied to the whole
derive tree. So `include!("src/cli.rs")` fails, but moving the derive half into its own file
would make it includable with no change to the queue or writer surface. Option 2 is cheap, not
invasive.

That changes the trade-off, but not the answer. See Decision 1.

## Design: generate at runtime (option 1)

Two new subcommands, following CLAUDE.md's rule that every mode is a subcommand:

```
cassette completions <SHELL>        # bash | zsh | fish | elvish | powershell → stdout
cassette man [--out-dir <DIR>]      # no flag: cassette.1 to stdout
                                    # --out-dir: cassette.1 plus one page per subcommand
                                    #   (cassette-queue.1, cassette-queue-write.1, …)
```

- `cli.rs` exposes `pub fn command() -> clap::Command { <Cli as clap::CommandFactory>::command() }`.
  Both generators take that, so they describe exactly what `parse()` parses.
- `completions` calls `clap_complete::generate(shell, &mut cli::command(), "cassette", &mut stdout)`.
  `clap_complete::Shell` is itself a `ValueEnum`, so the argument is validated by clap and lists
  its choices in `--help`.
- `man` uses `clap_mangen::Man::new(cmd).render(&mut stdout)`. With `--out-dir` it uses
  `clap_mangen::generate_to(cmd, dir)`, which writes a page per subcommand. The queue surface
  is where the detail lives, and one page listing subcommand names would not document it.
- Both run before the store or terminal is touched, like `themes`: no data dir is created, and
  `--json` is ignored.
- Exit codes: 0; clap's own 2 for a bad shell name; 1 if `--out-dir` cannot be written.

### Release packaging

`.github/workflows/release.yml` gains a step after the build that runs the just-built binary:

```
dist/
  cassette
  man/man1/cassette.1, cassette-*.1
  completions/cassette.bash, _cassette, cassette.fish
```

The tarball is made from `dist/` rather than `-C target/release cassette`. The binary is still at
the tarball root, so anyone extracting and copying `cassette` keeps working.

`docs/distribution.md` is created (CLAUDE.md already cites it and it does not exist). It holds
the tarball layout, where each file installs per platform, how `cargo install` users get
completions (`cassette completions zsh > ~/.zfunc/_cassette`), and the release procedure, which
moves out of the README's "For maintainers" section. The README keeps a one-line pointer.

## Decisions

1. **Runtime generation (option 1), not `build.rs`.** The deciding fact is how people install:
   the README documents `cargo install --path .` and a prebuilt tarball. `cargo install` ships
   only the binary. Anything `build.rs` writes lands in `target/…/build/cassette-*/out/` and
   never reaches that user. Runtime generation is the one option that serves both install paths
   with the same bytes. Its cost is two runtime dependencies. `clap_mangen` pulls in `roff`,
   which is small. `clap_complete` is a few hundred KB of release binary; the plan measures it,
   and if it exceeds ~500 KB it is flagged rather than silently accepted. Option 2 (now cheap,
   see above) remains the fallback if the size is judged too high. Option 3 (a lib target)
   stays rejected for the reason CLAUDE.md gives.
2. **`man` writes the full per-subcommand set only with `--out-dir`.** Stdout can carry one page,
   and `man cassette` is what people type. The set is what packagers install.
3. **Visible subcommands, not hidden flags.** `cargo install` users need to discover
   `completions`, and it appears in `--help`. `man` is visible for symmetry. The alternative
   (`--generate-man` style flags) would bring back top-level flags that act like modes, which
   the Phase 6 CLI shape removed.
4. **No drift test.** Drift is impossible by construction, since both generators read
   `Cli::command()`. A test that snapshots the output would only make every `--help` wording
   change fail twice.
5. **Linux-only release unchanged.** The workflow's commented macOS matrix entry is left alone.
   The new step is OS-agnostic, so enabling it later needs nothing more.

## Human steps

- Publishing a GitHub release to exercise the workflow change. The workflow cannot be run
  locally. The plan reproduces its commands in a local script, but the first real release is
  the only true test.

## Out of scope

- AUR / crates.io / Homebrew packaging (README "Ideas"). `docs/distribution.md` is where they
  would go.
- `cassette completions --install`. Where completions live varies by shell and distro, and
  writing into the user's dotfiles is not this binary's job.

## Acceptance criteria

- `cassette completions zsh` exits 0 and its output mentions `queue` and `write`. The same holds
  for bash and fish.
- `cassette completions nope` exits 2.
- `cassette man | head` starts with a `.TH` line naming `cassette`.
- `cassette man --out-dir D` writes `D/cassette.1` and `D/cassette-queue-write.1`.
- Neither command creates the data dir (`CASSETTE_DATA_DIR` pointing at a non-existent path is
  still non-existent afterwards).
- The release workflow builds `dist/` with the layout above. A local script running the same
  commands produces it.
- `docs/distribution.md` exists. The README links to it. CLAUDE.md's `cli.rs` paragraph names
  `command()` and the two subcommands. The follow-up entry is removed.
- Release binary size before and after is recorded in the commit message.
- `cargo test`, `cargo clippy --all-targets -- -D warnings` green.
