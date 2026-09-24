---
name: cassette-writeup
description: Turn a finished cassette session into one coherent document — the argument the session converged on, written to a file outside the store. Use when asked to write up, summarise into a doc, or turn a session into a document; not while the human is still writing (that is cassette-session).
---

# Writing up a cassette session

`cassette export <ID>` is the archive: every cassette verbatim, in queue order. This skill is
the interpretation: the document the session was converging on, written as one argument
instead of N numbered notes. The raw export stays one command away, so the write-up is free to
be shorter than the session. It is not free to be different from it.

## 1. Read

```bash
cassette queue list --session <ID> --status all --json
```

Use the JSON, not the export. It separates `side_a` from `side_b` and carries `status`,
`updated_at` and `topic`, which the rules below need.

Before anything else, check three things and say them in chat if they apply:

- **`unreadable > 0`**: those cassettes will be missing from the write-up. Tell the human
  before writing. `cassette export <ID>` names the damaged files.
- **Any cassette `busy: true`**: someone is still writing. Say the session is live and ask
  whether to go ahead or wait.
- **One cassette, or nearly nothing written**: a write-up adds little. Offer `cassette export`
  instead of padding a document.

## 2. Resolve

Apply these in order:

1. **Within a cassette, the latest text wins.** A cassette's body *is* the current shared
   understanding. Earlier states are gone on purpose, so do not reconstruct them.
2. **Across cassettes, explicit supersession wins, by `updated_at`.** "Later" means
   `updated_at`, not queue position, because the queue may be ordered by recency of reply. A
   cassette that reverses an earlier one is the conclusion. Mention the earlier position in one
   line only if a reader needs to know it was considered.
   `updated_at` has one-second precision, so cassettes written in quick succession can tie.
   When they tie, go by what the text says ("changed my mind", "instead of X"), never by
   queue position. If neither settles it, it is an open question.
3. **Anything still unresolved goes under `## Open questions`.** State each position fairly and
   **without attribution**. The document has no speakers.
4. **Closed cassettes:** the close-out blockquote (`> …`) is the verdict. Agreed or done means
   it belongs in the argument. Abandoned means one line under `## Set aside`, or omit it if it
   was trivial. Leaving out what was tried and rejected invites re-litigating it.
5. **Side B is scratch.** Read it for context. **Never quote it and never promote it into the
   argument**, however quotable it is.

Group cassettes by what they are *about*, not by topic string. Topics drift, and some are
`(untitled)`.

## 3. Outline first, in chat

Before writing the document, post in chat:

- the thesis in one sentence,
- the section headings,
- what goes under `## Open questions` and `## Set aside`,
- the proposed path.

Wait for a yes. A confident wrong thesis is this skill's worst failure, and one exchange here is
cheaper than rewriting 1 500 words.

## 4. Write

- **A file, never a cassette.** Writing the summary back into the session would make the next
  distillation pass rewrite it into the thing it summarises.
- **Path:** propose `<alias or YYYY-MM-DD>-writeup.md`, in `docs/` if the working directory is a
  repo that has one, else in the working directory. Confirm it in the outline step. **If the
  file exists, ask before overwriting.**
- **Prose that argues.** No cassette numbers, no "cassette 3 says", no bullet dump of cassette
  contents, no "the human said" / "the agent said".
- **Nothing no cassette says.** Your contribution is ordering and connective tissue, not new
  claims. If the argument needs a step nobody wrote, that step goes under Open questions as a
  gap. Don't fill it in.
- Keep the session's own words where a phrase was clearly deliberate. Otherwise paraphrase.
- Length follows the material. The ~15-lines-per-cassette guidance in `cassette-session` is
  about reading mid-flow and does not apply here.
- End with this footer, exactly:

  ```
  ---
  Distilled from cassette session <ID> (<N> cassettes, <YYYY-MM-DD>). Raw record: cassette export <ID>.
  ```

## 5. Report

In chat: the path, a one-paragraph thesis, and what stayed open. The document is the
deliverable; don't paste it into the chat.

## Red flags

| Thought | Reality |
|---|---|
| "The side-B line is the best quote in the session" | Side B is scratch. It never appears. |
| "Both answers are in the cassettes, I'll present both as live" | If a later cassette reversed an earlier one, the reversal is the answer. |
| "I'll just pick the more reasonable side" | An unresolved disagreement goes under Open questions, both positions stated. |
| "Crediting who said what is more transparent" | The distillation model has no speakers. Attribution is metadata, not prose. |
| "This step is obviously implied" | If no cassette says it, it's a gap. It goes under Open questions. |
| "I'll put the summary in a new cassette so it stays with the session" | Never. A file, confirmed in chat. |
| "The outline is obvious, I'll skip straight to writing" | Outline first. Always. |
