# cassette

A freewriting TUI for your terminal. Each writing surface is a **cassette
tape**: you write on it, flip it over, label it, and when the session ends
your words are saved to markdown. Built for getting words out, not editing
them.

## Why cassette?

- **Typewriter focus** — the view scrolls so your cursor stays centered, and
  lines fade out as they drift from the one you're writing. What you just
  wrote stays soft in view; what you wrote a minute ago recedes. Keep moving
  forward.
- **Timed sessions and word goals** — `cassette -t 10 -w 500` gives you a
  countdown and a target. Progress winds across the screen from one tape reel
  onto the other; hitting the goal rings the terminal bell and lights the
  stats up green. The session never cuts you off mid-thought, and when you
  quit you get a recap: words, duration, pace.
- **Record mode** — `cassette -R` when you want the genre-classic constraint:
  no deletions, no cursor movement, the tape only rolls forward. Strictly
  opt-in — normal sessions keep full editing, because fixing a typo mid-flow
  is fine and being *unable* to is its own distraction. And if a timed or
  record session goes quiet, a gentle nudge reminds you the tape's still
  rolling — no punishment, no deleted words.
- **Multiple cassettes** — up to 36 independent writing surfaces in one
  session. Keep separate threads of thought separate; Tab between them. The
  focused cassette is full height, the rest minimize to their last line.
- **Every tape has two sides** — flip to side B with Ctrl+B for scratch
  space: an outline, a stray tangent, notes to future you. Each side keeps
  its own cursor and undo history, and both sides land in the saved file.
- **Topics and templates** — label any cassette with a topic mid-session
  (`t`), or start from a template: define `morning = ["gratitude",
  "priorities", "loose thoughts"]` in your config and `cassette -T morning`
  opens a labeled cassette for each.
- **A daily practice** — `cassette today` keeps one note per day and appends
  each sitting as its own session; `cassette stats` reads your streak and
  weekly/monthly word counts straight out of the notes' frontmatter. Left a
  thought unfinished? `cassette --resume` loads the note back onto the tape.
- **Mini vim** — insert and normal modes, `hjkl`/`w`/`b`/`0`/`$`/`gg`/`G`
  motions, `x`/`dd`, and undo. Enough vim to feel at home, not enough to
  tempt you into editing when you should be writing.
- **Your words are safe** — dirty sessions autosave every 30 seconds; the
  terminal is restored and the session saved on crashes, `kill`, and a
  closed terminal window alike; Ctrl+Z suspends cleanly with your words
  already flushed; and if a session dies before finishing, the next launch
  offers to resume the draft. Quitting writes clean markdown with YAML
  frontmatter — ready for your notes vault. Empty sessions write nothing.
- **Themes** — six built-ins (gruvbox, nord, dracula, solarized…), full
  custom themes from the config file, and a ghostty-style `cassette +themes`
  listing with color swatches. The default stays true to your terminal's own
  colors.

## Getting started

### Prebuilt binary

Download the latest release from the Releases tab. Only Linux is supported at
this time.

### Build from source

You'll need the [Rust toolchain](https://rustup.rs). Then:

```
cargo install --path .
```

Launch with `cassette` and just start typing — you're in insert mode. When
you're done, `Esc` then `q` saves and quits.

## The interface

Your text lives in the cassette at the top: a line-numbered window that
scrolls typewriter-style, keeping the cursor row centered and bright while
other rows fade with distance. The active side of the tape (`╡ SIDE A ╞`) and
the cassette's topic are woven into the separator above it. At the bottom: the
reel/stats bar (when a timer or word goal is set), a vim-style info line
(mode, line/column, character count, cassette n/m, side), and a key cheat
sheet that follows the current mode.

### Key bindings

| Keys | Mode | Action |
|---|---|---|
| type, `Enter` | insert | write; new line |
| `Ctrl+W` / `Ctrl+U` | insert | delete word / to line start |
| `Esc` | insert | to normal mode |
| `i a I A o O` | normal | back to insert (vim-style) |
| `h j k l`, `w b`, `0 $`, `gg G` | normal | motions (rows are display rows) |
| `x`, `dd`, `u` | normal | delete char / line, undo |
| `t` | normal | set the cassette's topic (Enter saves, blank clears, Esc cancels) |
| `q` | normal | quit and save |
| `Tab` / `Shift+Tab` | both | next / previous cassette |
| `Ctrl+N` | both | new cassette |
| `Ctrl+T` | both | set the cassette's topic |
| `Ctrl+B` (or `Shift+Enter`¹) | both | flip the cassette to its other side |
| `Ctrl+Z` | both | suspend to the shell (words flushed first); `fg` resumes |
| `Ctrl+C` | both | quit and save |

¹ on terminals with the kitty keyboard protocol (kitty, ghostty, foot, …).

Undo is per side, per cassette, and entering insert mode takes one snapshot —
so a single `u` takes back the whole burst you just typed. Pasting works the
same way: a bracketed paste lands as one edit, so one `u` takes back the
whole paste.

### Side B

Every cassette has a flip side. Side B is scratch space — outline the next
paragraph, dump a tangent, leave yourself a note — and all the side B cues are
woven in wherever you are: a `╡ SIDE B ╞` tag in the separator, an accented
line-number gutter, `· side B` in the info line. Word counts cover both
sides, and side B is saved under its own heading.

## Timed sessions and word goals

Start a timed session with `-t` (minutes), set a target with `-w` (words), or
combine them:

```
cassette -t 10
cassette -w 500
cassette -t 10 -w 500
```

The stats bar shows the countdown and `words / goal` between two tape reels;
as you progress, tape visibly winds from the supply reel onto the take-up
reel. When time runs out the timer turns red and the bell rings — but the
session keeps going so you can finish the thought. Hitting the word goal
rings the bell once and locks the stats green.

If a timed session goes quiet for ~10 seconds, the info line shows a gentle
nudge — *tape's still rolling — keep writing* — that clears on your next
keypress. Untimed sessions are never nudged.

### Record mode

```
cassette -R
cassette -R -t 10        # a classic ten-minute one-take
```

In record mode the tape only rolls forward: typing and Enter work, but
backspace, delete, cursor movement, and normal mode don't. You can still
switch cassettes, flip sides, and set topics — none of that is editing.
Quit with `Ctrl+C` (which saves, as always).

Record mode is opt-in by design. Plenty of freewriters stay in flow better
when they *can* fix a stray typo; the default session leaves editing alone.

## A daily practice

One note per day, however many sittings it takes:

```
cassette today                # opens (or continues) 2026-07-03.md
cassette today -T morning     # morning pages, straight into today's note
```

The first `today` session of the day creates a note named after the date in
the notes dir. Later sessions the same day append to it as their own
`## Session 2 — 14:30` section — nothing is renamed, nothing overwritten —
and the note's frontmatter word count keeps the running total. Set
`daily_format` in the config to change the filename pattern.

Your history is queryable from the frontmatter alone:

```
$ cassette stats
streak:      4 days
this week:   5 notes · 2183 words
this month:  11 notes · 4907 words
total:       48 notes · 22410 words · since 2026-05-02
```

The streak counts consecutive days with at least one note and doesn't break
until a full day is missed — an unwritten *today* is still yours to write.

### Picking a thought back up

```
cassette find                 # browse recent notes, newest first
cassette find gratitude       # …filtered by name, topic, or content
cassette --resume             # continue the most recently modified note
cassette --resume myjournal   # continue a specific note
```

`find` lists what's in the notes dir — date, word count, topics, and the
first line of each note — so you can spot the one to resume without opening
anything.

Resume loads a saved note back into the TUI — cassettes, topics, both sides,
cursor at the end of the text — and keeps writing to the same file. And if a
session dies without finishing (power loss, a killed terminal), its autosave
still carries a `draft: true` marker, so the next launch notices and offers
to resume it.

## Saving

On quit, the session is written as markdown. By default it goes to
`~/.local/share/cassette/notes/` with a timestamp filename:

```
~/.local/share/cassette/notes/2026-05-31T10-30-00.md
```

Each cassette becomes a `# Cassette n — topic` section with `## Side A` and
(when used) `## Side B` under it, prefixed by YAML frontmatter:

```markdown
---
date: 2026-05-31T10:30:00
timer: 10m
word_goal: 500
word_count: 347
cassettes: 2
---
```

`timer` and `word_goal` appear only when those flags were passed.

Name the file, write to a path, or print to stdout instead:

```
cassette myjournal            # ~/.local/share/cassette/notes/myjournal.md
cassette ~/Documents/draft.md # any path (anything containing a /)
cassette -o                   # print to stdout, write no file
```

If the name is taken, cassette saves to `myjournal_1.md` rather than
overwrite. Sessions with no words write no file at all. While you write,
dirty sessions autosave every 30 seconds, and the saved-on-crash guarantee
covers more than panics: SIGTERM and a closed terminal window (SIGHUP) both
save the session and restore the terminal on the way out, and Ctrl+Z flushes
your words to disk before handing control back to the shell.

After every session, a recap prints to the terminal — words, duration, pace
(for sessions over 30 seconds), and a per-cassette breakdown when you used
more than one:

```
347 words in 10:02 — 35 wpm
  gratitude: 120 · cassette 2: 227
```

## Configuration

Cassette reads `$XDG_CONFIG_HOME/cassette/config.toml` — that's
`~/.config/cassette/config.toml` on every platform, including macOS — on
startup. The file is never created for you — no config means all defaults —
so to customize anything:

```bash
mkdir -p ~/.config/cassette
$EDITOR ~/.config/cassette/config.toml
```

Every key is optional:

```toml
# Where session notes are saved (default: ~/.local/share/cassette/notes).
notes_dir = "/home/you/notes/cassette"

# Filename pattern for `cassette today`, in chrono strftime syntax
# (default: %Y-%m-%d, e.g. 2026-07-03.md).
daily_format = "%Y-%m-%d"

# Visible text rows per cassette, 2-40 (default: 5). The -l flag overrides this.
visible_lines = 8

# Color theme (default: your terminal's own colors). --theme <name> overrides
# this for one session.
theme = "gruvbox"

# Topic templates: `cassette -T morning` starts a session with one labeled
# cassette per topic. During a session, `t` in normal mode edits the focused
# cassette's topic.
[templates]
morning = ["gratitude", "priorities", "loose thoughts"]

# Custom themes: any subset of the color fields, as "#rrggbb" or ANSI
# color names (yellow, darkgray, ...). Unset fields keep the default look.
[themes.mine]
text = "#e6e1cf"          # focused cassette text
background = "#0f1419"    # focused cassette background
unfocused_bg = "#1c2328"  # minimized cassette background
unfocused_fg = "#8a9199"  # minimized cassette text
accent_a = "#ffd580"      # side A tag + line-number gutter
accent_b = "#5c6773"      # side B tag + line-number gutter
help_key = "#e6e1cf"      # key combos in the help line
help_text = "#5c6773"     # descriptions in the help line
```

### Themes

List every available theme (built-in and user-defined) with color swatches:

```
cassette +themes
```

Built-ins: `default`, `dracula`, `gruvbox`, `nord`, `solarized-dark`,
`solarized-light`. Pick one in the config (`theme = "nord"`) or per session:

```
cassette --theme nord
```

A `[themes.<name>]` entry named after a built-in overrides just the fields you
set — for example, keep gruvbox but change the side A accent:

```toml
[themes.gruvbox]
accent_a = "#ff8800"
```

## CLI reference

```
Usage: cassette [OPTIONS] [NAME]

Arguments:
  [NAME]         output note name or path
                 (default: timestamped file in the notes dir)

Options:
  -t <MINUTES>   countdown timer in minutes
  -w <WORDS>     word goal (winds the tape reel)
  -l <LINES>     visible text rows per cassette (2-40)
  -T <TEMPLATE>  start with one cassette per topic from the named
                 [templates] entry in config.toml
  --theme <NAME> color theme for this session (overrides config)
  -R, --record   record mode: no deletions, the tape only rolls forward
  --resume [FILE] load a saved note back into the TUI and keep writing
                 (default: the most recently modified note)
  -o, --output   print to stdout on quit instead of writing a file
  -h, --help     print this help
  -V, --version  print version

Actions:
  today          open today's note (named by date); a later session the
                 same day appends as a new '## Session' section
  stats          streak, weekly/monthly notes and words, totals — read
                 from the frontmatter of everything in the notes dir
  find [TEXT]    list recent notes newest-first (date, words, topics,
                 first line); TEXT filters by name, topic, or content
  +themes        list available themes (built-in and from config.toml)
```

## For maintainers

### Releasing a new version

1. Bump `version` in `Cargo.toml` (e.g. `0.7.0`) and merge all changes to `main`.
2. On GitHub, go to **Releases → Draft a new release**.
3. Create a new tag (e.g. `v0.7.0`) targeting `main`.
4. Write release notes, then click **Publish release**.

The [Release workflow](.github/workflows/release.yml) will automatically build the Linux binary
and attach `cassette-linux-x86_64.tar.gz` to the release. First run may take longer due to cache
warming; subsequent releases should be faster.

### CI security: pinned Actions

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

## Ideas

- **Publishing the practice** — packaging for the AUR and crates.io, a demo
  GIF, man page and shell completions, macOS builds.
