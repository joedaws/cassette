---
name: verify
description: Drive the cassette TUI end-to-end and capture screens for verification.
---

# Verifying cassette

`cassette` is a raw-mode ratatui TUI; it needs a pty. `screen`, `expect` and
`pyte` are not installed; `zellij attach --create-background` creates a session
whose panes never render (dump-screen returns empty) — don't bother. **`tmux`
and `zellij` ARE installed** (this file claimed otherwise until 2026-09-23);
tmux is useful for keeping a long-running session alive, but it is not the tool
for scripted assertions — you still want the driver below, which gives you the
bytes directly.

What works: a Python `pty.fork()` driver. Pattern:

```python
import os, pty, re, select, struct, fcntl, termios, time
pid, fd = pty.fork()
if pid == 0:
    os.environ["XDG_CONFIG_HOME"] = scratch_xdg   # isolate config.toml
    os.environ["CASSETTE_DATA_DIR"] = scratch_store  # isolate the session store
    os.environ["TERM"] = "xterm-256color"
    # The note name is a `new` argument -- there is no top-level positional.
    os.execv(binary, [binary, "-t", "1", "new", note_name])
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 100, 0, 0))
# write keys with os.write(fd, ...), drain with select+os.read between sends
```

- **Isolation is two variables, and both are required.** `XDG_CONFIG_HOME`
  (write `$SCRATCH/xdg/cassette/config.toml` under it) and **`CASSETTE_DATA_DIR`**,
  which points at the session store. Without the second, every command here —
  `cassette`, `cassette new`, `today`, `resume`, `queue …`, and the `cargo test`
  that spawns the binary — reads and writes the user's real store at
  `~/.local/share/cassette/`. That has already happened once: a verification run
  left a real session behind in it. Set `CASSETTE_DATA_DIR=$SCRATCH/store` on
  **every** invocation, pty or not, and never run a cleanup command against the
  real store to undo a mistake — the words in it are the user's.
- **Answer cursor-position queries or the driver lies to you.** Since ratatui
  0.30, `Terminal::clear()` reads the cursor position, so a driver that never
  replies to `ESC[6n` makes the app die with "The cursor position could not be
  read within a normal duration" (exit 1) on any path that clears -- Ctrl+Z
  resume, for one. Real terminals always reply; a bare `pty.fork()` does not.
  Reply from inside the drain loop:

  ```python
  DSR = re.compile(rb'\x1b\[6n')
  for _ in DSR.findall(chunk):
      os.write(fd, b"\x1b[1;1R")   # row 1, col 1
  ```

  Without this you will chase a regression that is not there.

- Strip ANSI for assertions: `re.compile(rb'\x1b\[[0-9;?]*[a-zA-Z]|\x1b[()][0-9A-B]|\x1b[>=]|\x1b\][^\x07]*\x07')`.
  The first drain (~1.2s) is a full screen; later drains are ratatui diffs —
  assert on substrings, not layout.
- **Force a full repaint before asserting anything that matters.** Ratatui
  diff-renders: unchanged cells never reach the pty, so a capture of the diff
  stream can show text that has since been overwritten, and letters go missing
  mid-word (`caste` for `cassette`). Resize the pty to trigger a full redraw,
  then drain:

  ```python
  fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows + 1, cols, 0, 0))
  time.sleep(1.0)
  screen = drain()
  ```

  Phase 5c nearly lost a real bug to this: the diff capture showed the CORRECT
  banner, which a tick had already replaced. The repaint showed the wrong one.
  It cuts both ways — a diff-stream assertion also *fails* on text that is
  genuinely on screen, so check the repaint before believing either result.
- Ratatui positions text runs with cursor-move escapes that stand in for the
  spaces between them, so stripped output has words glued together
  (`helloworld`, `0/5`). Strip spaces from both sides before comparing:
  `b"helloworld" in screen.replace(b" ", b"")`.
- `cargo test` does **not** rebuild `target/debug/cassette` — run `cargo build`
  before driving the binary or you'll verify stale code.
- Key bytes: Esc `\x1b`, Enter `\r`, Backspace `\x7f`, Tab `\t`, Ctrl+X = chr(x & 0x1f)
  (Ctrl+B `\x02`, Ctrl+N `\x0e`, Ctrl+C `\x03`).
- Point `new <name>` at a scratch store and read the session back after quit
  (`Esc` then `q`) with `cassette queue list --session <id>` or by reading the
  cassette files under `$CASSETTE_DATA_DIR/sessions/<id>/cassettes/` — what
  landed on disk is the best end-to-end assertion. Bare `resume` now opens the
  **newest session in the store** (not a notes file), and quitting rewrites it:
  harmless against an isolated `CASSETTE_DATA_DIR`, destructive against the real
  one, which is the second reason that variable is not optional.
- CLI error paths (`-h`, bad flags, unknown `-T` template) exit before raw mode,
  so they can be run directly without a pty.

## Two ways a passing drive lies to you

- **A fixture nothing can see.** A cassette with no topic and no text renders
  as a blank row, so `assert not in screen` passes whether or not it was drawn.
  Phase 5c's first fold test passed with the feature deleted for exactly this
  reason. Give fixtures distinguishable topics or text, and assert that
  something you EXPECT to be drawn is present in the same test — that is what
  makes an absence mean anything.
- **A fixture too small to reach the failure.** Phase 6's `export` panicked on
  a closed pipe, on its documented primary use, and both the test suite and a
  manual drive missed it — the session was 650 bytes, under the 64KB pipe
  buffer. It needed 648KB to show at all. When the thing under test has a
  buffer, a cap or a screenful in it, size the fixture past that boundary.

Flows worth driving: type on side A → Ctrl+B → type on side B → check both land
under their `## Side A`/`## Side B` headings; `t` topic prompt in normal mode;
`-T <template>` startup; quit-with-nothing-typed (must print "nothing recorded"
and leave no session behind in the scratch store).
