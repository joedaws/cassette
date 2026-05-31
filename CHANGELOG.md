# Changelog for `cassette`

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to the
[Haskell Package Versioning Policy](https://pvp.haskell.org/).

## 0.1.0.0 - 2025-11-13

### Changed
- update the .github workflows from stack and haskell working to new rust and cargo (#9)
- update .gitignore so that it is appropriate for a rust project (#10)
- Change the implementation so that we can configure N lines being shown. The initial behavior is that the cursor stays directly in the middle, but we should transition to something that is more like traditional editors, and show N lines (thinking about targeting 5 to 7 lines). The text show start to fade at the top. (#3)
- Rewrite the applicatoin in Rust using the ratatui library (#2)
- create new cargo project and initialize it with ratatui as dependency. Choose a structure of project that cleanly separates application state code, UI code, and main.rs (#4)
- Rename the project as cassette and make updates to the code base to replace tape with cassette for variables and types. (#1)

- Basic interface with Brick of the Cassette.
