# Plan: README demo GIF (chainlink #49)

**Deliverable:** a short, reproducible terminal GIF at the top of the README showing
the app's core loop.
**Agent-executable:** fully (vhs runs headless; the repo also has a project `verify`
skill that drives the TUI and captures screens — useful for checking frames, but vhs
produces the actual GIF).

## Context

- vhs (charmbracelet) scripts terminal recordings as `.tape` files — reproducible,
  re-recordable after UI changes. Install: `pacman -S vhs` (Arch) or
  `go install github.com/charmbracelet/vhs@latest`; needs `ttyd` and `ffmpeg`.
- Scenes the issue asks for: start with `-t`/`-w`, type, flip to side B (Ctrl+B),
  set a topic (Ctrl+T), reel bar winding, themed look.
- Sessions write notes; the recording must not pollute the real notes dir — run with
  `-o` (stdout, no file) **and** point `XDG_DATA_HOME`/config at a temp dir inside
  the tape via `Env`.

## Steps

1. Create `demo/demo.tape`. Suggested script (~30–40s total, trim in review):
   ```tape
   Output demo/demo.gif
   Set FontSize 18
   Set Width 900
   Set Height 560
   Set TypingSpeed 60ms
   Env XDG_CONFIG_HOME /tmp/cassette-demo/config
   Env XDG_DATA_HOME /tmp/cassette-demo/data
   Type "cassette --theme gruvbox -t 10 -w 50 -o" Enter
   Sleep 1s
   Type "The tape is rolling. Ideas go straight onto side A without stopping to edit."
   Sleep 800ms
   Ctrl+T
   Type "morning pages" Enter
   Sleep 1s
   Ctrl+B
   Type "Side B is the scratch pad — half-thoughts land here."
   Sleep 1.5s
   Ctrl+B
   Sleep 1s
   Ctrl+C
   ```
   Adjust: enough words to visibly wind the reel toward the 50-word goal (repeat a
   couple of sentences if needed). Verify each binding against the README before
   recording.
2. `cargo build --release` first and put `target/release` on PATH inside the tape
   (`Env PATH ...`) or install the binary, so vhs runs the current build.
3. Run `vhs demo/demo.tape`; inspect `demo/demo.gif` (open frames or eyeball size).
   Keep the GIF **under ~3 MB** — reduce dimensions/length before reaching for gifsicle.
4. Embed at the top of README.md, directly under the title:
   `![cassette demo](demo/demo.gif)`.
5. Exclude `demo/` from the crates.io package (`exclude` in Cargo.toml, see #6).
6. Commit tape + GIF together (the tape is the source; the GIF is a build artifact
   we still commit so GitHub renders it).

## Acceptance criteria

- `vhs demo/demo.tape` reproduces the GIF from a clean checkout.
- GIF shows: timer+goal start, typing, topic label appearing, side flip (A→B→A),
  reel bar movement, a non-default theme.
- README renders it at the top; file size reasonable (<~3 MB).
