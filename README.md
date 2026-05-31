# cassette

Freewrite with the cassette TUI

## Getting started

### Prebuilt Binary

Download the latest release form the Releases tab. Only Linux supported at this time.

### build from source

Cloning this repo and then building the project is another way to get
the project. You'll need haskell's Stack.

Install with
```
stack install
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

1. Bump `version` in `package.yaml` (e.g. `0.2.0.0`) and merge all changes to `main`.
2. On GitHub, go to **Releases → Draft a new release**.
3. Create a new tag (e.g. `v0.2.0`) targeting `main`.
4. Write release notes, then click **Publish release**.

The [Release workflow](.github/workflows/release.yml) will automatically build the Linux binary
and attach `cassette-linux-x86_64.tar.gz` to the release. First run may take longer (30–45 minutes) due to cache warming;
subsequent releases are should hopefully be faster.

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
