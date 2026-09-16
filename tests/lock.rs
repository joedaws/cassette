//! Cross-process lock behaviour. Deterministic by construction: contention is
//! never assumed by spawn order — see `contend` below — and the release
//! signal is always a closed stdin pipe, an open one, or a real kill. Nothing
//! here sleeps or polls.

use std::io::{Read, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_cassette")
}

const SESSION: &str = "01K5GQ2R8V3XQZ0000000000AB";
const ID: &str = "01K5GR7T2M9WPD0000000000AB";

/// Build the store layout directly. Deliberately not through the CLI: the
/// commands that would do it arrive in Phase 4, and a hand-built fixture keeps
/// these tests honest about the on-disk format.
fn fixture() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("store");
    let cassettes = root.join("sessions").join(SESSION).join("cassettes");
    std::fs::create_dir_all(&cassettes).expect("mkdir");
    std::fs::create_dir_all(root.join("sessions").join(SESSION).join(".locks")).expect("mkdir");
    std::fs::write(
        root.join("sessions").join(SESSION).join("session.toml"),
        "created = \"2026-09-14T09:25:57Z\"\n",
    )
    .expect("session.toml");
    std::fs::write(
        cassettes.join(format!("gratitude-{ID}.md")),
        format!(
            "---\nid: {ID}\ntopic: gratitude\npriority: 10\nstatus: open\nlocked_by:\n\
             created_by: w\nlast_writer: w\nupdated_at: 2026-09-14T09:25:57Z\n---\n\n\
             ## Side A\n\noriginal\n"
        ),
    )
    .expect("cassette");
    (dir, root)
}

/// Spawn a `queue write` with an open stdin pipe, writing a throwaway body.
///
/// Only used by `contend`, where one of the two children is expected to lose
/// the race and exit almost immediately — before ever reading stdin — which
/// closes its read end. The write can therefore legitimately race a fast
/// exit and return `BrokenPipe`; that is not a test failure; it just means
/// this child turned out to be the loser, which `contend` determines
/// independently. Any other write error still propagates.
fn spawn_write(root: &std::path::Path, id: &str, user: &str) -> Child {
    let mut child = Command::new(bin())
        .args(["queue", "write", id, "--session", SESSION])
        .env("CASSETTE_DATA_DIR", root)
        .env("USER", user)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    match child.stdin.as_mut().expect("stdin").write_all(b"body\n") {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {}
        Err(e) => panic!("write: {e}"),
    }
    child
}

fn try_write(root: &std::path::Path, body: &str) -> std::process::Output {
    let mut child = Command::new(bin())
        .args(["queue", "write", ID, "--session", SESSION])
        .env("CASSETTE_DATA_DIR", root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(body.as_bytes())
        .expect("write");
    child.wait_with_output().expect("wait")
}

/// Spawn two `queue write` invocations racing for the same cassette lock, and
/// determine — after the fact, never by assumption — which one lost.
///
/// The naive approach (spawn a "holder" first, write a byte to its stdin, then
/// assume a second, freshly spawned process must lose to it) does not work:
/// writing to a pipe only proves the buffer accepted the byte, not that the
/// child read it, and two freshly forked processes racing to the same
/// instruction are close enough in startup cost that either can win. That
/// version of this suite measured roughly 1 failure in 6-15 runs on this
/// machine — not a hypothetical, an observed failure.
///
/// This instead races their *completions*, which needs no assumption about
/// who acquires first: exactly one of two non-blocking `try_lock` attempts on
/// the same flock must fail, so exactly one process is the loser and exits
/// almost immediately (before ever reading stdin), while the other — the
/// winner — blocks reading its own stdin, which nothing has closed, and
/// cannot exit on its own. Reading each child's stdout to EOF on its own
/// thread and taking whichever finishes first over a channel is a real
/// blocking wait (`Receiver::recv` returns exactly when a thread sends,
/// never before, and never by polling): a process's pipes close as part of
/// its own exit, so the loser's stdout reader unblocks first.
///
/// Deliberately does not use `wait_with_output`: it closes the child's stdin
/// before waiting, which would hand a genuine winner its EOF immediately and
/// let it finish before the caller can use it as a live, lock-holding
/// process.
fn contend(root: &std::path::Path, id: &str) -> (std::process::Output, Child) {
    let mut a = spawn_write(root, id, "contender-a");
    let mut b = spawn_write(root, id, "contender-b");
    let a_id = a.id();
    let b_id = b.id();
    let a_out = a.stdout.take().expect("stdout a");
    let b_out = b.stdout.take().expect("stdout b");

    let (tx, rx) = mpsc::channel();
    let tx2 = tx.clone();
    thread::spawn(move || {
        let mut out = a_out;
        let mut buf = Vec::new();
        let _ = out.read_to_end(&mut buf);
        let _ = tx.send(a_id);
    });
    thread::spawn(move || {
        let mut out = b_out;
        let mut buf = Vec::new();
        let _ = out.read_to_end(&mut buf);
        let _ = tx2.send(b_id);
    });

    // Blocks until whichever child's stdout closes first — the loser's, since
    // the winner's stays open until this function's caller acts on it.
    let first = rx.recv().expect("recv");
    let (loser, winner) = if first == a_id { (a, b) } else { (b, a) };
    let out = loser.wait_with_output().expect("wait loser");
    (out, winner)
}

#[test]
fn an_uncontended_write_succeeds() {
    let (_d, root) = fixture();
    let out = try_write(&root, "rewritten\n");
    assert_eq!(out.status.code(), Some(0), "{:?}", out);
    let text = std::fs::read_to_string(
        root.join("sessions")
            .join(SESSION)
            .join("cassettes")
            .join(format!("gratitude-{ID}.md")),
    )
    .expect("read");
    assert!(text.contains("rewritten"), "{text}");
}

#[test]
fn a_second_writer_gets_exit_three_and_is_told_who_holds_it() {
    let (_d, root) = fixture();
    let (loser_out, mut winner) = contend(&root, ID);

    assert_eq!(loser_out.status.code(), Some(3), "{:?}", loser_out);
    let err = String::from_utf8_lossy(&loser_out.stderr);
    assert!(err.contains("is open by"), "{err}");
    assert!(err.contains("try again later"), "{err}");

    // Releasing lets the next writer through: closing the winner's stdin
    // hands it EOF, so it finishes its write and exits normally, just like a
    // real writer that closes its input when done.
    drop(winner.stdin.take());
    let done = winner.wait().expect("wait winner");
    assert!(done.success(), "{:?}", done);
    assert_eq!(try_write(&root, "now ok\n").status.code(), Some(0));
}

#[cfg(unix)]
#[test]
fn killing_the_holder_releases_the_lock() {
    // The property the no-reaper decision rests on: the kernel releases an
    // flock on process death for any reason, including SIGKILL. If a future
    // refactor swapped in a create-to-lock/delete-to-unlock scheme, this test
    // is what would catch it.
    let (_d, root) = fixture();
    let (loser_out, mut winner) = contend(&root, ID);
    assert_eq!(
        loser_out.status.code(),
        Some(3),
        "exactly one of the two racing writers must be denied, which proves \
         the other actually holds the lock we are about to kill: {:?}",
        loser_out
    );

    winner.kill().expect("kill");
    winner.wait().expect("reap");

    assert_eq!(
        try_write(&root, "after the kill\n").status.code(),
        Some(0),
        "a SIGKILLed holder must not leave the lock stuck"
    );
}

#[test]
fn the_registry_lock_waits_rather_than_failing() {
    // The registry is the ONE lock in this system that blocks, and this is the
    // only cross-process check of that. A caller who cannot register a writer
    // has no fallback, so contention must make it wait — never return Busy.
    //
    // Honest about what this proves: the load-bearing assertion is the exit
    // code. We hold the registry anchor before the child is spawned, so the
    // child necessarily contends; a non-blocking implementation would surface
    // that contention as a non-zero exit, and a blocking one completes with 0.
    // The `try_wait` check below is corroboration, not proof — a child that has
    // merely not been scheduled yet also reports "still running". Proving
    // "it blocked" rather than "it did not fail" would need the child to signal
    // the instant before it acquires, which it has no way to do. The blocking
    // property is therefore established by construction (`Blocking::Yes` maps
    // to `FileExt::lock`, verified in Task 5's review) and corroborated here.
    let (_d, root) = fixture();
    std::fs::create_dir_all(root.join(".locks")).expect("mkdir");
    let anchor = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join(".locks").join("writers"))
        .expect("open the registry anchor");
    // std's inherent `File::lock` is `flock(2)` on Unix and interoperates with
    // the binary's fs4 lock — verified directly against this binary.
    anchor.lock().expect("hold the registry");

    let mut child = Command::new(bin())
        .args(["queue", "write", ID, "--session", SESSION])
        .env("CASSETTE_DATA_DIR", &root)
        .env("USER", "someone-new")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(b"body\n")
        .expect("write");

    // Corroboration only — see the note above.
    assert!(
        child.try_wait().expect("try_wait").is_none(),
        "registration should still be waiting on the registry lock"
    );

    drop(anchor);
    let out = child.wait_with_output().expect("wait");
    assert_eq!(
        out.status.code(),
        Some(0),
        "registration must complete once the registry frees, never fail on \
         contention (exit 3 would mean the registry lock became non-blocking): {out:?}"
    );
}

/// Write a throwaway body and close the pipe. Whichever of two racing writers
/// loses the cassette lock exits before ever reading stdin, without waiting
/// for us — so this write can legitimately race that exit and see
/// `BrokenPipe` rather than a clean write. That is not a test failure, just
/// this child turning out to be the loser; any other error still panics.
fn write_and_close(stdin: std::process::ChildStdin, body: &[u8]) {
    let mut stdin = stdin;
    match stdin.write_all(body) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::BrokenPipe => {}
        Err(e) => panic!("write: {e}"),
    }
}

#[test]
fn registering_writers_concurrently_keeps_both() {
    // The registry lock blocks rather than failing: two processes registering
    // at once must both succeed, because a caller that cannot register has no
    // fallback. Exit 3 here would be a regression.
    let (_d, root) = fixture();
    let spawn = |user: &str| {
        Command::new(bin())
            .args(["queue", "write", ID, "--session", SESSION])
            .env("CASSETTE_DATA_DIR", &root)
            .env("USER", user)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn")
    };
    let mut a = spawn("joseph");
    let mut b = spawn("agent");
    write_and_close(a.stdin.take().expect("stdin"), b"a\n");
    write_and_close(b.stdin.take().expect("stdin"), b"b\n");
    let ao = a.wait_with_output().expect("wait");
    let bo = b.wait_with_output().expect("wait");

    // One of them may lose the cassette lock (exit 3) — that is expected and
    // is not what this test is about. Neither may fail to register.
    for out in [&ao, &bo] {
        let code = out.status.code();
        assert!(
            code == Some(0) || code == Some(3),
            "registration must not fail: {:?}",
            out
        );
    }
    let registry = std::fs::read_to_string(root.join("writers.toml")).expect("read");
    assert!(registry.contains("joseph"), "{registry}");
    assert!(registry.contains("agent"), "{registry}");
}
