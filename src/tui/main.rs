//! Katana TUI: Katana Desktop in a terminal, on the same account, puzzle list and saved boards.

mod app;
mod board;
mod game;
mod theme;
mod ui;

use std::io::{self, stdout};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use katana_desktop::store::Store;
use ratatui::DefaultTerminal;
use ratatui::crossterm::event::{
    self, DisableBracketedPaste, DisableFocusChange, DisableMouseCapture, EnableBracketedPaste,
    EnableFocusChange, EnableMouseCapture, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
    PushKeyboardEnhancementFlags,
};
use ratatui::crossterm::execute;
use ratatui::crossterm::terminal::supports_keyboard_enhancement;

use app::{App, Msg, Screen};

/// Whether the terminal has been asked to report key releases (kitty keyboard protocol).
static ENHANCED: AtomicBool = AtomicBool::new(false);

/// Key releases are only asked for on the board, where Space can be held: everywhere else
/// keys arrive the plain way, which is what text fields work best with.
fn enhance(on: bool) {
    if ENHANCED.swap(on, Ordering::Relaxed) == on {
        return;
    }
    let _ = if on {
        // all keys as escape codes: otherwise Space is plain text and has no release
        let flags = KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
            | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
            | KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS
            | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES;
        execute!(stdout(), PushKeyboardEnhancementFlags(flags))
    } else {
        execute!(stdout(), PopKeyboardEnhancementFlags)
    };
}

/// Undo everything `main` asked the terminal for, besides what ratatui restores itself.
fn leave() {
    enhance(false);
    let _ = execute!(
        stdout(),
        DisableMouseCapture,
        DisableBracketedPaste,
        DisableFocusChange
    );
}

fn run(
    terminal: &mut DefaultTerminal,
    app: &mut App,
    rx: &Receiver<Msg>,
    can_enhance: bool,
) -> io::Result<()> {
    let mut next_tick = Instant::now();
    while !app.quit {
        enhance(can_enhance && app.screen == Screen::Play);
        if std::mem::take(&mut app.dirty) {
            terminal.draw(|f| ui::render(app, f))?;
        }
        match rx.recv_timeout(next_tick.saturating_duration_since(Instant::now())) {
            Ok(msg) => {
                app.on_msg(msg);
                while let Ok(msg) = rx.try_recv() {
                    app.on_msg(msg);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        let now = Instant::now();
        if now >= next_tick {
            next_tick = now + Duration::from_millis(100);
            app.tick(now);
        }
    }
    Ok(())
}

fn main() -> io::Result<()> {
    if let Some(arg) = std::env::args().nth(1) {
        match arg.as_str() {
            "-V" | "--version" => println!("katana-tui {}", env!("CARGO_PKG_VERSION")),
            _ => println!(
                "katana-tui {}\n{} in a terminal.\n\n\
                 It takes no arguments. Data is shared with Katana Desktop; set KATANA_DATA to use another directory.",
                env!("CARGO_PKG_VERSION"),
                env!("CARGO_PKG_DESCRIPTION")
            ),
        }
        return Ok(());
    }

    let store = Store::open();
    {
        let s = store.clone();
        std::thread::spawn(move || {
            s.load_catalog(false);
            if s.session.lock().unwrap().is_some() {
                let _ = s.sync();
            }
        });
    }

    let mut terminal = ratatui::init();
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        leave();
        hook(info);
    }));
    execute!(
        stdout(),
        EnableMouseCapture,
        EnableBracketedPaste,
        EnableFocusChange
    )?;
    // Asked before the reader thread starts: the answer comes in on stdin.
    // KATANA_KEY_RELEASE=0/1 overrides it, for terminals that get it wrong.
    let can_enhance = match std::env::var("KATANA_KEY_RELEASE").as_deref() {
        Ok(v) => v == "1",
        Err(_) => !cfg!(windows) && supports_keyboard_enhancement().unwrap_or(false),
    };
    // the Windows console always reports releases
    let key_release = can_enhance || cfg!(windows);

    let (tx, rx) = mpsc::channel();
    {
        let tx = tx.clone();
        std::thread::spawn(move || {
            while let Ok(ev) = event::read() {
                if tx.send(Msg::Term(ev)).is_err() {
                    break;
                }
            }
        });
    }
    let size = terminal.size()?;
    let mut app = App::new(store, tx, (size.width, size.height), key_release);
    let res = run(&mut terminal, &mut app, &rx, can_enhance);
    leave();
    ratatui::restore();
    res
}
