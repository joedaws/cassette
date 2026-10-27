---
name: cassette-session
description: Work alongside a human in a live cassette session — the commands, and the conventions for how a session is supposed to go.
---

# Working in a cassette session

The human writes in the TUI. You write through `cassette queue` on the CLI, into the same
session, at the same time. Neither waits for the other.

## Setup

```bash
cassette writer register --name bot --kind agent   # ONCE, per store
cassette queue list --session <ID> --json
```

**Ids say what they are:** `ses_…` is a session, `cas_…` a cassette. `--session` takes the
first, `<ID>` the second, and passing the wrong one is exit 2 naming both.

**Pass `--writer bot` on every command.** Without it the CLI falls back to `$USER` — the human's
name — and your writes are *attributed to the human*, which misroutes `waiting_on`. Sticky
locks still bind you (an identity taken from `$USER` never carries human authority), so
forgetting is no longer a way to trample a claim, but it is still wrong. `cassette writer
whoami` shows what you are acting as.

## What to act on

`waiting_on` is the inverse of whoever wrote last, so a cassette the human touched is
**waiting on you**. Act on cassettes where `waiting_on == "agent"`, `busy == false` **and**
`claimed_by` is null. Only writes change `waiting_on`: a `queue move`, `lock`, `close` or
`topic` no longer flips it. An empty cassette is the human's prompt — don't write into it, even
when `waiting_on == "agent"`.

```bash
cassette queue next --session <ID> --json    # highest-priority open, unlocked cassette
```

`queue next` skips **every** live claim, including your own. To resume a cassette you have
already claimed, use its id directly rather than asking `next` for it.

Exit codes are the idle signal: **3** everything is busy (wait), **5** nothing open and
unlocked (do nothing), **4** a sticky `queue lock` you must not write over, **6** queue full.

- **Reading never blocks.** `queue list`/`show` take no locks — you can always read every
  cassette, including the one the human is typing in right now.
- **Writing is exclusive per cassette.** `busy == true` means the human is focused there.
  Never write it; you will get exit 3. Their cursor moving is the handoff.

## Claim before you think

Reading and thinking take seconds to minutes, and the human sees the cassette as ready for
you the whole time. Mark it picked up first:

```bash
cassette queue claim <id> --session <ID> --writer bot [--for 300]   # default 120 s
cassette queue write <id> --session <ID> --writer bot < reply.md   # writing releases the claim
cassette queue release <id> --session <ID> --writer bot            # decided not to answer
```

- Pick `--for` longer than your think time. Expiry is a crash safety net, not a timer to beat;
  an expired claim simply stops counting.
- Never work a cassette whose `claimed_by` is someone else. Writing over another agent's live
  claim is exit 4; a claim on the human's focused cassette is exit 3.
- `claim` needs an explicit `--writer`; without it the command exits 2.

## What the human sees

Each open cassette carries one glyph in its separator line:

| Glyph | Status | Meaning |
|---|---|---|
| `✎` (pulses) | human | the human's move |
| `⚙` | agent | waiting on an agent |
| `●` | busy | the human has it focused, or an agent holds a live claim |

Closed cassettes show none. A blank cassette is `✎` unless busy. The glyphs are configurable, so
describe them by status name if the human may have changed them.

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
- **Retitle** a cassette whose subject has drifted — including an untitled one the human
  created — with `cassette queue topic <id> --session <ID> "<topic>"`. It does not change whose
  turn it is.
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
the session.

**Two surfaces, two jobs.** While they are writing, findings go in the cassette — the chat is
where they are not looking, and pulling them out of the TUI costs them the flow the tool
exists to protect. But nobody focuses for eight hours. Stepping back to refine, decide, and
look at the whole shape is what the Claude Code session is for, and it is the better surface
for it: scrollback, the codebase, and a conversation that is allowed to be long.

So do not try to move everything into cassettes. A cassette is where a thought gets refined;
the chat is where you decide what to do about it. When a session ends, the summary belongs in
the chat, not as one more cassette they would have to go find. If they want the session turned into a
document, that is the `cassette-writeup` skill.

You are reading everything they write, continuously, with no natural pause where they choose
what to share. For a scoped working session that is the point. If it starts to look like
private journalling, say so rather than keeping reading.
