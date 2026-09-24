#!/usr/bin/env bash
# Builds a small session that exercises every cassette-writeup rule, in a
# throwaway store. Usage:
#   eval "$(.claude/skills/cassette-writeup/fixture.sh)"   # exports CASSETTE_DATA_DIR and SID
# Set CASSETTE_BIN to use a dev build (default: `cassette` on PATH).
set -euo pipefail
BIN=${CASSETTE_BIN:-cassette}
export CASSETTE_DATA_DIR=${CASSETTE_DATA_DIR:-$(mktemp -d)/store}
unset CASSETTE_WRITER
$BIN writer register --name me --kind human >/dev/null
$BIN writer register --name bot --kind agent >/dev/null
SID=$($BIN session new --alias storage-choice)

w() { # w <writer> <id> [side] ; body on stdin
  $BIN --writer "$1" queue write "$2" --session "$SID" ${3:+--side "$3"} >/dev/null
}
new() { $BIN --writer "$1" queue new "$2" --session "$SID"; }

# A question, answered one way first...
A=$(new me "where do notes live")
printf 'Notes should be one markdown file per session. Simple, greppable, ours.\n' | w me "$A"
# ...with a tempting quote on the scratch side, which must never be quoted.
printf 'honestly maybe sqlite?? "a database is just a file with opinions"\n' | w me "$A" b

B=$(new bot "concurrency")
printf 'One file per session cannot be written by two writers at once without a lock on the whole session. Per-cassette files let a human and an agent write different cassettes concurrently.\n' | w bot "$B"

# ...then reversed in a later cassette. The pause gives it a later
# `updated_at` (seconds precision), which is what rule 2 orders by.
sleep 1
C=$(new me "reversal")
printf 'Changed my mind: per-cassette files, one directory per session. One-file-per-session is out.\n' | w me "$C"

# A disagreement that is still open. The bot wrote first; the human rewrote the
# cassette into the current (unresolved) shared state.
D=$(new bot "locking")
printf 'Use flock per cassette. Question still open: does flock hold on network filesystems? Unknown; we have not tested.\n' | w bot "$D"
printf 'Use flock per cassette. Network filesystems: bot thinks unsafe, I think fine for a single user. Unresolved.\n' | w me "$D"

# Closed as abandoned.
E=$(new me "sqlite")
printf 'Tried sketching a sqlite schema.\n' | w me "$E"
$BIN --writer me queue close "$E" --session "$SID" -m "abandoned: plain text is the point"

# Closed with a verdict.
F=$(new bot "atomic writes")
printf 'Write to a temp file and rename, so readers never see a torn cassette.\n' | w bot "$F"
$BIN --writer me queue close "$F" --session "$SID" -m "agreed, done"

# Untitled.
G=$(new me "placeholder")
$BIN --writer me queue topic "$G" --session "$SID" ""
printf 'Also: sessions need ids, not names, because two sessions can share a name.\n' | w me "$G"

echo "export CASSETTE_DATA_DIR='$CASSETTE_DATA_DIR' SID='$SID'"
