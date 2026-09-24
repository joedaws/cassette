# Session Write-up Skill Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Tasks 2–4 also REQUIRE superpowers:writing-skills (RED → GREEN → REFACTOR for documentation).

**Goal:** A `cassette-writeup` skill that turns a cassette session into one coherent document in a file outside the store, keeping disagreement visible and inventing nothing.

**Architecture:** No Rust changes. One skill file plus a fixture script that builds a realistic session through the real CLI. The skill is written TDD-style: first a baseline subagent run without the skill, then the skill written against the observed failures, then a re-run.

**Tech Stack:** `cassette` CLI (`session new`, `writer register`, `queue new|write|close|list --json`), Claude Code skills, subagents for the pressure runs.

**Spec:** `docs/superpowers/specs/2026-09-24-session-writeup-design.md`

## Global Constraints

- The binary stays model-free: no new subcommand, no Rust change.
- Output goes to a file outside the store and **never** into a cassette.
- The path is confirmed in chat before writing. Never overwrite without asking.
- An outline goes to chat and is approved before the document is written.
- Nothing appears in the write-up that no cassette contains. Gaps go under `## Open questions`.
- No speaker attribution. Side B is never quoted.
- A provenance footer, verbatim shape: `Distilled from cassette session <id> (<n> cassettes, <YYYY-MM-DD>). Raw record: cassette export <id>.`

## Review Focus

- **A session with `unreadable > 0`.** The skill must say so before writing. Task 3 adds this line to the skill; Task 4's run checks it by corrupting one fixture file.
- **Every cassette untitled.** Grouping must work from content, not topics. The fixture has one untitled cassette; the skill's text says "group by what they are about".
- **A one-cassette session.** The skill should say a write-up adds little and offer `cassette export` instead of producing a padded document. It's a line in the skill; Task 4 spot-checks it.
- **Target file already exists.** The skill asks before overwriting. Task 4 pre-creates the default path.
- **Invocation while the human is still writing** (a cassette `busy`). The skill notes the session is live and asks whether to proceed. It's a line in the skill.

---

### Task 1: Fixture session script

**Files:**
- Create: `.claude/skills/cassette-writeup/fixture.sh`

- [ ] **Step 1: Write the script**

```bash
#!/usr/bin/env bash
# Builds a small session exercising every write-up rule, in a throwaway store.
# Usage: eval "$(.claude/skills/cassette-writeup/fixture.sh)"  → exports CASSETTE_DATA_DIR and SID
set -euo pipefail
BIN=${CASSETTE_BIN:-cassette}
export CASSETTE_DATA_DIR=${CASSETTE_DATA_DIR:-$(mktemp -d)/store}
$BIN writer register --name me --kind human >/dev/null
$BIN writer register --name bot --kind agent >/dev/null
SID=$($BIN session new --alias storage-choice)

w() { # w <writer> <id> [side] ; body on stdin
  $BIN --writer "$1" queue write "$2" --session "$SID" ${3:+--side "$3"} >/dev/null
}
new() { $BIN --writer "$1" queue new "$2" --session "$SID"; }

A=$(new me "where do notes live")
printf 'Notes should be one markdown file per session. Simple, greppable, ours.\n' | w me "$A"
printf 'honestly maybe sqlite?? "a database is just a file with opinions"\n' | w me "$A" b

B=$(new bot "concurrency")
printf 'One file per session cannot be written by two writers at once without a lock on the whole session. Per-cassette files let a human and an agent write different cassettes concurrently.\n' | w bot "$B"

C=$(new me "reversal")
printf 'Changed my mind: per-cassette files, one directory per session. One-file-per-session is out.\n' | w me "$C"

D=$(new bot "locking")
printf 'Use flock per cassette. Question still open: does flock hold on network filesystems? Unknown; we have not tested.\n' | w bot "$D"
# The human rewrites D into the current (unresolved) shared state.
printf 'Use flock per cassette. Network filesystems: bot thinks unsafe, I think fine for a single user. Unresolved.\n' | w me "$D"

E=$(new me "sqlite")
printf 'Tried sketching a sqlite schema.\n' | w me "$E"
$BIN --writer me queue close "$E" --session "$SID" -m "abandoned: plain text is the point"

F=$(new bot "atomic writes")
printf 'Write to a temp file and rename, so readers never see a torn cassette.\n' | w bot "$F"
$BIN --writer me queue close "$F" --session "$SID" -m "agreed, done"

G=$($BIN --writer me queue new "" --session "$SID")
printf 'Also: sessions need ids, not names, because two sessions can share a name.\n' | w me "$G"

echo "export CASSETTE_DATA_DIR='$CASSETTE_DATA_DIR' SID='$SID'"
```
`chmod +x`. The untitled cassette is `queue new ""`. If `queue new` rejects an empty topic, create it titled and, if the `queue topic` plan has landed, clear it with `queue topic "$G" --session "$SID" ""`. Otherwise hand-edit the frontmatter line to `topic:`.

- [ ] **Step 2: Run it against the dev build**

```bash
cargo build && CASSETTE_BIN=$PWD/target/debug/cassette eval "$(.claude/skills/cassette-writeup/fixture.sh)" \
  && target/debug/cassette queue list --session "$SID" --status all --json | head -c 600
```
Expected: JSON with 7 cassettes, two `"status":"closed"`, and one with a non-empty `side_b`.

- [ ] **Step 3: Commit**

```bash
git add .claude/skills/cassette-writeup/fixture.sh
git commit -m "test: fixture session for the cassette-writeup skill"
```

---

### Task 2: RED, a baseline run without the skill

- [ ] **Step 1:** Build the fixture (Task 1 Step 2). Dispatch a fresh general-purpose subagent with **no mention of the skill**:

> "There is a cassette session `$SID` (store at `$CASSETTE_DATA_DIR`; binary `target/debug/cassette`). Read it with the `cassette` CLI and write it up as a coherent document for someone who wasn't there. Put the result wherever you think is right."

- [ ] **Step 2:** Score the output against this checklist and record each hit or miss verbatim:
  1. Asked or proposed an outline before writing?
  2. Wrote to a file (not a cassette), and confirmed the path?
  3. Kept the network-filesystem disagreement as an open question with both positions?
  4. Stated the reversal (one-file → per-cassette) as the conclusion rather than presenting both as live?
  5. Quoted side B ("a database is just a file with opinions")?
  6. Attributed speakers ("the bot said", "I said")?
  7. Invented claims not in any cassette?
  8. Handled the closed cassettes: sqlite as set aside, atomic writes as agreed?
  9. Included a provenance footer?
  10. Numbered-cassette dump instead of an argument?

- [ ] **Step 3: Commit** the notes as `.claude/skills/cassette-writeup/BASELINE.md` (short: the checklist with results and the two or three most telling quotes). Delete it in Task 4 once GREEN is recorded, if you prefer; the commit message keeps it either way.

```bash
git add .claude/skills/cassette-writeup/BASELINE.md
git commit -m "test: baseline write-up run without the skill"
```

---

### Task 3: GREEN, write the skill against the observed failures

**Files:**
- Create: `.claude/skills/cassette-writeup/SKILL.md`

- [ ] **Step 1: Write it.** Frontmatter:

```yaml
---
name: cassette-writeup
description: Turn a finished cassette session into one coherent document — the argument the session converged on, written to a file outside the store. Use when asked to write up, summarise into a doc, or turn a session into a document; not while the human is still writing (that is cassette-session).
---
```

Body sections, in this order. Keep each to the rules the spec states; add emphasis only where the baseline failed:

1. **What this is:** `cassette export` is the verbatim archive; this is the interpretation. One sentence each.
2. **Read:** `cassette queue list --session <ID> --status all --json`. Report `unreadable > 0` before anything else. If any cassette is `busy`, say the session is live and ask whether to proceed. For one cassette, suggest `cassette export` instead.
3. **Resolve**, the five rules verbatim from the spec (latest text wins within a cassette; explicit supersession across cassettes by `updated_at`; unresolved → `## Open questions`, unattributed, each position fairly; closed → close-out blockquote is the verdict, abandoned → `## Set aside`; side B is context only, never quoted).
4. **Outline first:** thesis sentence plus headings plus what goes in Open questions / Set aside, in chat. Wait for a yes.
5. **Write:** a file, never a cassette. Propose the path (`<alias or date>-writeup.md` in cwd, or `docs/` in a repo that has one); confirm; never overwrite without asking. Prose that argues. No cassette numbers, no speakers. Nothing no cassette says, and a missing step becomes an open question. End with the provenance footer (exact shape from Global Constraints).
6. **Report:** the path and a one-paragraph thesis in chat, plus what stayed open.

- [ ] **Step 2: Re-run** Task 2's exact prompt with a fresh subagent, this time prefixed "Use the cassette-writeup skill at `.claude/skills/cassette-writeup/SKILL.md`." When the subagent proposes an outline or a path, answer "yes" (via SendMessage) so it proceeds.

- [ ] **Step 3: Score** against the same ten-item checklist. Every item must now pass. For each that fails, tighten the skill's wording where the subagent went wrong (quote the rationalization it used, per writing-skills) and re-run.

- [ ] **Step 4: Commit**

```bash
git add .claude/skills/cassette-writeup/SKILL.md
git commit -m "feat: cassette-writeup skill turns a session into a document

Baseline: <n>/10 checklist items passed. With skill: 10/10."
```

---

### Task 4: REFACTOR, edge cases, cross-links and docs

**Files:**
- Modify: `.claude/skills/cassette-writeup/SKILL.md` (only if a run below fails)
- Modify: `.claude/skills/cassette-session/SKILL.md`
- Modify: `docs/follow-up.md`, `CLAUDE.md` (if needed)

- [ ] **Step 1: Edge runs**, each with a fresh subagent and the skill:
  - Pre-create `storage-choice-writeup.md` in the cwd. Expect it to ask before overwriting.
  - Corrupt one fixture cassette's frontmatter (`sed -i '1s/---/--x/' <file>`). Expect `unreadable` to be reported first.
  - A fresh session with one cassette. Expect it to suggest `cassette export`.
  Tighten the skill for any miss and re-run that case.

- [ ] **Step 2: Cross-link.** In `cassette-session`'s "Two surfaces, two jobs" paragraph, after "the summary belongs in the chat, not as one more cassette", add: "If they want the session turned into a document, that is the `cassette-writeup` skill."

- [ ] **Step 3:** Remove the "Document export — two modes, one of them unbuilt" section from `docs/follow-up.md`. In CLAUDE.md's `src/export.rs` paragraph, add one sentence: "Its interpretive counterpart, which writes the argument a session converged on, is the `cassette-writeup` skill (`.claude/skills/`), not a subcommand, because it needs a model."

- [ ] **Step 4:** Delete `BASELINE.md` if Task 2 kept it (history has it).

- [ ] **Step 5: Commit**

```bash
git add .claude/skills docs/follow-up.md CLAUDE.md
git commit -m "docs: point cassette-session at cassette-writeup and retire the export follow-up"
```
