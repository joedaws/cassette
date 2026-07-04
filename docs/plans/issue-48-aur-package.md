# Plan: package cassette for the AUR (chainlink #48)

**Prerequisite:** #12 (a GitHub release tag with the final Cargo.toml metadata; the
PKGBUILD builds from the tag tarball, not crates.io, so strictly it needs a tagged
release more than the crates.io publish itself).
**Deliverable:** an AUR package installable with an AUR helper on Arch.
**Agent-executable:** mostly — AUR account + SSH key setup and the final `git push`
to aur.archlinux.org are user steps. Local build/test is fully agent-runnable (the
dev machine is Arch).

## Steps

1. Name check: search https://aur.archlinux.org/packages and the official repos
   (`pacman -Si cassette` / `pacman -Ss '^cassette'`) for collisions. If `cassette`
   is taken, use `cassette-tui` as the **package** name; the installed binary is
   `cassette` either way (note the conflict in `conflicts=()` if a same-binary
   package exists). Keep the name consistent with the crates.io decision from #5.
2. Create the PKGBUILD in a separate local dir (AUR packages live in their own repo,
   not in this one — but keep a copy in-repo under `packaging/aur/PKGBUILD` so it's
   versioned). Skeleton:
   ```bash
   # Maintainer: Joseph Daws <daws.joseph@gmail.com>
   pkgname=cassette-tui        # or cassette, per name check
   _binname=cassette
   pkgver=1.0.0                # match the release tag
   pkgrel=1
   pkgdesc="TUI freewriting app: write on tape-like cassettes with timers, word goals, and themes"
   arch=(x86_64)
   url="https://github.com/joedaws/cassette"
   license=(BSD-3-Clause)
   depends=(gcc-libs)
   makedepends=(cargo)
   source=("$pkgname-$pkgver.tar.gz::$url/archive/v$pkgver.tar.gz")
   sha256sums=(...)            # updpkgsums fills this

   prepare() { cd "cassette-$pkgver"; cargo fetch --locked --target "$(rustc -vV | sed -n 's/host: //p')"; }
   build()   { cd "cassette-$pkgver"; cargo build --release --frozen; }
   check()   { cd "cassette-$pkgver"; cargo test --frozen; }
   package() {
     cd "cassette-$pkgver"
     install -Dm755 "target/release/$_binname" "$pkgdir/usr/bin/$_binname"
     install -Dm644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
     # once #50 lands in the tag:
     # install -Dm644 cassette.1 "$pkgdir/usr/share/man/man1/cassette.1"
     # install -Dm644 completions/cassette.bash "$pkgdir/usr/share/bash-completion/completions/cassette"
     # install -Dm644 completions/_cassette "$pkgdir/usr/share/zsh/site-functions/_cassette"
     # install -Dm644 completions/cassette.fish "$pkgdir/usr/share/fish/vendor_completions.d/cassette.fish"
   }
   ```
   Note: the repo must have `Cargo.lock` checked in for `--locked/--frozen` (verify;
   it should be for a binary crate).
3. Local test: `updpkgsums && makepkg -si` (or `makepkg --check`), then run
   `cassette -V`. Cleanest is a chroot build (`extra-x86_64-build` from devtools) if
   available; plain `makepkg` is acceptable.
4. Generate `.SRCINFO`: `makepkg --printsrcinfo > .SRCINFO`.
5. **Human steps:** create/confirm AUR account, add SSH key, then:
   ```bash
   git clone ssh://aur@aur.archlinux.org/<pkgname>.git
   # copy PKGBUILD + .SRCINFO in, commit, push
   ```
6. Verify with an AUR helper from a clean state (`yay -Si <pkgname>`), comment the
   AUR URL on #48, close it. Update README install section with the AUR package.

## Acceptance criteria

- `makepkg -si` builds and installs; `cassette -V` matches the tag.
- Package page live on aur.archlinux.org; `.SRCINFO` in sync with PKGBUILD.
- In-repo copy at `packaging/aur/PKGBUILD`; README mentions the AUR package.
