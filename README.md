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
- **A daily practice** — `cassette today` opens (or continues) the session
  aliased with today's date, so every sitting the same day lands in one
  place; `cassette stats` reads your streak and weekly/monthly word counts
  straight out of the session store. Left a thought unfinished? `cassette
  resume` loads the most recent session back onto the tape.
- **Mini vim** — insert and normal modes, `hjkl`/`w`/`b`/`0`/`$`/`gg`/`G`
  motions, `x`/`dd`, and undo. Enough vim to feel at home, not enough to
  tempt you into editing when you should be writing.
- **Your words are safe** — dirty cassettes autosave every 30 seconds; the
  terminal is restored and the session saved on crashes, `kill`, and a
  closed terminal window alike; Ctrl+Z suspends cleanly with your words
  already flushed. Each cassette is its own file with YAML frontmatter,
  written straight to the session store as you write — nothing waits for
  quit. Empty sessions leave no trace.
- **Shared with agents** — cassette writes into the same session store its
  `queue`/`session`/`writer` commands already read, so a human at the
  keyboard and one or more agents working the queue can write different
  cassettes of the same session side by side, each seeing the other's words.
  The focused cassette's lock is held for as long as it stays focused; if
  another writer already holds it, it opens read-only instead of losing your
  keystrokes.
- **Themes** — six built-ins (gruvbox, nord, dracula, solarized…), full
  custom themes from the config file, and a ghostty-style `cassette themes`
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

**Focusing a cassette holds its lock**, for as long as it stays focused —
there is no idle timeout, so tabbing away or leaving cassette open overnight
does not give it back on its own. If an agent (or another `cassette`
process) is already holding it, the cassette opens **read-only**: the info
line shows `-- READ ONLY --`, the help row says why, and every keystroke is
silently dropped rather than lost at the next flush. Since agents drive the
queue entirely through the CLI and never need the TUI running, the norm is
to close a session when you're done with it rather than leave the tape
loaded — that's what actually releases the lock.

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

One session per day, however many sittings it takes:

```
cassette today                # opens (or continues) the session aliased 2026-07-03
cassette today -T morning     # the day's first launch, seeded with topics
```

The first `today` launch of the day creates a session aliased with the
date; later launches the same day find it by that alias and resume it —
cassettes, topics, both sides, cursor at the end of the text. Set
`daily_format` in the config to change the alias's date format.

`-T` seeds a *new* session, so it pairs with the first `today` of the day;
once the day has a session there is nothing to seed and `today -T` says so
rather than opening the day's work with the topics quietly dropped.

Your history is queryable straight from the session store:

```
$ cassette stats
streak:      4 days
this week:   5 notes · 2183 words
this month:  11 notes · 4907 words
total:       48 notes · 22410 words · since 2026-05-02
```

The streak counts consecutive days with at least one session and doesn't
break until a full day is missed — an unwritten *today* is still yours to
write. **`stats` and `find` read only the session store**: a session's words
count from the moment it's created there, and totals started fresh when the
TUI moved onto the store. Notes written before that move, still sitting in
`~/.local/share/cassette/notes/`, are no longer counted by either command —
they were not migrated in, on purpose — but nothing about them changed: the
files are untouched, still on disk, and still plain markdown you can open,
grep, or paste into an editor same as always.

### Picking a thought back up

```
cassette find                 # browse recent sessions, newest first
cassette find gratitude       # …filtered by alias, topic, or content
cassette resume               # continue the most recently created session
cassette resume myjournal     # continue the session aliased myjournal
cassette resume 01K5GQ2R8V…   # …or by the id `find` printed
cassette new myjournal        # start a *new* session aliased myjournal
```

`find` lists what's in the session store — date, word count, topics, and the
first line of the highest-priority cassette — so you can spot the session to
resume without opening anything. Each row shows the session's id, ready to
paste into `--session` or straight back into `resume`.

`new <NAME>` always starts a fresh session; it does not look for an existing
alias by that name the way the old flat-note `new` used to resume a
same-named file. To continue earlier work, reach for `resume <name>`
instead — aliases aren't unique, so it picks the most recently created
session carrying that alias.

Resume loads a session back into the TUI — cassettes, topics, both sides,
cursor at the end of the last one, all still individually lockable by
whichever writer opens them — and keeps writing to the same session. The
session's stats start from zero on resume: the word-goal reel and the
closing recap count only what you write this sitting (`12 new words in 4:10
(773 total)`).

## Saving

Cassette does not batch your session into one file at the end. Each
cassette is its own file in the session store: the focused one autosaves
every 30 seconds while you write, and the moment you `Tab`/`Shift+Tab`
away, the cassette you're leaving is flushed and its lock released *before*
the next one's is taken — so nothing waits on the 30-second timer to reach
disk when you move on. The saved-on-crash guarantee covers more than
panics: SIGTERM and a
closed terminal window (SIGHUP) both flush the focused cassette and restore
the terminal on the way out, and Ctrl+Z flushes before handing control back
to the shell. Sessions with no words leave no trace: a session this run
created, into which nothing was written by this process or by anyone else,
is removed rather than littering `session list`.

Each cassette file carries YAML frontmatter — its id, topic, queue
priority, status, sticky lock, who created and last wrote it, and when —
over a body with `## Side A` and (when used) `## Side B`:

```markdown
---
id: 01M2QSSJG751F2KCGP17K9GQVY
topic: morning pages
priority: 10
status: open
locked_by:
created_by: 01M2QSSJFSWSZ7BW9J4XT7FKD6
last_writer: 01M2QSSJFSWSZ7BW9J4XT7FKD6
updated_at: 2026-09-17T13:44:43Z
---

## Side A

writing about the morning
```

Print to stdout instead of the store, writing nothing at all:

```
cassette -o
```

Because `-o` persists nothing, it names no session: combining it with
`resume`, `new` or `today` is a usage error rather than a silently ignored
subcommand.

After every session, a recap prints to the terminal — words, duration, pace
(for sessions over 30 seconds), and a per-cassette breakdown when you used
more than one:

```
347 words in 10:02 — 35 wpm
  gratitude: 120 · cassette 2: 227
```

## The session store

The TUI and the `queue`/`writer`/`session` subcommands write into the same
**session store**, where each cassette is its own file, so a human at the
keyboard and one or more agents can write different cassettes of the same
session without overwriting each other.

**Focus means held.** While a cassette is focused in the TUI, its lock is
held — the same `.locks/<id>` flock `queue write` takes, just kept open for
as long as focus stays there rather than acquired-and-released per write.
There is no idle timeout: leaving the TUI open on a cassette holds it
indefinitely, so the norm is to close a session (quit) once you're done
with it rather than leave it running in a background tab. Agents never need
this to change — they drive the queue entirely through the CLI, which
acquires each cassette's lock only for the moment it writes and never holds
one across commands. Tabbing to a different cassette flushes and releases the one you're leaving
before taking the next one's lock; if that next cassette's lock is already
held live by someone else — another `cassette` process, most likely — it
opens **read-only** instead of refusing to focus it at all: the info line
shows `-- READ ONLY --`, the help row explains why, and keystrokes are
dropped rather than typed into a buffer that can't be saved. This is the
live `.locks/<id>` flock, not a `queue lock` sticky claim — a sticky lock
only ever restricts an agent's writes through the CLI, never the human at
the keyboard.

**There is no active session.** Cassette never remembers which session you were
last using: every `queue` command takes an explicit `--session <id>`, and if
you've lost track of one, `cassette session list` prints every session's id
(and its alias, if it has one) so you can pass it again. An **alias** (set with
`session new --alias <name>` or `session alias <id> <name>`) is a display label
shown next to the id in `session list` — it is never accepted in place of an id
anywhere, including `--session` itself; sessions are named by id only. The
TUI is the one exception to "always pass `--session`": launching `cassette`
resolves or creates a session for you (a bare launch or `new <NAME>` always
starts a new one; `today` creates or finds one by date alias; `resume`
finds an existing one) and holds it only for that process's lifetime — it
still never becomes some persistent "current session" the CLI picks up
afterwards.

Every command that takes a session id (`queue …`, `session alias`) checks it
before touching the store: it must be a well-formed 26-character ULID naming a
session that already exists. A malformed id and an id for a session nobody
created are both usage errors (exit 2), reported differently so you can tell a
typo from a stale id. Only `session new` ever creates a session — no command
brings one into being as a side effect of being handed an unfamiliar id.

The store lives under `~/.local/share/cassette/`, or wherever
`$CASSETTE_DATA_DIR` points:

```
~/.local/share/cassette/
  writers.toml                     # who may be credited with writing
  .locks/writers                   # lock anchor for the registry
  sessions/<session id>/
    session.toml
    cassettes/<slug>-<id>.md       # one file per cassette
    .locks/<id>                    # lock anchor, one per cassette
```

The whole directory is created `0700` — freewriting is private by default, and
the mode is set as the directory is created rather than tightened afterwards, so
it is never briefly readable by other users on a shared machine.

Writing a cassette takes a lock on its `.locks/<id>` anchor first. If someone
else holds it you get exit code 3 and a message naming them, and the right move
is to write a different cassette rather than wait. The lock is held by the
kernel, so it is released even if a writer is killed outright — there is nothing
to clean up and no stale-lock state to repair.

### Command surface

```
session new [--alias <NAME>]                       # create a session, print its id
session list [--all]                                # sessions newest-first (15 by default)
session alias <ID> <ALIAS>                          # set/replace a session's display label

writer register --name <NAME> --kind human|agent    # register a writer; kind is fixed at registration
writer list                                          # list registered writers
writer whoami                                        # show the writer this invocation acts as

queue new <TOPIC> --session <ID> [--first|--last|--priority <N>]
queue list --session <ID> [--status open|closed|all] [--since <TIME>]
queue show <ID> --session <ID>
queue next --session <ID>                            # id of the next open, unlocked cassette
queue write <ID> --session <ID> [--side a|b] [--append|--replace]
                                                      # write one side of a cassette's body, read from stdin
                                                      # (side: a, default; write mode: replace, default)
queue close <ID> --session <ID> [-m <TEXT>]
queue reopen <ID> --session <ID>
queue move <ID> --session <ID> (--before|--after) <ID>
queue lock <ID> --session <ID>                       # sticky-lock a cassette to yourself (human-only)
queue unlock <ID> --session <ID>                     # clear a cassette's sticky lock, whoever holds it (human-only)
```

Add `--json` to any command (global, alongside `-t`/`-w`/etc.) to get
machine-readable output — see "Machine-readable output" below.

Every `queue` command that mutates something also resolves a writer identity
(`--writer <NAME>`, else `$CASSETTE_WRITER`, else `$USER`). Naming a writer
explicitly with `--writer` or `$CASSETTE_WRITER` is a claim about identity, so
an unregistered name is a usage error (exit 2) rather than silently creating a
second, privileged (`human`) identity; only an unregistered `$USER` bootstraps
a new writer on first use. `queue list` and `queue show` need no identity at
all — they attribute nothing.

Exit codes beyond the usual 0/1/2: **3** another writer currently holds the
cassette's lock (try a different one, or wait); **4** the cassette carries a
sticky `locked_by` claim and the acting writer is an agent, so only a human may
write or close it — or a human's `queue lock` found it already claimed by a
*different* writer; **5** `queue next` found no open cassettes at all; **6**
`queue new`/`queue reopen` would exceed the session's open-cassette cap
(`max_open`, config key, default 36). An agent invoking `queue lock` or
`queue unlock` at all is exit **2**, not 4 — it has misused the CLI, not run
into someone else's claim.

Exit 1 is reserved for I/O failures the caller cannot fix by trying a
different argument — a `session.toml` cassette cannot read because of
permissions, say, rather than an unknown or malformed `--session`, which is
exit 2. The distinction matters most under `--json`, where an agent branches
on `code`: exit 2 says "check what you passed in and retry"; exit 1 says
"something is wrong with the store itself — escalate instead of looping".

### Machine-readable output (`--json`)

Add `--json` to any `queue`, `session`, or `writer` invocation. On success the
three read-only queue commands differ in shape: `queue list --json` wraps its
results in the full `{session, cassettes, unreadable}` contract shown below,
while `queue show --json` and `queue next --json` each emit a **bare cassette
object** — the same shape as one entry of `list`'s `cassettes` array, not
wrapped in a `{session, cassettes, unreadable}` envelope (see the example
below). Don't script `.cassettes[0]` against `show`/`next` output — there is
no `cassettes` key there.

`session list --json`, `writer list --json`, and `writer whoami --json` emit
the same **prose** those commands always have — no promise is broken (the
JSON-success contract above only names the three queue reads), but
`cassette session list --json | jq` will fail to parse, so don't expect JSON
from them.

Every mutating command (`queue new`, `write`, `close`, `reopen`, `move`,
`lock`, `unlock`, and the `session`/`writer` commands) keeps the same exit
code and successful output it has without `--json`: `queue new` still prints
the new cassette's bare ULID and nothing else, and every other mutating
command prints nothing extra. **Any** command, on failure, emits a one-line
`{"error", "code"}` envelope to stdout instead of the usual stderr prose, so
an agent reading only stdout still gets a parseable failure:

```
$ cassette queue list --session 01AAAAAAAAAAAAAAAAAAAAAAAA --json
{"code":2,"error":"no session '01AAAAAAAAAAAAAAAAAAAAAAAA' — `cassette session list` shows what exists"}
```

A successful `queue list --json` looks like this (one real session, one
cassette, captured from a live run):

```
$ cassette queue list --session 01M2QSSJG2CG8XQYG7PVH3E59H --json
{"session":{"id":"01M2QSSJG2CG8XQYG7PVH3E59H","alias":"demo"},"cassettes":[{"id":"01M2QSSJG751F2KCGP17K9GQVY","topic":"morning pages","priority":10,"status":"open","words":4,"busy":false,"sticky_lock":null,"created_by":{"name":"joseph","kind":"human"},"last_writer":{"name":"agent-1","kind":"agent"},"waiting_on":"human","updated_at":"2026-09-17T13:44:43Z","side_a":"writing about the morning","side_b":""}],"unreadable":0}
```

`queue show --json` and `queue next --json` are bare cassette objects, not
wrapped in `{session, cassettes, unreadable}` (a different demo session,
captured from a live run):

```
$ cassette queue show 01M2RFZEC6QQXWNRS950HKSBP0 --session 01M2RFZEC00N0WPKKRBBVZTD26 --json
{"id":"01M2RFZEC6QQXWNRS950HKSBP0","topic":"morning pages","priority":10,"status":"open","words":4,"busy":false,"sticky_lock":null,"created_by":{"name":"joseph","kind":"human"},"last_writer":{"name":"agent-1","kind":"agent"},"waiting_on":"human","updated_at":"2026-09-17T20:12:24Z","side_a":"writing about the morning\n","side_b":""}

$ cassette queue next --session 01M2RFZEC00N0WPKKRBBVZTD26 --json
{"id":"01M2RFZEC6QQXWNRS950HKSBP0","topic":"morning pages","priority":10,"status":"open","words":4,"busy":false,"sticky_lock":null,"created_by":{"name":"joseph","kind":"human"},"last_writer":{"name":"agent-1","kind":"agent"},"waiting_on":"human","updated_at":"2026-09-17T20:12:24Z","side_a":"writing about the morning\n","side_b":""}
```

`unreadable` counts cassette files the store could not parse — the same count
the prose listing shows as `N unreadable`, never silently dropped. `busy` is
whether the cassette's advisory lock is currently held by a live writer;
`sticky_lock` is the durable claim set by `queue lock` (see below), `null`
when there is none. `waiting_on` is the inverse of `last_writer`'s kind
(`"human"` wrote last → `"agent"` is up next), or `null` when the last writer
can't be resolved against `writers.toml`. `side_a`/`side_b` split the body on
the `## Side A`/`## Side B` headings both the TUI and `queue write --side b`
write; `side_b` is `""` for a cassette nobody has written to on that side.

Once a cassette carries a sticky lock, `sticky_lock` is populated and an
agent's write against it fails through the same envelope:

```
$ echo "agent tries again" | cassette queue write --session 01M2QSSJG2CG8XQYG7PVH3E59H 01M2QSSJG751F2KCGP17K9GQVY --writer agent-1 --json
{"code":4,"error":"cassette is locked by '01M2QSSJFSWSZ7BW9J4XT7FKD6' — only a human may write it"}
```

### The sticky lock

`queue lock`/`queue unlock` are how a human says *hands off this one* to
every agent, independent of who is actively holding the advisory flock at
any given instant:

```
queue lock <ID> --session <ID>       # claim it — sets locked_by to you
queue unlock <ID> --session <ID>     # clear it, whoever holds it
```

**Only a human writer may set or clear the lock.** An agent calling either
command exits 2 — misuse of the CLI, not a claim to escalate over. Locking
is idempotent for its own holder (locking a cassette you already hold is a
no-op, exit 0) and `unlock` on a cassette that isn't locked is likewise a
no-op — neither writes the file. Locking a cassette a *different* writer
already holds is exit 4; unlocking always succeeds regardless of who holds
it, so a human can always take back a cassette an agent or another human
claimed and went quiet on.

Once set, the lock has two effects on an agent, and none on a human:

- **`queue next` skips it.** A sticky-locked cassette is invisible to the
  next-work-item query — the whole point of removing something from an
  agent's queue — even while it is otherwise open and unlocked at the flock
  level.
- **`queue write` and `queue close` refuse it**, exit 4, for an agent. A
  human may write or close a locked cassette freely; the lock only ever
  restricts agents, never other humans, so one terminal session can't lock
  another human out of their own work.

The lock is orthogonal to the advisory `.locks/<id>` flock `queue write`
already takes: `busy` (someone actively writing right now) and
`sticky_lock` (someone has durably claimed it) are independent fields in
the `--json` contract and can be true/set in any combination.

### `writers.toml` is managed by cassette, not by you

Every cassette records *who* created it and who wrote it last, by writer id.
`writers.toml` is what turns those ids back into a name and a kind
(`human` or `agent`).

**Avoid editing it by hand, and never while cassette is running.** It is not
protected by file permissions — the reason is how it is written:

- Cassette takes a lock and rewrites the **whole file** on every change. Your
  editor does not take that lock, so an edit saved while a write is in flight is
  silently overwritten in full.
- If the file does not parse, writes fail loudly rather than quietly starting
  over — which is the safe behaviour, but it does mean a stray keystroke stops
  you writing until you fix it.
- Deleting an entry orphans every cassette that credits it: the id stays in
  those files and no longer resolves to anyone.
- Reordering entries achieves nothing. The file is rewritten in a fixed order
  (by writer id, which is roughly registration order), so any hand-sorting is
  normalised away on the next write.

There is one edit that is currently legitimate: correcting a writer's `kind` if
it was registered wrongly. A command for that may come later; until then, do it
while cassette is not running.

## Configuration

Cassette reads `$XDG_CONFIG_HOME/cassette/config.toml` — that's
`~/.config/cassette/config.toml` on every platform, including macOS — on
startup. The file is never created for you — no config means all defaults —
so to customize anything:

```bash
mkdir -p ~/.config/cassette
$EDITOR ~/.config/cassette/config.toml
```

Every key is optional. `notes_dir` (from before the session store) still
parses if you have it set, but nothing reads it any more — `stats`, `find`,
and every session cassette now live under `$CASSETTE_DATA_DIR`/the XDG data
default instead:

```toml
# Alias pattern for `cassette today`'s session, in chrono strftime syntax
# (default: %Y-%m-%d, e.g. today's session is aliased 2026-07-03).
daily_format = "%Y-%m-%d"

# Most cassettes one session may hold open at once (default: 36).
max_open = 36

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
cassette themes
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
cassette — a freewriting TUI

Usage: cassette [OPTIONS] [COMMAND]

Commands:
  new      start a session in a named note
  today    open today's note, named by date
  resume   load a saved note back into the TUI (default: most recently modified)
  stats    streak, weekly/monthly notes and words, totals
  find     list recent notes newest-first; TEXT filters by name, topic, or content
  themes   list available themes (built-in and from config.toml)
  queue    work with the shared cassette queue
  writer   register and inspect writers
  session  create and inspect sessions

Options:
  -t <MINUTES>         countdown timer in minutes
  -w <WORDS>           word goal (winds the tape reel)
  -l <LINES>           visible text rows per cassette (2-40)
  -T <TEMPLATE>        start with one cassette per topic from the named [templates] entry
      --theme <NAME>   color theme for this session (overrides config)
  -R, --record         record mode: no deletions, the tape only rolls forward
  -o, --output         print to stdout on quit instead of writing a file
      --writer <NAME>  registered writer to act as (default: $CASSETTE_WRITER, else $USER — only $USER may register on first use)
      --json           emit machine-readable JSON (full data on queue list/next/show; {"error","code"} on any command that fails)
  -h, --help           Print help
  -V, --version        Print version
```

`queue`, `writer`, and `session` are the session-store commands — see
"The session store" above for the full command surface.

The help text above still says "note" throughout (`new`, `today`, `resume`,
`find`) — that wording predates the session store and hasn't caught up
(clap generates it straight from `src/cli.rs`'s doc comments), but the
behavior underneath it is store-backed: `new NAME` creates a session
aliased `NAME`, `today` opens or creates the session aliased with today's
date, `resume [NAME]` loads a session back into the TUI by alias or by id
(default: the most recently created session), and `find [TEXT]` lists recent sessions
newest-first (date, words, topics, first line of the top cassette),
filtered by `TEXT` when given. A bare `cassette` (no subcommand) starts a
fresh, unaliased session. `stats` reads streak/weekly/monthly totals from
the session store, not from any note file — see "A daily practice" above
for what that means for notes written before the store existed.

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
