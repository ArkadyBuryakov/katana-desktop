//! Application state and everything that changes it: keys, the mouse, and the results of
//! the network and disk work done on other threads.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use katana_desktop::protocol::{self, Board, Puzzle};
use katana_desktop::store::Store;
use ratatui::crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::board::View;
use crate::game::{Game, Kind, Opts, Rgb};
use crate::theme::{Theme, blend, terminal_is_dark};

/// Puzzles per page of the list; more are loaded as the selection nears the end.
const PER: usize = 60;
pub const SIDE_W: u16 = 34;
pub const RAIL_W: u16 = 5;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Screen {
    Browse,
    Login,
    Play,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Browse,
    Continue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Search,
    Author,
}

/// A filter with a fixed set of (value, label) options, opened with `key`.
pub struct Select {
    pub key: char,
    pub options: &'static [(&'static str, &'static str)],
}

pub const SELECTS: [Select; 5] = [
    Select {
        key: 's',
        options: &[
            ("0-999", "Any size"),
            ("0-15", "Small ≤15"),
            ("16-30", "Medium 16–30"),
            ("31-50", "Large 31–50"),
            ("51-999", "Huge 51–80"),
            ("31-999", "Large & huge (>30)"),
        ],
    },
    Select {
        key: 'c',
        options: &[
            ("all", "B/W & color"),
            ("bw", "Black & white"),
            ("color", "Color"),
        ],
    },
    Select {
        key: 't',
        options: &[
            ("unsolved", "Unsolved"),
            ("all", "All"),
            ("solved", "Solved"),
        ],
    },
    Select {
        key: 'o',
        options: &[
            ("newest", "Newest"),
            ("rating", "Best rated"),
            ("fans", "Most loved"),
            ("easy", "Easiest"),
            ("difficulty", "Hardest"),
            ("big", "Biggest"),
            ("small", "Smallest"),
            ("oldest", "Oldest"),
        ],
    },
    Select {
        key: 'r',
        options: &[
            ("0", "Any rating"),
            ("3", "★ 3+"),
            ("4", "★ 4+"),
            ("4.5", "★ 4.5+"),
        ],
    },
];
const STATUS: usize = 2;

/// What the web UI keeps in localStorage: here it is `tui.json` in the data directory.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub search: String,
    pub author: String,
    /// the chosen option of each of SELECTS
    pub selects: [usize; 5],
    pub auto_fit: bool,
    pub zoom: usize,
    pub auto_clues: bool,
    pub auto_cross: bool,
    pub auto_gaps: bool,
    pub side_collapsed: bool,
    /// None: follow the terminal
    pub dark: Option<bool>,
    /// Draw crosses with the Nerd Font icon. None: when a Nerd Font is installed
    pub nerd_font: Option<bool>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            search: String::new(),
            author: String::new(),
            selects: [0; 5],
            auto_fit: true,
            zoom: 0,
            auto_clues: true,
            auto_cross: false,
            auto_gaps: false,
            side_collapsed: false,
            dark: None,
            nerd_font: None,
        }
    }
}

/// A row of the puzzle list.
#[derive(Clone, Debug, Deserialize)]
pub struct Card {
    pub id: u32,
    pub w: u32,
    pub h: u32,
    pub color: bool,
    pub rating: f32,
    pub difficulty: f32,
    pub fans: f32,
    pub title: String,
    pub title2: String,
    pub author: String,
    pub solved: bool,
    /// percent filled in, for boards in progress
    pub progress: Option<u32>,
}

impl Card {
    pub fn name(&self) -> &str {
        if self.title.is_empty() {
            "Untitled"
        } else {
            &self.title
        }
    }
    /// The picture the list shows for it: the player's own board for one in progress (never
    /// the solution), the finished picture for a solved one.
    pub fn image(&self) -> Option<ImgKey> {
        match self.progress {
            Some(_) => Some((self.id, true)),
            None => self.solved.then_some((self.id, false)),
        }
    }
}

#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub struct Status {
    pub logged_in: bool,
    pub nickname: Option<String>,
    pub email: Option<String>,
    pub solved_count: usize,
    pub pending: usize,
    pub score: Option<i64>,
    pub sync_error: Option<String>,
    pub catalog_count: usize,
    pub catalog_time: f64,
    pub catalog_loading: bool,
    pub catalog_error: Option<String>,
}

/// (puzzle id, is the player's board rather than the solution)
pub type ImgKey = (u32, bool);

pub struct Img {
    pub w: usize,
    pub h: usize,
    pub px: Vec<Rgb>,
}

pub enum Preview {
    Loading,
    Ready(Img),
    Failed,
}

pub struct Loaded {
    board: Board,
    card: Card,
    progress: Value,
}

pub enum Msg {
    Term(Event),
    Cards {
        token: u64,
        reset: bool,
        total: usize,
        items: Vec<Card>,
    },
    Strip(Vec<Card>),
    Puzzle {
        token: u64,
        res: Result<Box<Loaded>, String>,
    },
    Image {
        key: ImgKey,
        res: Result<Img, String>,
    },
    LoggedIn(Result<(), String>),
    Synced(Result<usize, String>),
    LoggedOut(Result<(), String>),
}

#[derive(Default)]
pub struct Input {
    pub text: String,
    /// cursor position, in characters
    pub pos: usize,
}

impl Input {
    fn new(text: &str) -> Input {
        Input {
            text: text.to_string(),
            pos: text.chars().count(),
        }
    }
    fn byte(&self, pos: usize) -> usize {
        self.text
            .char_indices()
            .nth(pos)
            .map_or(self.text.len(), |(i, _)| i)
    }
    fn insert(&mut self, c: char) {
        self.text.insert(self.byte(self.pos), c);
        self.pos += 1;
    }
    /// Returns whether the text changed.
    fn key(&mut self, k: &KeyEvent) -> bool {
        let len = self.text.chars().count();
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Char('u') if ctrl => {
                self.pos = 0;
                let had = !self.text.is_empty();
                self.text.clear();
                return had;
            }
            KeyCode::Char('a') if ctrl => self.pos = 0,
            KeyCode::Char('e') if ctrl => self.pos = len,
            KeyCode::Char(c) if !ctrl && !k.modifiers.contains(KeyModifiers::ALT) => {
                self.insert(c);
                return true;
            }
            KeyCode::Backspace if self.pos > 0 => {
                self.pos -= 1;
                self.text.remove(self.byte(self.pos));
                return true;
            }
            KeyCode::Delete if self.pos < len => {
                self.text.remove(self.byte(self.pos));
                return true;
            }
            KeyCode::Left => self.pos = self.pos.saturating_sub(1),
            KeyCode::Right => self.pos = (self.pos + 1).min(len),
            KeyCode::Home => self.pos = 0,
            KeyCode::End => self.pos = len,
            _ => {}
        }
        false
    }
    fn paste(&mut self, s: &str) {
        s.chars()
            .filter(|c| !c.is_control())
            .for_each(|c| self.insert(c));
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginItem {
    Nickname,
    Email,
    Password,
    Submit,
    Switch,
    Guest,
}

#[derive(Default)]
pub struct Login {
    pub register: bool,
    pub nickname: Input,
    pub email: Input,
    pub password: Input,
    pub focus: usize,
    pub error: Option<String>,
    pub busy: bool,
}

impl Login {
    pub fn items(&self) -> Vec<LoginItem> {
        use LoginItem::*;
        let mut v = vec![Email, Password, Submit, Switch, Guest];
        if self.register {
            v.insert(0, Nickname);
        }
        v
    }
    pub fn focused(&self) -> LoginItem {
        self.items()[self.focus]
    }
    pub fn on_input(&self) -> bool {
        matches!(
            self.focused(),
            LoginItem::Nickname | LoginItem::Email | LoginItem::Password
        )
    }
    fn input(&mut self) -> Option<&mut Input> {
        match self.focused() {
            LoginItem::Nickname => Some(&mut self.nickname),
            LoginItem::Email => Some(&mut self.email),
            LoginItem::Password => Some(&mut self.password),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum BtnStyle {
    Ghost,
    Primary,
    Danger,
}

/// What a dialog button does, besides closing the dialog.
#[derive(Clone, Copy)]
pub enum Then {
    Close,
    DeleteProgress(u32),
    SolveAgain(u32),
    BackToList,
    ForceLogout,
    Reset,
}

pub enum ModalImg {
    None,
    Key(ImgKey),
    Own(Img),
}

/// One dialog for everything.
pub struct Modal {
    pub title: String,
    pub text: String,
    pub img: ModalImg,
    pub buttons: Vec<(&'static str, BtnStyle, Then)>,
    pub sel: usize,
}

#[derive(Clone, Copy)]
pub enum Pick {
    Login,
    Sync,
    Refresh,
    Theme,
    Logout,
    Quit,
    /// (which of SELECTS, which option)
    Option(usize, usize),
}

/// A popup list: the account menu and the filters' options.
pub struct Menu {
    /// top left corner, or top right with `right`
    pub at: (u16, u16),
    pub right: bool,
    pub head: Vec<String>,
    pub items: Vec<(String, Pick)>,
    pub sel: usize,
}

/// Something on screen that can be clicked. The renderer records where each one is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Act {
    Mode(Mode),
    UserMenu,
    Select(usize),
    Focus(Field),
    Row(usize),
    Author(usize),
    Open(u32),
    MenuItem(usize),
    CloseMenu,
    ModalBtn(usize),
    CloseModal,
    LoginItem(usize),
    Back,
    Collapse,
    Tool(Option<usize>),
    Undo,
    Redo,
    Zoom(i32),
    Fit,
    AutoClues,
    AutoCross,
    AutoGaps,
    Check,
    Reset,
    Help,
    Stage,
    /// swallows clicks, e.g. on a dialog's own background
    Nothing,
}

pub struct App {
    pub store: Arc<Store>,
    tx: Sender<Msg>,
    pub settings: Settings,
    pub theme: Theme,
    pub size: (u16, u16),
    pub status: Status,
    pub screen: Screen,
    pub quit: bool,
    /// something changed: draw again
    pub dirty: bool,
    /// counts 100 ms steps, for spinners
    pub ticks: usize,
    next_second: Instant,
    /// the terminal reports key releases, so Space can be held
    pub key_release: bool,
    /// what a crossed cell shows
    pub cross: char,

    // browse
    pub mode: Mode,
    pub focus: Option<Field>,
    pub search: Input,
    pub author: Input,
    pub items: Vec<Card>,
    pub total: usize,
    page: usize,
    pub loading: bool,
    load_token: u64,
    /// the list is empty because the catalog hasn't arrived yet
    pub waiting_for_catalog: bool,
    pub sel: usize,
    pub scroll: usize,
    /// rows of the list on screen, as last drawn
    pub list_h: usize,
    pub strip: Vec<Card>,
    pub previews: HashMap<ImgKey, Preview>,
    filter_at: Option<Instant>,
    /// when the selection last moved: its picture is fetched once it rests
    sel_at: Instant,
    /// where each filter's options open, as last drawn
    pub select_at: [(u16, u16); 5],

    pub login: Login,

    // play
    pub game: Option<Game>,
    pub meta: Option<Card>,
    pub view: View,
    open_token: u64,
    pub hover: Option<(i32, i32)>,
    /// middle-drag: where it started and where the grid was
    panning: Option<(i32, i32, i32, i32)>,
    flash_until: Option<Instant>,
    /// the keyboard stroke was started with Enter: it lasts until the next Enter
    sticky: bool,
    /// when Space was last pressed, with no other key since
    last_space: Option<Instant>,

    pub modal: Option<Modal>,
    modal_at: Instant,
    pub menu: Option<Menu>,
    pub toast: Option<(String, Instant)>,
    pub hits: Vec<(Rect, Act)>,
}

/// Whether a Nerd Font is installed, as far as fontconfig knows: terminals fall back to it
/// for the icons even when it isn't the font they are set to.
fn has_nerd_font() -> bool {
    std::process::Command::new("fc-list")
        .args([":", "family"])
        .stdin(std::process::Stdio::null())
        .output()
        .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).contains("Nerd Font"))
}

fn settings_path(store: &Store) -> PathBuf {
    store.dir().join("tui.json")
}

fn card_of(p: &Puzzle, solved: bool) -> Card {
    Card {
        id: p.id,
        w: p.w,
        h: p.h,
        color: p.color,
        rating: p.rating,
        difficulty: p.difficulty,
        fans: p.fans,
        title: p.title.clone(),
        title2: p.title2.clone(),
        author: p.author.clone(),
        solved,
        progress: None,
    }
}

fn load_puzzle(store: &Store, id: u32) -> Result<Box<Loaded>, String> {
    let board = store.board(id)?;
    let meta = store
        .catalog
        .read()
        .unwrap()
        .by_id
        .get(&id)
        .cloned()
        .ok_or("Unknown puzzle")?;
    let solved = store.account.lock().unwrap().solved.contains(&id);
    Ok(Box::new(Loaded {
        board,
        card: card_of(&meta, solved),
        progress: store.get_progress(id),
    }))
}

fn load_image(store: &Store, (id, thumb): ImgKey) -> Result<Img, String> {
    let png = if thumb {
        store.progress_thumb(id)
    } else {
        store.puzzle_png(id)
    }?;
    let (w, h, px) = protocol::decode_png(&png)?;
    // pictures are shown on white, like the board of a puzzle that doesn't bring its own background
    let px = px
        .iter()
        .map(|p| blend([255; 3], [p[0], p[1], p[2]], p[3] as f32 / 255.0))
        .collect();
    Ok(Img {
        w: w as usize,
        h: h as usize,
        px,
    })
}

pub fn fmt_time(s: u32) -> String {
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

impl App {
    pub fn new(store: Arc<Store>, tx: Sender<Msg>, size: (u16, u16), key_release: bool) -> App {
        let settings: Settings = std::fs::read(settings_path(&store))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let now = Instant::now();
        let mut app = App {
            theme: Theme::new(settings.dark.unwrap_or_else(terminal_is_dark)),
            search: Input::new(&settings.search),
            author: Input::new(&settings.author),
            cross: match settings.nerd_font.unwrap_or_else(has_nerd_font) {
                true => '\u{f467}', // nf-oct-x
                false => '×',
            },
            store,
            tx,
            settings,
            size,
            status: Status::default(),
            screen: Screen::Browse,
            quit: false,
            dirty: true,
            ticks: 0,
            next_second: now,
            key_release,
            mode: Mode::Browse,
            focus: None,
            items: Vec::new(),
            total: 0,
            page: 0,
            loading: false,
            load_token: 0,
            waiting_for_catalog: false,
            sel: 0,
            scroll: 0,
            list_h: 1,
            strip: Vec::new(),
            previews: HashMap::new(),
            filter_at: None,
            sel_at: now,
            select_at: [(0, 0); 5],
            login: Login::default(),
            game: None,
            meta: None,
            view: View::default(),
            open_token: 0,
            hover: None,
            panning: None,
            flash_until: None,
            sticky: false,
            last_space: None,
            modal: None,
            modal_at: now,
            menu: None,
            toast: None,
            hits: Vec::new(),
        };
        for (sel, s) in app.settings.selects.iter_mut().zip(&SELECTS) {
            *sel = (*sel).min(s.options.len() - 1);
        }
        app.settings.zoom = app.settings.zoom.min(3);
        app.status = serde_json::from_value(app.store.status()).unwrap_or_default();
        app.show_browse(Mode::Browse);
        app
    }

    /// Run network or disk work on another thread; its result comes back as a message.
    fn spawn(&self, f: impl FnOnce(&Arc<Store>) -> Msg + Send + 'static) {
        let (store, tx) = (self.store.clone(), self.tx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(f(&store));
        });
    }

    fn save_settings(&self) {
        if let Ok(data) = serde_json::to_vec_pretty(&self.settings) {
            let _ = std::fs::write(settings_path(&self.store), data);
        }
    }

    pub fn toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now() + Duration::from_millis(2600)));
    }

    fn refresh_status(&mut self) {
        let before = (self.status.catalog_count, self.status.catalog_time);
        self.status = serde_json::from_value(self.store.status()).unwrap_or_default();
        // the puzzle list arrived or was refreshed
        if (self.status.catalog_count, self.status.catalog_time) != before
            && self.screen == Screen::Browse
        {
            self.load_cards(true);
            self.load_strip();
        }
    }

    // ------------------------------------------------------------ time

    /// Called every 100 ms or so.
    pub fn tick(&mut self, now: Instant) {
        self.ticks += 1;
        if self.loading || (self.screen == Screen::Play && self.game.is_none()) || self.login.busy {
            self.dirty = true; // spinners
        }
        if self
            .previews
            .values()
            .any(|p| matches!(p, Preview::Loading))
        {
            self.dirty = true;
        }
        if self.toast.as_ref().is_some_and(|t| now >= t.1) {
            self.toast = None;
            self.dirty = true;
        }
        if self.flash_until.is_some_and(|t| now >= t) {
            self.flash_until = None;
            if let Some(g) = &mut self.game {
                g.flash = None;
            }
            self.dirty = true;
        }
        if self.filter_at.is_some_and(|t| now >= t) {
            self.filter_at = None;
            self.settings.search = self.search.text.clone();
            self.settings.author = self.author.text.clone();
            self.save_settings();
            self.load_cards(true);
        }
        if self.screen == Screen::Browse
            && now >= self.sel_at + Duration::from_millis(150)
            && let Some(key) = self.items.get(self.sel).and_then(Card::image)
        {
            self.want_image(key);
        }
        if now >= self.next_second {
            self.next_second = now + Duration::from_secs(1);
            self.refresh_status();
            self.dirty = true;
            if self.screen == Screen::Play
                && let Some(g) = self.game.as_mut().filter(|g| !g.solved)
            {
                g.time += 1;
                if g.time % 5 == 0 {
                    g.dirty = true;
                    self.after_game();
                }
            }
        }
    }

    // ------------------------------------------------------------ messages

    pub fn on_msg(&mut self, msg: Msg) {
        self.dirty = true;
        match msg {
            Msg::Term(ev) => self.on_event(ev),
            Msg::Cards {
                token,
                reset,
                total,
                items,
            } => {
                if token != self.load_token {
                    return;
                }
                self.loading = false;
                self.total = total;
                if reset {
                    // stay on the puzzle that was selected, when it is still on the first page
                    let id = self.items.get(self.sel).map(|c| c.id);
                    self.sel = items.iter().position(|c| Some(c.id) == id).unwrap_or(0);
                    self.scroll = 0;
                    self.waiting_for_catalog = items.is_empty() && self.status.catalog_count == 0;
                    self.items = items;
                } else {
                    self.items.extend(items);
                }
                self.sel_at = Instant::now();
            }
            Msg::Strip(items) => self.strip = items,
            Msg::Puzzle { token, res } => {
                if token != self.open_token || self.screen != Screen::Play {
                    return;
                }
                match res {
                    Ok(l) => self.start_game(*l),
                    Err(e) => {
                        self.toast(format!("Could not load puzzle: {e}"));
                        self.show_browse(self.mode);
                    }
                }
            }
            Msg::Image { key, res } => {
                self.previews
                    .insert(key, res.map_or(Preview::Failed, Preview::Ready));
            }
            Msg::LoggedIn(res) => {
                self.login.busy = false;
                match res {
                    Ok(()) => {
                        self.login.password = Input::default();
                        self.refresh_status();
                        let s = &self.status;
                        let who = s
                            .nickname
                            .clone()
                            .filter(|n| !n.is_empty())
                            .or(s.email.clone());
                        self.toast(match &s.sync_error {
                            Some(e) => format!("Logged in, but sync failed: {e}"),
                            None => format!("Logged in as {}", who.unwrap_or_default()),
                        });
                        self.show_browse(Mode::Browse);
                    }
                    Err(e) => self.login.error = Some(e),
                }
            }
            Msg::Synced(res) => {
                self.toast(match res {
                    Ok(0) => "Synced".to_string(),
                    Ok(n) => format!("Synced, uploaded {n} solved"),
                    Err(e) => format!("Sync failed: {e}"),
                });
                self.refresh_status();
                if self.screen == Screen::Browse {
                    self.load_cards(true);
                }
            }
            Msg::LoggedOut(Ok(())) => {
                self.refresh_status();
                self.toast("Logged out");
                self.show_browse(Mode::Browse);
            }
            Msg::LoggedOut(Err(e)) => {
                self.show(Modal {
                    title: "Log out?".into(),
                    text: format!("{e}. They will be lost if you log out now. Log out anyway?"),
                    img: ModalImg::None,
                    buttons: vec![
                        ("Cancel", BtnStyle::Ghost, Then::Close),
                        ("Log out", BtnStyle::Danger, Then::ForceLogout),
                    ],
                    sel: 0,
                });
            }
        }
    }

    fn on_event(&mut self, ev: Event) {
        match ev {
            Event::Key(k) => self.on_key(k),
            Event::Mouse(m) => self.on_mouse(m),
            Event::Paste(s) => {
                let input = match (self.screen, self.focus) {
                    (Screen::Login, _) => self.login.input(),
                    (Screen::Browse, Some(Field::Search)) => Some(&mut self.search),
                    (Screen::Browse, Some(Field::Author)) => Some(&mut self.author),
                    _ => None,
                };
                if let Some(input) = input {
                    input.paste(&s);
                    if self.screen == Screen::Browse {
                        self.filter_at = Some(Instant::now() + Duration::from_millis(250));
                    }
                }
            }
            Event::Resize(w, h) => {
                self.size = (w, h);
                self.layout_stage();
            }
            Event::FocusLost => {
                // nobody is holding Space any more; and the terminal may be about to close
                self.end_space();
            }
            _ => {}
        }
    }

    // ------------------------------------------------------------ browse

    fn show_browse(&mut self, mode: Mode) {
        self.leave_game();
        self.screen = Screen::Browse;
        self.modal = None;
        self.menu = None;
        self.focus = None;
        self.mode = mode;
        self.load_cards(true);
        self.load_strip();
    }

    fn query(&self) -> Value {
        if self.mode == Mode::Continue {
            return json!({ "status": "started", "page": self.page, "per": PER });
        }
        let value = |i: usize| SELECTS[i].options[self.settings.selects[i]].0;
        let (min, max) = value(0).split_once('-').unwrap_or(("0", "999"));
        json!({
            "q": self.search.text, "author": self.author.text,
            "min": min, "max": max, "color": value(1), "status": value(2), "sort": value(3), "rating": value(4),
            "page": self.page, "per": PER,
        })
    }

    fn load_cards(&mut self, reset: bool) {
        if reset {
            self.page = 0;
            // boards in progress have changed since their pictures were made
            self.previews
                .retain(|key, p| !key.1 && matches!(p, Preview::Ready(_)));
        }
        self.load_token += 1;
        self.loading = true;
        let (token, query) = (self.load_token, self.query());
        self.spawn(move |s| {
            let mut res = s.search(&query);
            Msg::Cards {
                token,
                reset,
                total: res["total"].as_u64().unwrap_or(0) as usize,
                items: serde_json::from_value(res["items"].take()).unwrap_or_default(),
            }
        });
    }

    fn load_strip(&mut self) {
        self.spawn(|s| {
            let mut res = s.search(&json!({ "status": "started", "per": 12 }));
            Msg::Strip(serde_json::from_value(res["items"].take()).unwrap_or_default())
        });
    }

    fn want_image(&mut self, key: ImgKey) {
        if self.previews.contains_key(&key) {
            return;
        }
        self.previews.insert(key, Preview::Loading);
        self.spawn(move |s| Msg::Image {
            key,
            res: load_image(s, key),
        });
    }

    fn select(&mut self, sel: usize) {
        let sel = sel.min(self.items.len().saturating_sub(1));
        if sel != self.sel {
            self.sel = sel;
            self.sel_at = Instant::now();
        }
        // endless list: fetch the next page before the selection gets to the end
        if self.sel + 20 >= self.items.len() && self.items.len() < self.total && !self.loading {
            self.page += 1;
            self.load_cards(false);
        }
    }

    fn open(&mut self, card: Card) {
        if card.solved && card.progress.is_none() {
            // the finished picture, with the option to play it again from scratch
            self.want_image((card.id, false));
            self.show(Modal {
                title: card.name().to_string(),
                text: format!(
                    "#{} · {}×{} · by {} · ✓ solved",
                    card.id, card.w, card.h, card.author
                ),
                img: ModalImg::Key((card.id, false)),
                buttons: vec![
                    ("Close", BtnStyle::Ghost, Then::Close),
                    ("Solve again", BtnStyle::Primary, Then::SolveAgain(card.id)),
                ],
                sel: 1,
            });
        } else {
            self.open_puzzle(card.id);
        }
    }

    fn ask_delete(&mut self) {
        let Some(card) = self.items.get(self.sel) else {
            return;
        };
        let Some(pct) = card.progress else { return };
        self.show(Modal {
            title: "Delete progress?".into(),
            text: format!(
                "Your progress on “{}” ({pct}%) will be deleted. This can't be undone.",
                card.name()
            ),
            img: ModalImg::None,
            buttons: vec![
                ("Cancel", BtnStyle::Ghost, Then::Close),
                ("Delete", BtnStyle::Danger, Then::DeleteProgress(card.id)),
            ],
            sel: 1,
        });
    }

    fn delete_progress(&mut self, id: u32) {
        if let Err(e) = self.store.delete_progress(id) {
            self.toast(format!("Could not delete: {e}"));
            return;
        }
        self.toast("Progress deleted");
        // update in place so the list keeps its position and loaded pages
        self.strip.retain(|c| c.id != id);
        self.previews.remove(&(id, true));
        if self.mode == Mode::Continue {
            let before = self.items.len();
            self.items.retain(|c| c.id != id);
            self.total -= (before - self.items.len()).min(self.total);
            self.select(self.sel);
        } else {
            self.items
                .iter_mut()
                .filter(|c| c.id == id)
                .for_each(|c| c.progress = None);
        }
        self.sel_at = Instant::now();
    }

    fn filter_by_author(&mut self, row: usize) {
        let Some(card) = self.items.get(row) else {
            return;
        };
        self.author = Input::new(&card.author);
        self.settings.author = card.author.clone();
        self.settings.selects[STATUS] = 1; // all
        self.save_settings();
        self.focus = None;
        self.show_browse(Mode::Browse);
    }

    fn open_select(&mut self, i: usize) {
        let items = SELECTS[i].options.iter().enumerate();
        self.menu = Some(Menu {
            at: self.select_at[i],
            right: false,
            head: Vec::new(),
            items: items
                .map(|(k, o)| (o.1.to_string(), Pick::Option(i, k)))
                .collect(),
            sel: self.settings.selects[i],
        });
    }

    fn open_user_menu(&mut self) {
        let s = &self.status;
        let mut head = vec![match s.logged_in {
            true => s.email.clone().unwrap_or_default(),
            false => "Progress is kept on this device only".to_string(),
        }];
        head.extend(
            s.sync_error
                .iter()
                .filter(|_| s.logged_in)
                .map(|e| format!("Sync error: {e}")),
        );
        let theme = if self.theme.dark {
            "Light theme"
        } else {
            "Dark theme"
        };
        let mut items = vec![
            ("↓ Refresh puzzle list".to_string(), Pick::Refresh),
            (theme.to_string(), Pick::Theme),
        ];
        if s.logged_in {
            items.insert(0, ("⟳ Sync now".into(), Pick::Sync));
            items.push(("Log out".into(), Pick::Logout));
        } else {
            items.insert(0, ("Log in or create account".into(), Pick::Login));
        }
        items.push(("Quit".into(), Pick::Quit));
        self.menu = Some(Menu {
            at: (self.size.0.saturating_sub(1), 1),
            right: true,
            head,
            items,
            sel: 0,
        });
    }

    fn pick(&mut self, pick: Pick) {
        self.menu = None;
        match pick {
            Pick::Login => {
                self.login.error = None;
                self.login.focus = 0;
                self.focus = None;
                self.screen = Screen::Login;
            }
            Pick::Sync => self.spawn(|s| Msg::Synced(s.sync())),
            Pick::Refresh => {
                let store = self.store.clone();
                std::thread::spawn(move || store.load_catalog(true));
                self.toast("Refreshing the puzzle list…");
            }
            Pick::Theme => {
                self.settings.dark = Some(!self.theme.dark);
                self.theme = Theme::new(!self.theme.dark);
                self.save_settings();
            }
            Pick::Logout => self.spawn(|s| Msg::LoggedOut(s.logout(false).map(|_| ()))),
            Pick::Quit => self.quit(),
            Pick::Option(i, k) => {
                self.settings.selects[i] = k;
                self.save_settings();
                self.load_cards(true);
            }
        }
    }

    pub fn quit(&mut self) {
        self.leave_game();
        self.quit = true;
    }

    // ------------------------------------------------------------ dialogs

    fn show(&mut self, modal: Modal) {
        self.modal = Some(modal);
        self.modal_at = Instant::now();
    }

    fn close_modal(&mut self, then: Then) {
        self.modal = None;
        match then {
            Then::Close => {}
            Then::DeleteProgress(id) => self.delete_progress(id),
            Then::SolveAgain(id) => {
                let _ = self.store.delete_progress(id); // nothing to clear is fine
                self.open_puzzle(id);
            }
            Then::BackToList => self.show_browse(self.mode),
            Then::ForceLogout => self.spawn(|s| Msg::LoggedOut(s.logout(true).map(|_| ()))),
            Then::Reset => {
                if let Some(g) = &mut self.game {
                    g.reset();
                }
                self.after_game();
            }
        }
    }

    // ------------------------------------------------------------ login

    fn submit_login(&mut self) {
        if self.login.busy {
            return;
        }
        self.login.busy = true;
        self.login.error = None;
        let l = &self.login;
        let (register, nick) = (l.register, l.nickname.text.clone());
        let (email, password) = (l.email.text.clone(), l.password.text.clone());
        self.spawn(move |s| {
            let res = if register {
                s.register(&email, &password, &nick)
            } else {
                s.login(&email, &password)
            };
            Msg::LoggedIn(res.map(|_| ()))
        });
    }

    fn login_act(&mut self, item: LoginItem) {
        match item {
            LoginItem::Submit => self.submit_login(),
            LoginItem::Switch => {
                self.login.register = !self.login.register;
                self.login.error = None;
                self.login.focus = 0;
            }
            LoginItem::Guest => self.show_browse(Mode::Browse),
            LoginItem::Password => self.submit_login(),
            _ => self.login.focus += 1,
        }
    }

    fn login_key(&mut self, k: KeyEvent) {
        let n = self.login.items().len();
        match k.code {
            KeyCode::Esc => self.show_browse(Mode::Browse),
            KeyCode::Tab | KeyCode::Down => self.login.focus = (self.login.focus + 1) % n,
            KeyCode::BackTab | KeyCode::Up => self.login.focus = (self.login.focus + n - 1) % n,
            KeyCode::Enter => self.login_act(self.login.focused()),
            KeyCode::Char(' ') if !self.login.on_input() => self.login_act(self.login.focused()),
            _ => {
                if let Some(input) = self.login.input() {
                    input.key(&k);
                }
            }
        }
    }

    // ------------------------------------------------------------ play

    fn open_puzzle(&mut self, id: u32) {
        self.leave_game();
        self.modal = None;
        self.menu = None;
        self.screen = Screen::Play;
        self.open_token += 1;
        let token = self.open_token;
        self.spawn(move |s| Msg::Puzzle {
            token,
            res: load_puzzle(s, id),
        });
    }

    pub fn opts(&self) -> Opts {
        Opts {
            auto_cross: self.settings.auto_cross,
            auto_gaps: self.settings.auto_gaps,
        }
    }

    fn start_game(&mut self, l: Loaded) {
        let game = Game::load(&l.board, l.card.color, &l.progress, self.opts());
        self.view = View::new(self.settings.zoom, self.settings.auto_fit);
        self.game = Some(game);
        self.meta = Some(l.card);
        self.hover = None;
        self.sticky = false;
        self.next_second = Instant::now() + Duration::from_secs(1);
        self.layout_stage();
    }

    /// (sidebar, stage)
    pub fn play_areas(&self) -> (Rect, Rect) {
        let (w, h) = self.size;
        let side = if self.settings.side_collapsed {
            RAIL_W
        } else {
            SIDE_W
        }
        .min(w / 2);
        (Rect::new(0, 0, side, h), Rect::new(side, 0, w - side, h))
    }

    fn layout_stage(&mut self) {
        let stage = self.play_areas().1;
        if let Some(g) = &self.game {
            self.view.resize(g, stage.width, stage.height);
        }
    }

    /// After anything that may have changed the board: save it (moves are discrete and the
    /// terminal may close at any moment), and celebrate if that was the last move.
    fn after_game(&mut self) {
        let Some(g) = &mut self.game else { return };
        if g.dirty {
            g.dirty = false;
            if let Err(e) = self.store.save_progress(g.id, &g.snapshot()) {
                self.toast(format!("Save failed: {e}"));
            }
        }
        if self
            .game
            .as_mut()
            .is_some_and(|g| std::mem::take(&mut g.just_solved))
        {
            self.on_solved();
        }
    }

    fn leave_game(&mut self) {
        self.end_space();
        if let Some(g) = &mut self.game {
            g.end_stroke();
        }
        self.after_game();
        self.game = None;
        self.meta = None;
        self.panning = None;
        self.flash_until = None;
    }

    fn on_solved(&mut self) {
        let (Some(g), Some(m)) = (&self.game, &self.meta) else {
            return;
        };
        let helps = match g.helps {
            0 => "no help".to_string(),
            1 => "1 help".to_string(),
            n => format!("{n} helps"),
        };
        let mut text = format!("{} · {} · {helps}", m.name(), fmt_time(g.time));
        let img = Img {
            w: g.w,
            h: g.h,
            px: g.sol.iter().map(|&v| g.pal[v as usize]).collect(),
        };
        let logged_in = self.store.session.lock().unwrap().is_some();
        match self.store.mark_solved(g.id, g.time as i32) {
            Ok(_) if !logged_in => text += " · saved on this device, log in to sync it",
            Ok(true) => text += " · syncing to your account…",
            Ok(false) => text += " · already on your account",
            Err(e) => self.toast(format!("Could not record solve: {e}")),
        }
        self.show(Modal {
            title: "Solved!".into(),
            text,
            img: ModalImg::Own(img),
            buttons: vec![
                ("Look at it", BtnStyle::Ghost, Then::Close),
                ("Back to list", BtnStyle::Primary, Then::BackToList),
            ],
            sel: 1,
        });
    }

    fn end_space(&mut self) {
        self.sticky = false;
        if let Some(g) = &mut self.game
            && g.space.is_some()
        {
            g.end_space();
            self.after_game();
        }
    }

    /// Space went down. `repeat`: it is being held.
    fn space_down(&mut self, repeat: bool) {
        let burst = self
            .last_space
            .replace(Instant::now())
            .is_some_and(|t| t.elapsed() < Duration::from_millis(60));
        let Some(g) = self.game.as_mut().filter(|g| !g.solved) else {
            return;
        };
        if g.space.is_some() {
            if self.sticky {
                self.end_space();
            }
            return;
        }
        // without release events a held Space only shows as a burst of presses
        if repeat || (!self.key_release && burst) {
            return;
        }
        g.kb = true;
        g.apply_space(true, false);
        if !self.key_release {
            g.end_space();
        }
        self.after_game();
    }

    /// Enter starts a keyboard stroke that lasts until the next Enter: holding Space, for
    /// terminals that can't tell when a key is released.
    fn toggle_stroke(&mut self) {
        let Some(g) = self.game.as_mut().filter(|g| !g.solved) else {
            return;
        };
        if g.space.is_some() {
            self.end_space();
        } else {
            g.kb = true;
            g.apply_space(true, false);
            self.sticky = true;
            self.after_game();
        }
    }

    fn game_act(&mut self, act: Act) {
        let Some(g) = &mut self.game else { return };
        match act {
            Act::Tool(t) => g.select_tool(t),
            Act::Undo => g.history(true),
            Act::Redo => g.history(false),
            Act::Zoom(dir) => self.view.zoom_by(g, dir, None),
            Act::Fit => self.view.set_auto_fit(g, true),
            Act::AutoClues => self.settings.auto_clues ^= true,
            Act::AutoCross => self.settings.auto_cross ^= true,
            Act::AutoGaps => self.settings.auto_gaps ^= true,
            Act::Check => {
                let msg = match g.check() {
                    0 => "No mistakes so far".to_string(),
                    1 => "1 mistake".to_string(),
                    n => format!("{n} mistakes"),
                };
                self.flash_until = Some(Instant::now() + Duration::from_millis(1500));
                self.toast(msg);
            }
            Act::Reset => {
                self.show(Modal {
                    title: "Clear the whole board?".into(),
                    text: String::new(),
                    img: ModalImg::None,
                    buttons: vec![
                        ("Cancel", BtnStyle::Ghost, Then::Close),
                        ("Reset", BtnStyle::Danger, Then::Reset),
                    ],
                    sel: 1,
                });
            }
            Act::Help => {
                if let Some((msg, cell)) = g.help() {
                    self.view
                        .reveal(g, ((cell % g.w) as i32, (cell / g.w) as i32));
                    self.toast(msg);
                }
            }
            _ => {}
        }
        match act {
            Act::Zoom(_) | Act::Fit => self.remember_view(),
            Act::AutoClues | Act::AutoCross | Act::AutoGaps => {
                self.save_settings();
                let opts = self.opts();
                if let Some(g) = &mut self.game {
                    g.opts = opts;
                    g.update_done(None);
                }
            }
            _ => {}
        }
        self.after_game();
    }

    /// Zoom and auto-fit are kept between puzzles and runs.
    fn remember_view(&mut self) {
        if (self.settings.auto_fit, self.settings.zoom) != (self.view.auto_fit, self.view.zoom) {
            (self.settings.auto_fit, self.settings.zoom) = (self.view.auto_fit, self.view.zoom);
            self.save_settings();
        }
    }

    fn toggle_sidebar(&mut self) {
        self.settings.side_collapsed ^= true;
        self.save_settings();
        self.layout_stage();
    }

    fn move_cursor(&mut self, dx: i32, dy: i32) {
        if let Some(g) = &mut self.game {
            g.move_cursor(dx, dy);
            self.view.reveal(g, g.cursor);
        }
        self.after_game();
    }

    // ------------------------------------------------------------ keys

    fn on_key(&mut self, k: KeyEvent) {
        if k.kind == KeyEventKind::Release {
            if k.code == KeyCode::Char(' ') && !self.sticky {
                self.end_space();
            }
            return;
        }
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        if ctrl && k.code == KeyCode::Char('c') {
            return self.quit();
        }
        if let Some(m) = &mut self.modal {
            let n = m.buttons.len();
            match k.code {
                KeyCode::Left | KeyCode::BackTab | KeyCode::Char('h') => {
                    m.sel = (m.sel + n - 1) % n
                }
                KeyCode::Right | KeyCode::Tab | KeyCode::Char('l') => m.sel = (m.sel + 1) % n,
                // not for a key still held from the move that opened the dialog
                KeyCode::Enter | KeyCode::Char(' ') => {
                    let then = m.buttons[m.sel].2;
                    if k.kind == KeyEventKind::Press
                        && self.modal_at.elapsed() > Duration::from_millis(400)
                    {
                        self.close_modal(then);
                    }
                }
                KeyCode::Esc => self.close_modal(Then::Close),
                _ => {}
            }
            return;
        }
        if let Some(m) = &mut self.menu {
            let n = m.items.len();
            match k.code {
                KeyCode::Up | KeyCode::Char('k') => m.sel = (m.sel + n - 1) % n,
                KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => m.sel = (m.sel + 1) % n,
                KeyCode::Enter | KeyCode::Char(' ') => {
                    let pick = m.items[m.sel].1;
                    self.pick(pick);
                }
                _ => self.menu = None,
            }
            return;
        }
        match self.screen {
            Screen::Browse => self.browse_key(k),
            Screen::Login => self.login_key(k),
            Screen::Play => self.play_key(k),
        }
    }

    fn browse_key(&mut self, k: KeyEvent) {
        if let Some(field) = self.focus {
            let input = if field == Field::Search {
                &mut self.search
            } else {
                &mut self.author
            };
            match k.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Down => self.focus = None,
                KeyCode::Tab | KeyCode::BackTab => {
                    self.focus = Some(if field == Field::Search {
                        Field::Author
                    } else {
                        Field::Search
                    })
                }
                _ => {
                    if input.key(&k) {
                        self.filter_at = Some(Instant::now() + Duration::from_millis(250));
                    }
                }
            }
            return;
        }
        let page = self.list_h.max(2) - 1;
        let browsing = self.mode == Mode::Browse;
        match k.code {
            KeyCode::Char('q') => self.quit(),
            KeyCode::Down | KeyCode::Char('j') => self.select(self.sel + 1),
            KeyCode::Up | KeyCode::Char('k') => self.select(self.sel.saturating_sub(1)),
            KeyCode::PageDown | KeyCode::Char(' ') => self.select(self.sel + page),
            KeyCode::PageUp => self.select(self.sel.saturating_sub(page)),
            KeyCode::Home | KeyCode::Char('g') => self.select(0),
            KeyCode::End | KeyCode::Char('G') => self.select(usize::MAX),
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => {
                if let Some(card) = self.items.get(self.sel).cloned() {
                    self.open(card);
                }
            }
            KeyCode::Char('d') | KeyCode::Delete => self.ask_delete(),
            KeyCode::Char('A') => self.filter_by_author(self.sel),
            KeyCode::Tab | KeyCode::BackTab => self.show_browse(if browsing {
                Mode::Continue
            } else {
                Mode::Browse
            }),
            KeyCode::Char('1') => self.show_browse(Mode::Browse),
            KeyCode::Char('2') => self.show_browse(Mode::Continue),
            KeyCode::Char('m') => self.open_user_menu(),
            KeyCode::Char('/') if browsing => self.focus = Some(Field::Search),
            KeyCode::Char('a') if browsing => self.focus = Some(Field::Author),
            KeyCode::Char(c) if browsing => {
                if let Some(i) = SELECTS.iter().position(|s| s.key == c) {
                    self.open_select(i);
                }
            }
            _ => {}
        }
    }

    fn play_key(&mut self, k: KeyEvent) {
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        if k.code != KeyCode::Char(' ') {
            self.last_space = None;
        }
        let code = match k.code {
            KeyCode::Char(c) if shift && c.is_ascii_lowercase() => {
                KeyCode::Char(c.to_ascii_uppercase())
            }
            code => code,
        };
        if k.modifiers.contains(KeyModifiers::CONTROL) {
            match code {
                KeyCode::Char('z') => self.game_act(Act::Undo),
                KeyCode::Char('Z' | 'y' | 'r') => self.game_act(Act::Redo),
                _ => {}
            }
            return;
        }
        if k.modifiers.contains(KeyModifiers::ALT) {
            return;
        }
        let far = if shift { 5 } else { 1 };
        match code {
            KeyCode::Esc => {
                if self.game.as_ref().is_some_and(|g| g.space.is_some()) {
                    self.end_space();
                } else {
                    self.show_browse(self.mode);
                }
            }
            KeyCode::Char('q') => self.show_browse(self.mode),
            KeyCode::Char(' ') => self.space_down(k.kind == KeyEventKind::Repeat),
            KeyCode::Enter if k.kind == KeyEventKind::Press => self.toggle_stroke(),
            KeyCode::Left | KeyCode::Char('h') => self.move_cursor(-far, 0),
            KeyCode::Right | KeyCode::Char('l') => self.move_cursor(far, 0),
            KeyCode::Up | KeyCode::Char('k') => self.move_cursor(0, -far),
            KeyCode::Down | KeyCode::Char('j') => self.move_cursor(0, far),
            KeyCode::Char('H') => self.move_cursor(-5, 0),
            KeyCode::Char('L') => self.move_cursor(5, 0),
            KeyCode::Char('K') => self.move_cursor(0, -5),
            KeyCode::Char('J') => self.move_cursor(0, 5),
            KeyCode::Char(c @ '0'..='9') => {
                let n = if c == '0' {
                    10
                } else {
                    c as usize - '0' as usize
                };
                let n = n + if shift { 10 } else { 0 };
                // 0 is also Fit, for puzzles with fewer than ten colours
                let colors = self.game.as_ref().map_or(0, |g| g.pal.len() - 1);
                self.game_act(if n == 10 && colors < 10 {
                    Act::Fit
                } else {
                    Act::Tool(Some(n))
                });
            }
            KeyCode::Char(c) if "!@#$%^&*()".contains(c) => {
                self.game_act(Act::Tool("!@#$%^&*()".find(c).map(|i| i + 11)));
            }
            KeyCode::Char('x') => self.game_act(Act::Tool(None)),
            KeyCode::Char('+' | '=') => self.game_act(Act::Zoom(1)),
            KeyCode::Char('-') => self.game_act(Act::Zoom(-1)),
            KeyCode::Char('f') => self.game_act(Act::Fit),
            KeyCode::Char('u') => self.game_act(Act::Undo),
            KeyCode::Char('U') => self.game_act(Act::Redo),
            KeyCode::Char('c') => self.game_act(Act::Check),
            KeyCode::Char('R') => self.game_act(Act::Reset),
            KeyCode::Char('?') => self.game_act(Act::Help),
            KeyCode::Char('n') => self.game_act(Act::AutoClues),
            KeyCode::Char('e') => self.game_act(Act::AutoCross),
            KeyCode::Char('g') => self.game_act(Act::AutoGaps),
            KeyCode::Char('[') => self.toggle_sidebar(),
            _ => {}
        }
    }

    // ------------------------------------------------------------ mouse

    fn hit(&self, x: u16, y: u16) -> Option<(Rect, Act)> {
        self.hits
            .iter()
            .rev()
            .find(|(r, _)| r.contains(Position::new(x, y)))
            .copied()
    }

    fn on_mouse(&mut self, m: MouseEvent) {
        let hit = self.hit(m.column, m.row);
        let over_stage = matches!(hit, Some((_, Act::Stage)));
        let stage = self.play_areas().1;
        let (sx, sy) = (
            m.column as i32 - stage.x as i32,
            m.row as i32 - stage.y as i32,
        );
        match m.kind {
            MouseEventKind::Down(button) if over_stage => self.stage_down(button, sx, sy),
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some((_, act)) = hit {
                    self.act(act);
                }
            }
            MouseEventKind::Drag(_) => self.stage_drag(sx, sy),
            MouseEventKind::Up(_) => {
                self.panning = None;
                if let Some(g) = &mut self.game {
                    g.end_stroke();
                }
                self.after_game();
            }
            MouseEventKind::Moved => {
                let Some(g) = &mut self.game else { return };
                let cell = self
                    .view
                    .hit(g, sx, sy)
                    .filter(|h| over_stage && h.kind == Kind::Cell);
                self.hover = cell.map(|h| (h.x, h.y));
                if over_stage {
                    g.kb = false;
                }
            }
            MouseEventKind::ScrollUp
            | MouseEventKind::ScrollDown
            | MouseEventKind::ScrollLeft
            | MouseEventKind::ScrollRight => {
                let dir = if matches!(
                    m.kind,
                    MouseEventKind::ScrollUp | MouseEventKind::ScrollLeft
                ) {
                    -1
                } else {
                    1
                };
                let sideways = matches!(
                    m.kind,
                    MouseEventKind::ScrollLeft | MouseEventKind::ScrollRight
                ) || m.modifiers.contains(KeyModifiers::SHIFT);
                if self.modal.is_some() || self.menu.is_some() {
                } else if let (true, Some(g)) = (over_stage, &self.game) {
                    if m.modifiers.contains(KeyModifiers::CONTROL) {
                        self.view.zoom_by(g, -dir, Some((sx, sy)));
                    } else if sideways {
                        self.view.scroll(g, dir * 6, 0);
                    } else {
                        self.view.scroll(g, 0, dir * 3);
                    }
                    self.remember_view();
                } else if self.screen == Screen::Browse {
                    self.select(if dir < 0 {
                        self.sel.saturating_sub(3)
                    } else {
                        self.sel + 3
                    });
                }
            }
            _ => {}
        }
    }

    fn stage_down(&mut self, button: MouseButton, x: i32, y: i32) {
        self.end_space();
        let Some(g) = &mut self.game else { return };
        if button == MouseButton::Middle {
            let (gx, gy) = self.view.origin();
            self.panning = Some((x, y, gx, gy));
            return;
        }
        if g.solved {
            return;
        }
        let Some(hit) = self.view.hit(g, x, y) else {
            return;
        };
        g.kb = false;
        g.cursor = (hit.x, hit.y);
        match hit.kind {
            Kind::Cell => {
                self.hover = Some((hit.x, hit.y));
                g.start_stroke(
                    hit.x as usize,
                    hit.y as usize,
                    button == MouseButton::Right || g.cross_tool,
                );
            }
            Kind::Row => _ = g.toggle_mark(Kind::Row, hit.y as usize, hit.k, None),
            Kind::Col => _ = g.toggle_mark(Kind::Col, hit.x as usize, hit.k, None),
        }
        self.after_game();
    }

    fn stage_drag(&mut self, x: i32, y: i32) {
        let Some(g) = &mut self.game else { return };
        if let Some((px, py, gx, gy)) = self.panning {
            self.view.pan_to(g, gx + x - px, gy + y - py);
            self.remember_view();
        } else if g.drag.is_some() {
            let (cx, cy) = self.view.cell_at(g, x, y);
            g.move_stroke(cx, cy);
            self.hover = Some((cx as i32, cy as i32));
        }
    }

    fn act(&mut self, act: Act) {
        match act {
            Act::Mode(mode) => self.show_browse(mode),
            Act::UserMenu => match self.menu {
                Some(_) => self.menu = None,
                None => self.open_user_menu(),
            },
            Act::Select(i) => self.open_select(i),
            Act::Focus(field) => self.focus = Some(field),
            Act::Row(i) => {
                self.focus = None;
                if i == self.sel {
                    if let Some(card) = self.items.get(i).cloned() {
                        self.open(card);
                    }
                } else {
                    self.select(i);
                }
            }
            Act::Author(i) => self.filter_by_author(i),
            Act::Open(id) => self.open_puzzle(id),
            Act::MenuItem(i) => {
                if let Some(pick) = self
                    .menu
                    .as_ref()
                    .and_then(|m| m.items.get(i))
                    .map(|it| it.1)
                {
                    self.pick(pick);
                }
            }
            Act::CloseMenu => self.menu = None,
            Act::ModalBtn(i) => {
                if let Some(then) = self
                    .modal
                    .as_ref()
                    .and_then(|m| m.buttons.get(i))
                    .map(|b| b.2)
                {
                    self.close_modal(then);
                }
            }
            Act::CloseModal => self.close_modal(Then::Close),
            Act::LoginItem(i) => {
                self.login.focus = i;
                if !self.login.on_input() {
                    self.login_act(self.login.focused());
                }
            }
            Act::Back => self.show_browse(self.mode),
            Act::Collapse => self.toggle_sidebar(),
            Act::Stage | Act::Nothing => {}
            _ => self.game_act(act),
        }
    }
}
