# Katana Desktop

An unofficial native Windows/macOS/Linux client for the user-created puzzles of
[Nonograms Katana](https://nonograms-katana.com/), synced with your Katana account.

> This is an independent fan project. It is not made, endorsed or supported by the developers of
> Nonograms Katana. "Nonograms Katana" is their trademark, and the puzzles belong to their authors.

It's a single Rust binary (~4 MB) built with [wry](https://github.com/tauri-apps/wry)/[tao](https://github.com/tauri-apps/tao).
The UI runs in the system webview (WebView2 on Windows, WKWebView on macOS, WebKitGTK on Linux) and is embedded in the binary.
There's no local server: the page talks to Rust over IPC.

## Install

Download a package for your system from [Releases](https://github.com/ArkadyBuryakov/katana-desktop/releases/latest):

- **Arch Linux and derivatives:** from the AUR, `yay -S katana-desktop-bin` (prebuilt) or `yay -S katana-desktop` (built from source).
- **Debian, Ubuntu, Mint, Pop!_OS (22.04+):** `katana-desktop_<version>_amd64.deb`, install with `sudo apt install ./katana-desktop_*.deb`.
- **Fedora:** `katana-desktop-<version>-1.x86_64.rpm`, install with `sudo dnf install ./katana-desktop-*.rpm`.
- **Other Linux:** `katana-desktop-<version>-linux-<arch>.tar.gz` holds a `usr/` tree; it needs `webkit2gtk-4.1`.
- **Windows:** `KatanaDesktop-<version>-setup.exe` or `-portable.exe` (see below).
- **macOS 11+:** `KatanaDesktop-<version>-macos-universal.dmg`. The app isn't notarized, so on first launch
  macOS refuses to open it: allow it in System Settings → Privacy & Security → "Open Anyway",
  or run `xattr -dr com.apple.quarantine "/Applications/Katana Desktop.app"`.

## Build & run

```sh
make              # build target/release/katana-desktop
make install      # Linux: ~/.local/bin + app-menu entry; macOS: ~/Applications/Katana Desktop.app
make uninstall    # remove them again (your data is kept)
make run | test | clean
```

Variables: on Linux `PREFIX` (default `~/.local`) and `DESTDIR` for packaging, e.g.
`make && sudo make install PREFIX=/usr/local`; on macOS `APPDIR` (default `~/Applications`).

- **Linux:** needs `webkit2gtk-4.1` and `gtk3` (`pacman -S webkit2gtk-4.1`, `apt install libwebkit2gtk-4.1-dev`).
- **macOS:** needs the Xcode command line tools. `make install` runs `scripts/bundle-macos.sh` to build the `.app`.
  You can also run that script directly with `aarch64-apple-darwin` or `x86_64-apple-darwin` to build for a specific chip.
- **Windows 10/11:** needs Rust from [rustup.rs](https://rustup.rs) with the MSVC build tools; WebView2 ships with Windows.
  `make` isn't usual on Windows, so there's a PowerShell equivalent:
  ```powershell
  powershell -ExecutionPolicy Bypass -File scripts\windows.ps1             # build
  powershell -ExecutionPolicy Bypass -File scripts\windows.ps1 -Install    # %LOCALAPPDATA%\Programs, Start menu, Apps & features
  powershell -ExecutionPolicy Bypass -File scripts\windows.ps1 -Uninstall  # or uninstall from Apps & features
  ```
  To build Windows releases from Linux: install `mingw-w64-gcc` and `nsis`, run
  `rustup target add x86_64-pc-windows-gnu`, then `make windows`. That writes:
  - `dist/KatanaDesktop-<version>-setup.exe`: an installer for your user only, no admin rights needed.
    It adds a Start menu shortcut, an optional desktop shortcut, and an uninstaller in Apps & features.
  - `dist/KatanaDesktop-<version>-portable.exe`: a single file that runs without installing anything.
    It unpacks itself to a temp folder, which is removed when you close the app.

  Both keep progress and login in `%LOCALAPPDATA%\katana-desktop`, so they share them with each other.
  Neither is code-signed, so Windows SmartScreen may ask for confirmation on first run.

Data (session, catalog cache, puzzle images, in-progress boards) lives in
`~/.local/share/katana-desktop` on Linux, `~/Library/Application Support/katana-desktop` on macOS
and `%LOCALAPPDATA%\katana-desktop` on Windows.
Set `KATANA_DATA` to use another directory.

## Playing

| | |
| --- | --- |
| Left / right click | fill / cross; drag paints a straight line (its length shows in the corner) |
| Click a clue number | cross it out (solved numbers are crossed out automatically; toggle in the sidebar) |
| Arrows or `h j k l` | move the cursor over the board **and** the clue numbers; Shift moves 5 |
| `Space` | cycle fill → cross → empty; hold it while moving to repeat the same mark; on a clue it crosses the number out |
| `1`–`0`, `Shift`+`1`–`0` | colours 1–10, 11–20 |
| `X` | cross tool |
| Wheel, Shift+wheel, Ctrl+wheel, middle-drag | scroll, scroll sideways, zoom, pan |
| `0` / Fit | auto-fit the puzzle to the window (stays on until you zoom) |
| `[` | collapse / expand the sidebar |
| `Ctrl+Z`, `Ctrl+Y` | undo, redo |
| `Esc` | back to the list |

Sidebar helpers, all off by default except the first:

- **Cross out solved numbers** strikes through a clue number once its block is finished.
- **Cross empty cells of finished lines** fills the rest of a finished row or column with crosses.
- **Cross gaps around solved numbers** crosses the cells next to a block that can't grow any further,
  everything between two solved neighbouring numbers, and everything between the border and a solved
  first or last number.
- **Help** (bottom of the sidebar, also when collapsed) does the first of these that applies:
  fixes one mistake; fills in a block, part of a block or a gap that a single row or column gives away,
  and highlights that line; reveals one random cell. The highlight stays until your next move.
  The button counts the helps used on the puzzle; the count is saved with the board and shown when you solve it.

## What syncs

An account is optional. Without one, solves and boards are kept on this device only. When you log in,
solves made as a guest are uploaded to the account. Logging out clears the local list of solved
puzzles; in-progress boards stay on the device.

The Katana account only stores *which* puzzles are solved, plus total score and play time, and that
is what gets synced in both directions. Half-finished boards are saved locally on each device, the same
as in the official apps.

When you solve a puzzle, the app downloads the account blob, adds the puzzle id to the user-puzzle
(`dwl:`) list, adds the cell count to your score and the play time to your total time, then uploads it.
Every other section of the blob is passed through byte for byte.

## Protocol notes (reverse engineered from the GWT web client)

| What | How |
| --- | --- |
| Login | `POST ucdevs.com/ujc/login.php` `op=login&idn1=<email>&idn2=base64(sha256(SALT+password))&uid=<32 hex device id>` → magic `0x1A8D19C2`, status, user id, token, nickname. `op=register` adds `nickname`; `op=logout` takes `idn1=<user id>&idn2=<token>`. Errors come back as `text/html` messages. |
| Catalog | `GET getlist10.php?aid=0&ts=0` → packets (magic `0x1A7804B1`) of raw-deflated puzzle records + MD5 |
| Puzzle | `GET puzw/user/<hex(((id*7+1048583)<<8) \| ((id&4095)*69069&255))>.png`, one pixel per cell |
| Sync | `POST syncget.php` / multipart `POST syncput.php` with `idn1=<user id>&idn2=<token>`; blob = magic `0x1A1CC80B`, version 3, gzip(categories, score, lists, time) |

See `src/protocol.rs`. `cargo test` covers the format. With `KATANA_SAMPLES=<dir>` holding a captured
`catalog.bin` and `sync.bin`, it also checks that real data parses and that the sync blob round-trips byte for byte.

## Development

To release: `make release patch` (or `minor`, `major`, `rc` for a release candidate, or an exact `x.y.z` / `x.y.z-rcN`), then push the tag it prints. `.github/workflows/release.yml` then builds
the Linux (x86_64 and aarch64 tar.gz, .deb, .rpm), Windows and macOS packages, publishes a GitHub release
with them and their `SHA256SUMS`, and pushes `packaging/aur/*` to the AUR with the new version and checksums.
Tags like `v1.2.0-rc1` make a prerelease and skip the AUR. The AUR step needs an `AUR_SSH_PRIVATE_KEY`
repository secret: the private half of an SSH key added to the AUR account that maintains the packages.

Debug builds can also serve the UI over HTTP for browser-based testing:
`KATANA_DEV_HTTP=8766 cargo run` (add `KATANA_HEADLESS=1` to skip the window). IPC is then `POST /ipc`.

## License

[MIT](LICENSE). The license covers this project's code only, not the Nonograms Katana service or its content.
