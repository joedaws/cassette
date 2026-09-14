---
name: verify
description: Drive the cassette TUI end-to-end and capture screens for verification.
---

# Verifying cassette

`cassette` is a raw-mode ratatui TUI; it needs a pty. On this machine there is
**no tmux/screen/expect/pyte**, and `zellij attach --create-background` creates a
session whose panes never render (dump-screen returns empty) — don't bother.

What works: a Python `pty.fork()` driver. Pattern:

```python
import os, pty, re, select, struct, fcntl, termios, time
pid, fd = pty.fork()
if pid == 0:
    os.environ["XDG_CONFIG_HOME"] = scratch_xdg   # isolate config.toml
    os.environ["TERM"] = "xterm-256color"
    # Note path is a `new` argument -- there is no top-level positional.
    os.execv(binary, [binary, "-t", "1", "new", note_path])
fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 100, 0, 0))
# write keys with os.write(fd, ...), drain with select+os.read between sends
```

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
- Ratatui positions text runs with cursor-move escapes that stand in for the
  spaces between them, so stripped output has words glued together
  (`helloworld`, `0/5`). Strip spaces from both sides before comparing:
  `b"helloworld" in screen.replace(b" ", b"")`.
- `cargo test` does **not** rebuild `target/debug/cassette` — run `cargo build`
  before driving the binary or you'll verify stale code.
- Key bytes: Esc `\x1b`, Enter `\r`, Backspace `\x7f`, Tab `\t`, Ctrl+X = chr(x & 0x1f)
  (Ctrl+B `\x02`, Ctrl+N `\x0e`, Ctrl+C `\x03`).
- Point `new <path>` at a scratch path and read the markdown after quit (`Esc` then `q`) —
  the saved file is the best end-to-end assertion. Never drive `resume` with no
  argument: it opens the newest note in the real notes dir and quitting rewrites it.
- Config isolation: write `$SCRATCH/xdg/cassette/config.toml` and set `XDG_CONFIG_HOME`.
- CLI error paths (`-h`, bad flags, unknown `-T` template) exit before raw mode,
  so they can be run directly without a pty.

Flows worth driving: type on side A → Ctrl+B → type on side B → check both land
under their `## Side A`/`## Side B` headings; `t` topic prompt in normal mode;
`-T <template>` startup; quit-with-nothing-typed (must print "nothing recorded"
and write no file).
