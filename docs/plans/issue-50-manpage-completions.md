# Plan: man page and shell completions (chainlink #50)

**Deliverable:** `cassette.1` man page + bash/zsh/fish completions, shipped in the
release tarball and installable by packages (#48).
**Agent-executable:** fully.

## Context

- Arg parsing is **hand-rolled** in `src/main.rs::parse_args` (no clap), so there is
  no generator — man page and completions are written by hand. The full CLI surface
  (verify against `parse_args` and the `USAGE` const in `src/main.rs` before writing):
  - Flags: `-t <min>`, `-w <words>`, `-l <rows>`, `-T <template>`, `--theme <name>`,
    `-R/--record`, `--resume [file]`, `-o/--output`, `-h/--help`, `-V/--version`
  - Actions: `today`, `stats`, `+themes`
  - Positional: `[note-name]` (mutually exclusive with `today`/`stats`)
- Built-in themes for completion: `default dracula gruvbox nord solarized-dark
  solarized-light` (see `src/theme.rs::builtins`). User themes come from config —
  completions can optionally parse `[themes.*]` from
  `${XDG_CONFIG_HOME:-~/.config}/cassette/config.toml`; same file's `[templates]`
  keys for `-T` completion.

## Steps

1. Write the man page in **scdoc** source at `doc/cassette.1.scd` (readable,
   diffable; `pacman -S scdoc`) and check in both the source and the generated
   `doc/cassette.1` (`scdoc < doc/cassette.1.scd > doc/cassette.1`) so builds don't
   need scdoc. Sections: NAME, SYNOPSIS, DESCRIPTION (one paragraph),
   OPTIONS, ACTIONS (today/stats/+themes), KEY BINDINGS (condense from README:
   both-modes chords, insert, normal, record), FILES (config path, notes dir,
   config keys: `theme`, `visible_lines`, `daily_format`, `notes_dir`,
   `[templates]`, `[themes.<name>]`), EXIT STATUS (0 ok, 2 usage/config error),
   SEE ALSO (repo URL). Verify: `man ./doc/cassette.1` renders,
   `mandoc -T lint doc/cassette.1` (or `groff -ww`) is clean.
2. Write completions under `completions/`:
   - `completions/cassette.bash` — complete flags/actions; theme names after
     `--theme`; template names after `-T` (parse config if easy, else skip values).
   - `completions/_cassette` — zsh `#compdef cassette`, `_arguments` covering every
     flag with descriptions, theme-name completion for `--theme`, file completion
     for `--resume` (files in the notes dir), no completion after `-t/-w/-l`.
   - `completions/cassette.fish` — `complete -c cassette ...` per flag/action with
     descriptions; `complete -c cassette -l theme -xa "(cassette +themes | ...)"`
     is acceptable if `+themes` output is parseable, else hardcode built-ins.
   Test each in its shell: `bash -c 'source ...; complete -p cassette'`,
   `zsh -f` with fpath pointing at completions/, `fish -c 'complete -C"cassette -"'`.
3. Ship in releases — extend the "Package artifact" step in
   `.github/workflows/release.yml` to include the docs:
   ```yaml
   run: |
     mkdir -p pkg && cp target/release/cassette pkg/
     cp doc/cassette.1 pkg/ && cp -r completions pkg/
     tar -czf "${{ matrix.artifact }}" -C pkg .
   ```
4. README: add an "installing the man page & completions" note (paths:
   `/usr/share/man/man1/`, bash-completion dir, zsh `site-functions`, fish
   `vendor_completions.d`). Update the AUR plan's commented install lines (#48)
   are already written to match these paths — keep them in sync.
5. Whenever a flag is added later, `doc/cassette.1.scd`, the three completion
   files, and `USAGE` all change together — add a line saying so to CLAUDE.md's
   Conventions section.

## Acceptance criteria

- `man ./doc/cassette.1` renders with all flags/actions/keybindings; lint-clean.
- Completions load without error in bash, zsh, fish and complete flags + theme names.
- Release tarball contains binary + man page + completions.
- Every flag in `parse_args` appears in man page and all three completion files.
