# Katana Desktop

A native macOS/Linux client for the user-created puzzles of
[Nonograms Katana](https://nonograms-katana.com/), synced with your Katana account.

It's a single Rust binary (~4 MB) built with [wry](https://github.com/tauri-apps/wry)/[tao](https://github.com/tauri-apps/tao).
The UI runs in the system webview (WKWebView on macOS, WebKitGTK on Linux) and is embedded in the binary.
There's no local server: the page talks to Rust over IPC.

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

Data (session, catalog cache, puzzle images, in-progress boards) lives in
`~/.local/share/katana-desktop` on Linux or `~/Library/Application Support/katana-desktop` on macOS.
Set `KATANA_DATA` to use another directory.

## Playing

| | |
| --- | --- |
| Left / right click | fill / cross; drag paints a straight line (its length shows in the corner) |
| Click a clue number | cross it out (finished numbers also grey out automatically) |
| Arrows or `h j k l` | move the cursor over the board **and** the clue numbers; Shift moves 5 |
| `Space` | cycle fill → cross → empty; hold it while moving to repeat the same mark; on a clue it crosses the number out |
| `1`–`0`, `Shift`+`1`–`0` | colours 1–10, 11–20 |
| `X` | cross tool |
| Wheel, Shift+wheel, Ctrl+wheel, middle-drag | scroll, scroll sideways, zoom, pan |
| `0` / Fit | auto-fit the puzzle to the window (stays on until you zoom) |
| `[` | collapse / expand the sidebar |
| `Ctrl+Z`, `Ctrl+Y` | undo, redo |
| `Esc` | back to the list |

## What syncs

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

Debug builds can also serve the UI over HTTP for browser-based testing:
`KATANA_DEV_HTTP=8766 cargo run` (add `KATANA_HEADLESS=1` to skip the window). IPC is then `POST /ipc`.
