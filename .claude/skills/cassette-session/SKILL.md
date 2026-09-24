---
name: cassette-session
description: Work alongside a human in a live cassette session — the commands, and the conventions for how a session is supposed to go.
---

# Working in a cassette session

The human writes in the TUI. You write through `cassette queue` on the CLI, into the same
session, at the same time. Neither waits for the other.

## Setup, and the one trap

```bash
cassette writer register --name bot --kind agent   # ONCE, per store
cassette queue list --session <ID> --json
```

**Pass `--writer bot` on every command.** Without it the CLI falls back to `$USER`, which
**auto-registers as `human` kind** — the privileged one. You will look like an agent and not be
one: sticky locks will not bind you and `write_permitted` will wave you through. This has
already produced one false bug report. Check with `cassette writer list` if unsure.

## What to act on

`waiting_on` is the inverse of whoever wrote last, so a cassette the human touched is
**waiting on you**. Act on cassettes where `waiting_on == "agent"` **and** `busy == false`.

```bash
cassette queue next --session <ID> --json    # highest-priority open, unlocked cassette
```

Exit codes are the idle signal: **3** everything is busy (wait), **5** nothing open and
unlocked (do nothing), **4** a sticky `queue lock` you must not write over, **6** queue full.

- **Reading never blocks.** `queue list`/`show` take no locks — you can always read every
  cassette, including the one the human is typing in right now.
- **Writing is exclusive per cassette.** `busy == true` means the human is focused there.
  Never write it; you will get exit 3. Their cursor moving is the handoff.

## How a session goes

**A cassette is a distillation, not a transcript.** Rewrite the whole body into the current
shared understanding each pass. `queue write` replaces by default and that is correct;
`--append` is the exception, not the habit.

- **No writer prefixes.** A rewritten cassette has no history to attribute. Attribution is
  metadata (`last_writer`), not prose.
- **A new thought gets a new cassette.** Replying inside the human's cassette queues you both
  behind one lock for no reason.
- **Surface a reply** with `queue move <id> --session <ID> --before <current-first>`. Be aware
  this orders the session by recency rather than topic — right for a conversation, wrong for
  morning pages.
- **Cadence in minutes, not seconds.** Freewriting arrives in paragraphs; a fast tick catches
  half a thought, and every tick costs a model call whether or not there is work.
- **Say nothing when there is nothing.** A stream of "nothing to do" in the terminal defeats
  the point of not having to watch it.

## Length

**Aim for ~15 lines per cassette.** The human is mid-flow and has to read this on a screen
beside their own writing — a wall of text is something to bounce off, not read. Prefer the
one idea they asked about over the four adjacent ones you could add.

Adjust this number if it turns out to be wrong; it is a starting point, not a finding.

## Judgement

Respond to what they actually wrote, not to what would be easy to answer. If they ask a
design question, answer it — including when the answer reverses something you said earlier in
the session. Put findings in the cassette, not in the chat: the chat is where they are not
looking.

You are reading everything they write, continuously, with no natural pause where they choose
what to share. For a scoped working session that is the point. If it starts to look like
private journalling, say so rather than keeping reading.
