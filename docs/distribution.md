# Distribution

How `cassette` ships: what is in a release, where each piece installs, and how a release is cut.

## What ships

Each GitHub release attaches `cassette-linux-x86_64.tar.gz`:

```
./cassette                         the binary
./man/man1/cassette.1              top-level man page
./man/man1/cassette-*.1            one page per subcommand (cassette-queue-write.1, …)
./completions/cassette.bash        bash
./completions/_cassette            zsh
./completions/cassette.fish        fish
```

The binary stays at the tarball root, so extracting and copying `cassette` works as it always has.

## Installing from the tarball

```bash
install -m755 cassette ~/.local/bin/
install -Dm644 -t ~/.local/share/man/man1/ man/man1/*.1
install -Dm644 completions/cassette.bash ~/.local/share/bash-completion/completions/cassette
install -Dm644 completions/_cassette     ~/.zfunc/_cassette        # any directory on $fpath
install -Dm644 completions/cassette.fish ~/.config/fish/completions/cassette.fish
```

For zsh, `~/.zfunc` must be on `$fpath` before `compinit` (`fpath=(~/.zfunc $fpath)` in
`~/.zshrc`).

## Installing with `cargo install`

`cargo install` ships only the binary, so generate the rest from it:

```bash
cassette man --out-dir ~/.local/share/man/man1
cassette completions bash > ~/.local/share/bash-completion/completions/cassette
cassette completions zsh  > ~/.zfunc/_cassette
cassette completions fish > ~/.config/fish/completions/cassette.fish
```

`cassette man` with no flag prints `cassette.1` to stdout; `--out-dir` writes every page.
`completions` also accepts `elvish` and `powershell`.

## Why the binary generates its own docs

Both generators read `cli::command()`, the same clap tree that parses the command line, so the
man pages and completions cannot drift from the real surface. They are generated at runtime
rather than by `build.rs` because `build.rs` output never reaches a `cargo install` user; the cost
is two small runtime dependencies (`clap_complete`, `clap_mangen`: about 410 KB of release
binary). See `docs/superpowers/specs/2026-09-24-man-and-completions-design.md`.

## Releasing a new version

1. Bump `version` in `Cargo.toml` (e.g. `0.7.0`) and merge all changes to `main`.
2. On GitHub, go to **Releases → Draft a new release**.
3. Create a new tag (e.g. `v0.7.0`) targeting `main`.
4. Write release notes, then click **Publish release**.

The [Release workflow](../.github/workflows/release.yml) will automatically build the Linux binary
and attach `cassette-linux-x86_64.tar.gz` to the release. First run may take longer due to cache
warming; subsequent releases should be faster.

## CI security: pinned Actions

The release workflow pins every third-party GitHub Action to a specific commit SHA rather than a
mutable tag like `@v2` or `@stable`. This prevents a compromised action repository from silently
pushing malicious code under an existing tag and having it run in your CI pipeline — a class of
attack that has affected several popular actions (tj-actions, reviewdog, and others) in 2025–2026.

The pins currently in use:

| Action | Tag | Pinned SHA |
|---|---|---|
| `dtolnay/rust-toolchain` | `stable` | `29eef336d9b2848a0b548edc03f92a220660cdb8` |
| `softprops/action-gh-release` | `v2` | `3bb12739c298aeb8a4eeaf626c5b8d85266b0e65` |

**Updating a pin:** when you want to pick up a newer version of an action, resolve the new SHA and
update the workflow manually:

```bash
# find the SHA the tag currently points to
gh api repos/softprops/action-gh-release/tags \
  --jq '.[] | select(.name=="v2") | .commit.sha'
```

Then replace the SHA in `.github/workflows/release.yml` and leave a `# v2` comment so the intent
stays readable.

## Human step after changing the release workflow

The workflow can only be exercised by publishing a release. After the first release that ships
man pages and completions, download the tarball and check its listing
(`tar -tzf cassette-linux-x86_64.tar.gz`) against "What ships" above.
