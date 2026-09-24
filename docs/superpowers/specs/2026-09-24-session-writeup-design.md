# Session write-up: export's second mode, as a skill

> Written unattended overnight on 2026-09-24 from `docs/follow-up.md`'s "Document export — two
> modes, one of them unbuilt". Settles the entry's three open questions. Judgement calls are
> under **Decisions**.

## Purpose

`cassette export <id>` answers "what was written": every cassette verbatim, in queue order,
closed and damaged ones marked. The unbuilt second mode answers "what did we arrive at". The
agent reads the session and writes the document it was converging on, as one argument rather
than N numbered notes.

## The three open questions, settled

### Is it a CLI command? **No. It is a skill.**

It needs a model and `cassette` is model-free. Keeping it that way is worth more than a
subcommand's discoverability. The artifact is `.claude/skills/cassette-writeup/SKILL.md`,
alongside `cassette-session`. It drives the existing CLI and adds nothing to the binary. The
follow-up leaned this way; nothing found contradicts it.

### Where does the output go? **A file outside the store. Never a cassette.**

- A write-up is a deliverable, so it lives where deliverables live. Written back into the
  session, the next distillation pass would start rewriting the summary into the thing it
  summarises. The follow-up flagged this risk, and the `cassette-session` skill's own rule is
  that the end-of-session summary goes in the chat, "not as one more cassette they would have to
  go find".
- **Path:** the skill proposes one and confirms it in chat before writing. The default is
  `<alias or YYYY-MM-DD>-writeup.md` in the current working directory, or under `docs/` if the
  working directory is a repo that has one. It never overwrites without asking.
- **Provenance footer** at the end of the file: `Distilled from cassette session <id> (<n>
  cassettes, <date>). Raw record: cassette export <id>.` The raw archive stays one command away,
  and the document says it is an interpretation.

### What does it do with disagreement? **It keeps it, in its own section.**

A session that answered one question three ways has no single line. Flattening that would be
lossy in a way the raw export is not. The rules, in the order the skill applies them:

1. **Within a cassette, the latest text wins.** Under the distillation model a cassette's body
   *is* the current shared understanding. Earlier states are gone by design and are not
   reconstructed.
2. **Across cassettes, a later cassette that explicitly supersedes an earlier one wins.** It
   gets a one-line "earlier we thought X" only if the reversal matters to a reader. "Later"
   means `updated_at`, not queue position, which may be recency-of-reply
   (`cassette-session`'s "surface a reply" rule).
3. **Anything still unresolved goes under `## Open questions`,** with each position stated
   fairly and without attribution. The distillation model has no speakers to attribute to.
4. **Closed cassettes:** a close-out blockquote (`> …`, from `queue close -m`) is read as the
   verdict. A cassette closed as abandoned goes under `## Set aside` in one line each, or is
   omitted when trivial. An argument that silently drops what was tried and rejected invites
   re-litigating it.
5. **Side B is scratch.** It is read for context and never quoted or promoted into the
   argument.

## Shape of the skill

**Input.** `cassette queue list --session <ID> --status all --json`, not `export`'s markdown.
The JSON separates `side_a`/`side_b`, and carries `status`, `updated_at` and `topic`. Those are
the fields rules 1–5 need. A nonzero `unreadable` is reported to the human before writing, since
the write-up would silently lack those cassettes.

**Process.**

1. Read everything. Group cassettes by what they are *about*, not by topic string. Topics drift
   and some are `(untitled)`.
2. Propose an outline **in chat**: the thesis in a sentence, the section headings, and what goes
   under Open questions and Set aside. Wait for a yes. This is the judgement call the follow-up
   says belongs to the agent. Showing it before writing costs one exchange and catches a wrong
   thesis before it is 1 500 words long.
3. Write the file. Prose that argues. No cassette numbers, no "the human said / the agent said",
   no bullet dump of cassette contents.
4. Say in chat where it went. Give the one-paragraph thesis and anything the session left
   unresolved.

**Fidelity rules** (the skill's hard constraints):

- Nothing in the write-up that is not in a cassette. Connective tissue and ordering are the
  agent's contribution. New claims are not. If the argument needs a step nobody wrote, it goes
  under Open questions as a gap.
- The session's own words where a phrase was clearly deliberate. Paraphrase otherwise.
- Length follows the material. The `cassette-session` ~15-lines-per-cassette guidance is about
  reading mid-flow and does not apply here.

**Trigger.** The description names the moments: "write up this session", "turn the session
into a doc", end of a working session when the human asks what came of it. It explicitly does
*not* trigger mid-session. While the human is writing, `cassette-session` governs.

## Decisions

1. **Skill, not subcommand.** See above.
2. **File, not cassette.** See above. The path is confirmed in chat, never guessed silently.
3. **Outline approval before writing.** It is the only interactive step. Without it the skill's
   biggest failure mode (a confident wrong thesis) is found only after the whole document exists.
4. **JSON over export markdown** as input. Structure is needed for the side-B and closed rules.
5. **Disagreement preserved** under `## Open questions`, unattributed.
6. **Built TDD-style per `superpowers:writing-skills`.** A baseline subagent run without the
   skill on a fixture session records what goes wrong. Then the skill is written against those
   failures and the run is repeated.

## Out of scope

- Writing the result back into the session in any form.
- Multi-session write-ups ("everything this week"). They would need a session-selection step and
  a different grouping model. Revisit if asked.
- Any change to `cassette export`.

## Acceptance criteria

- `.claude/skills/cassette-writeup/SKILL.md` exists with a trigger-shaped `description`.
- A fixture session script (committed under `.claude/skills/cassette-writeup/fixture.sh`) builds a
  session containing: a question answered two conflicting ways, a reversal across cassettes, a
  closed-with-verdict cassette, a closed-abandoned one, side-B scratch with a tempting quote, and
  an untitled cassette.
- The baseline run (no skill) and the final run (with skill) are both recorded in the plan's
  commit messages. The final run's output has an Open questions section with both positions, no
  side-B quote, no speaker attribution, the provenance footer, and was preceded by an outline in
  chat.
- `cassette-session`'s "Two surfaces" paragraph points to `cassette-writeup` for "turn this into
  a document".
- The follow-up entry is removed from `docs/follow-up.md`. CLAUDE.md's Planning section mentions
  that skills live in `.claude/skills/`, if it doesn't already.
