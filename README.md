# cassette

Freewrite with the cassette TUI

## Getting started

### Prebuilt Binary

Download the latest release form the Releases tab. Only Linux supported at this time.

### Build from source

You'll need the [Rust toolchain](https://rustup.rs). Then:

```
cargo install --path .
```

Once installed launch with `cassette`.

## Cassette interface

Each cassette is displayed as a cassette widget. Two reels flank a cassette window: the left reel fills
as you write, the right reel depletes, and the hub animates on every keypress. A stats line inside
the cassette window shows your timer and word count.

## Timed sessions

Start a timed session with the `-t` flag followed by the number of minutes:

```
cassette -t 10
```

A countdown timer appears inside each cassette in green. When time expires it turns red.
The session keeps running so you can finish your thought. Press `Esc` to quit as usual.

## Word goal

Set a word-count target with the `-w` flag:

```
cassette -w 500
```

Your current word count and the goal display inside the cassette stats line (`47 / 500`).
Combine both flags to show timer and word count together:

```
cassette -t 10 -w 500
```

## Releasing a new version

1. Bump `version` in `Cargo.toml` (e.g. `0.7.0`) and merge all changes to `main`.
2. On GitHub, go to **Releases → Draft a new release**.
3. Create a new tag (e.g. `v0.7.0`) targeting `main`.
4. Write release notes, then click **Publish release**.

The [Release workflow](.github/workflows/release.yml) will automatically build the Linux binary
and attach `cassette-linux-x86_64.tar.gz` to the release. First run may take longer due to cache
warming; subsequent releases should be faster.

## CI security: pinned Actions

The release workflow pins every third-party GitHub Action to a specific commit SHA rather than a
mutable tag like `@v2` or `@stable`. This prevents a compromised action repository from silently
pushing malicious code under an existing tag and having it run in your CI pipeline — a class of
attack that has affected several popular actions (tj-actions, reviewdog, and others) in 2025–2026.

The pins currently in use:

| Action | Tag | Pinned SHA |
|---|---|---|
| `dtolnay/rust-toolchain` | `stable` | `29eef336d9b2848a0b548edc03f92a220660cdb8` |
| `softprops/action-gh-release` | `v2` | `3bb12739c298aeb8a4eeaf626c5b8d85266b0e65` |

**Updating a pin:** when you want to pick up a newer version of an action, resolve the new SHA and
update the workflow manually:

```bash
# find the SHA the tag currently points to
gh api repos/softprops/action-gh-release/tags \
  --jq '.[] | select(.name=="v2") | .commit.sha'
```

Then replace the SHA in `.github/workflows/release.yml` and leave a `# v2` comment so the intent
stays readable.

## Feature ideas

### Visual indication of progress toward word goal
Have the words that go off the screen fill up some kind of container
so that users can see how much progress they have made toward a goal of
writing a certain amount of words

### Daily freewrite management
What does it look like to use this tool to do a daily free-writing session?

### Persist cassettes between sessions
In freewriting what data would you want to persist between sessions?

### Export for org mode and markdown
Make it so that you can save your daily sessions to other formats and
be able to view them that way.

### Cassettes return to the beginning after fixed number of characters so you can review what you

### Customise the hub animation
The reel hub animation frames are defined in a top-level constant `reelFrames` in `app/Main.hs`.
Edit the list of `(leftSpokeChar, rightSpokeChar)` pairs to change the look; list length controls speed.
