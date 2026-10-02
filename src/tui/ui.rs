//! Drawing. Everything clickable is recorded in `hits` as it is drawn.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::{
    Act, App, BtnStyle, Card, Field, Img, Input, LoginItem, ModalImg, Mode, Preview, SELECTS,
    Screen, fmt_time,
};
use crate::board;
use crate::game::{Rgb, luminance};
use crate::theme::{BLACK, Theme, WHITE, blend};

const BOLD: Modifier = Modifier::BOLD;
const SPINNER: [char; 10] = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];

struct Ui<'a> {
    buf: &'a mut Buffer,
    th: Theme,
    area: Rect,
    hits: Vec<(Rect, Act)>,
    /// where the text cursor goes, when a text field has the focus
    cursor: Option<Position>,
    spinner: char,
}

fn width(s: &str) -> u16 {
    s.width() as u16
}

/// Cut to `w` columns, with an ellipsis when something was cut.
fn ellipsis(s: &str, w: u16) -> String {
    if width(s) <= w {
        return s.to_string();
    }
    let (mut out, mut used) = (String::new(), 1);
    for c in s.chars() {
        used += c.width().unwrap_or(0) as u16;
        if used > w {
            break;
        }
        out.push(c);
    }
    out + "…"
}

fn wrap(s: &str, w: u16) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    for word in s.split_whitespace() {
        match lines.last_mut() {
            Some(line) if width(line) + 1 + width(word) <= w => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(ellipsis(word, w)),
        }
    }
    lines
}

/// 1234567 → "1,234,567"
fn group(n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    let mut out = String::from(if n < 0 { "-" } else { "" });
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn stars(v: f32) -> String {
    if v > 0.0 {
        format!("{v:.1}")
    } else {
        "–".to_string()
    }
}

impl Ui<'_> {
    fn st(&self, fg: Rgb, bg: Rgb) -> Style {
        Style::new().fg(self.th.c(fg)).bg(self.th.c(bg))
    }

    fn fill(&mut self, r: Rect, bg: Rgb) {
        let style = self.st(self.th.ink, bg);
        for pos in r.intersection(self.area).positions() {
            let cell = &mut self.buf[pos];
            cell.reset();
            cell.set_style(style);
        }
    }

    /// Returns the width drawn.
    fn text(&mut self, x: u16, y: u16, s: &str, style: Style, max: u16) -> u16 {
        if y >= self.area.bottom() || x >= self.area.right() {
            return 0;
        }
        let max = max.min(self.area.right() - x);
        self.buf.set_stringn(x, y, s, max as usize, style).0 - x
    }

    fn centered(&mut self, r: Rect, y: u16, s: &str, style: Style) {
        let s = ellipsis(s, r.width);
        self.text(
            r.x + r.width.saturating_sub(width(&s)) / 2,
            y,
            &s,
            style,
            r.width,
        );
    }

    fn hit(&mut self, r: Rect, act: Act) {
        self.hits.push((r, act));
    }

    fn button(&mut self, x: u16, y: u16, label: &str, style: Style, act: Act) -> u16 {
        let w = self.text(x, y, &format!(" {label} "), style, u16::MAX);
        self.hit(Rect::new(x, y, w, 1), act);
        w
    }

    /// A filled box with a thin border.
    fn panel(&mut self, r: Rect, bg: Rgb) {
        self.fill(r, bg);
        let style = self.st(self.th.muted, bg);
        let (x1, y1) = (r.right() - 1, r.bottom() - 1);
        for x in r.x + 1..x1 {
            self.text(x, r.y, "─", style, 1);
            self.text(x, y1, "─", style, 1);
        }
        for y in r.y + 1..y1 {
            self.text(r.x, y, "│", style, 1);
            self.text(x1, y, "│", style, 1);
        }
        for (x, y, c) in [
            (r.x, r.y, "╭"),
            (x1, r.y, "╮"),
            (r.x, y1, "╰"),
            (x1, y1, "╯"),
        ] {
            self.text(x, y, c, style, 1);
        }
    }

    fn input(&mut self, r: Rect, input: &Input, focused: bool, placeholder: &str, mask: bool) {
        let th = self.th;
        let bg = if focused {
            blend(th.card, th.accent, 0.18)
        } else {
            th.card
        };
        self.fill(r, bg);
        let chars: Vec<char> = if mask {
            vec!['•'; input.text.chars().count()]
        } else {
            input.text.chars().collect()
        };
        if chars.is_empty() && !focused {
            self.text(
                r.x + 1,
                r.y,
                placeholder,
                self.st(th.muted, bg),
                r.width.saturating_sub(2),
            );
            return;
        }
        // scrolled sideways so that the cursor is on screen
        let inner = r.width.saturating_sub(2);
        let w = |cs: &[char]| {
            cs.iter()
                .map(|c| c.width().unwrap_or(0) as u16)
                .sum::<u16>()
        };
        let pos = input.pos.min(chars.len());
        let mut start = 0;
        while start < pos && w(&chars[start..pos]) >= inner {
            start += 1;
        }
        let shown: String = chars[start..].iter().collect();
        self.text(r.x + 1, r.y, &shown, self.st(th.ink, bg), inner);
        if focused {
            self.cursor = Some(Position::new(r.x + 1 + w(&chars[start..pos]), r.y));
        }
    }

    /// The picture in half blocks: one column and half a row per pixel, scaled to fit `r`.
    fn image(&mut self, r: Rect, img: &Img, bg: Rgb) {
        let (cols, rows, s) = image_size(img.w, img.h, r.width, r.height);
        let px_h = ((img.h as f32 * s).round() as u16).max(1);
        let x0 = r.x + r.width.saturating_sub(cols) / 2;
        let at = |c: u16, py: u16| {
            let sx = (((c as f32 + 0.5) / s) as usize).min(img.w - 1);
            let sy = (((py as f32 + 0.5) / s) as usize).min(img.h - 1);
            img.px[sy * img.w + sx]
        };
        for row in 0..rows {
            for c in 0..cols {
                let top = at(c, row * 2);
                let bottom = if row * 2 + 1 < px_h {
                    at(c, row * 2 + 1)
                } else {
                    bg
                };
                self.text(x0 + c, r.y + row, "▀", self.st(top, bottom), 1);
            }
        }
    }
}

/// (columns, rows, scale) of a w×h picture in at most max_w columns and max_rows rows:
/// whole pixels when it fits, shrunk otherwise.
fn image_size(w: usize, h: usize, max_w: u16, max_rows: u16) -> (u16, u16, f32) {
    let s = (max_w as f32 / w.max(1) as f32).min(max_rows as f32 * 2.0 / h.max(1) as f32);
    let s = if s >= 1.0 { s.floor() } else { s };
    let cols = ((w as f32 * s).round() as u16).clamp(1, max_w.max(1));
    let px_h = ((h as f32 * s).round() as u16).max(1);
    (cols, px_h.div_ceil(2).min(max_rows.max(1)), s)
}

pub fn render(app: &mut App, f: &mut Frame) {
    let area = f.area();
    let mut ui = Ui {
        buf: f.buffer_mut(),
        th: app.theme,
        area,
        hits: Vec::new(),
        cursor: None,
        spinner: SPINNER[app.ticks % SPINNER.len()],
    };
    let th = ui.th;
    ui.fill(area, th.bg);
    if area.width < 40 || area.height < 12 {
        ui.text(
            0,
            0,
            "The window is too small",
            ui.st(th.ink, th.bg),
            area.width,
        );
    } else {
        match app.screen {
            Screen::Browse => browse(&mut ui, app),
            Screen::Login => login(&mut ui, app),
            Screen::Play => play(&mut ui, app),
        }
        menu(&mut ui, app);
        modal(&mut ui, app);
        if let Some((msg, _)) = &app.toast {
            let msg = ellipsis(msg, area.width - 6);
            let x = (area.width - width(&msg) - 2) / 2;
            ui.text(
                x,
                area.height - 2,
                &format!(" {msg} "),
                ui.st(th.bg, th.ink),
                area.width,
            );
        }
    }
    app.hits = std::mem::take(&mut ui.hits);
    if let Some(pos) = ui.cursor {
        f.set_cursor_position(pos);
    }
}

// ------------------------------------------------------------------ browse

/// The account's numbers, for the top bar.
fn account(app: &App, th: &Theme) -> Vec<(String, Rgb)> {
    let s = &app.status;
    let mut out = vec![(format!("{} solved", group(s.solved_count as i64)), th.muted)];
    if let Some(score) = s.score {
        out.push((format!("{} pts", group(score)), th.muted));
    }
    if !s.logged_in {
        out.push(("not synced".into(), th.muted));
    } else {
        if s.pending > 0 {
            out.push((format!("{} to sync", s.pending), th.muted));
        }
        if s.sync_error.is_some() {
            out.push(("sync error".into(), th.error));
        }
    }
    out.push(match &s.catalog_error {
        _ if s.catalog_loading && s.catalog_count == 0 => {
            ("downloading puzzle list…".into(), th.muted)
        }
        Some(e) => (e.clone(), th.error),
        None => (
            format!("{} puzzles", group(s.catalog_count as i64)),
            th.muted,
        ),
    });
    out
}

fn browse(ui: &mut Ui, app: &mut App) {
    let th = ui.th;
    let (w, h) = (ui.area.width, ui.area.height);

    // top bar
    ui.fill(Rect::new(0, 0, w, 1), th.panel);
    let mut x =
        1 + ui.text(
            1,
            0,
            "Katana",
            ui.st(th.accent, th.panel).add_modifier(BOLD),
            w,
        ) + 2;
    for (mode, label) in [(Mode::Browse, "Browse"), (Mode::Continue, "In progress")] {
        let on = app.mode == mode;
        let style = if on {
            ui.st(th.accent_ink, th.accent).add_modifier(BOLD)
        } else {
            ui.st(th.ink, th.panel)
        };
        x += ui.button(x, 0, label, style, Act::Mode(mode)) + 1;
    }
    let who = match (app.status.logged_in, app.status.nickname.as_deref()) {
        (false, _) => "Guest",
        (true, Some(n)) if !n.is_empty() => n,
        _ => "Account",
    };
    let who = format!("{} ▾", ellipsis(who, 20));
    let user_x = w.saturating_sub(width(&who) + 3);
    ui.button(user_x, 0, &who, ui.st(th.ink, th.card), Act::UserMenu);
    let mut parts = account(app, &th);
    // drop the least important numbers when the bar is narrow
    while parts.len() > 1 && x + parts.iter().map(|p| width(&p.0) + 3).sum::<u16>() > user_x {
        parts.remove(0);
    }
    let mut ax = user_x
        .saturating_sub(parts.iter().map(|p| width(&p.0) + 3).sum::<u16>())
        .max(x);
    for (i, (text, color)) in parts.iter().enumerate() {
        if i > 0 {
            ax += ui.text(
                ax,
                0,
                " · ",
                ui.st(th.muted, th.panel),
                user_x.saturating_sub(ax),
            );
        }
        ax += ui.text(
            ax,
            0,
            text,
            ui.st(*color, th.panel),
            user_x.saturating_sub(ax + 1),
        );
    }

    let mut y = 2;
    if app.mode == Mode::Browse {
        // boards in progress, most recent first
        if !app.strip.is_empty() {
            let all = "All in progress →";
            let end = w.saturating_sub(width(all) + 4);
            let mut x = 1 + ui.text(1, y, "Continue playing ", ui.st(th.muted, th.bg), w);
            for card in &app.strip {
                let label = format!(
                    "{} {}%",
                    ellipsis(card.name(), 18),
                    card.progress.unwrap_or(0)
                );
                if x + width(&label) + 3 > end {
                    break;
                }
                x += ui.button(x, y, &label, ui.st(th.ink, th.card), Act::Open(card.id)) + 1;
            }
            ui.button(
                end + 1,
                y,
                all,
                ui.st(th.accent, th.bg),
                Act::Mode(Mode::Continue),
            );
            y += 2;
        }
        // filters
        let key = ui.st(th.accent, th.bg).add_modifier(BOLD);
        let mut x = 1;
        for (field, hint, input, placeholder, fw) in [
            (
                Field::Search,
                "/",
                &app.search,
                "Search title or #id…",
                (w / 3).clamp(18, 44),
            ),
            (
                Field::Author,
                "a",
                &app.author,
                "Author (exact)",
                (w / 5).clamp(14, 26),
            ),
        ] {
            x += ui.text(x, y, hint, key, 2) + 1;
            let r = Rect::new(x, y, fw, 1);
            ui.input(r, input, app.focus == Some(field), placeholder, false);
            ui.hit(r, Act::Focus(field));
            x += fw + 2;
        }
        let mut count = format!("{} puzzles", group(app.total as i64));
        if app.loading {
            count = format!("{} {count}", ui.spinner);
        }
        ui.text(
            w.saturating_sub(width(&count) + 1).max(x),
            y,
            &count,
            ui.st(th.muted, th.bg),
            w,
        );
        y += 1;
        let mut x = 1;
        for (i, select) in SELECTS.iter().enumerate() {
            let label = format!("{} ▾", select.options[app.settings.selects[i]].1);
            if x + width(&label) + 4 > w {
                break;
            }
            x += ui.text(x, y, &select.key.to_string(), key, 2) + 1;
            app.select_at[i] = (x, y + 1);
            x += ui.button(x, y, &label, ui.st(th.ink, th.card), Act::Select(i)) + 2;
        }
        y += 2;
    } else {
        ui.text(
            1,
            y,
            "In progress",
            ui.st(th.ink, th.bg).add_modifier(BOLD),
            w,
        );
        let count = format!("{} puzzles", group(app.total as i64));
        ui.text(
            w.saturating_sub(width(&count) + 1),
            y,
            &count,
            ui.st(th.muted, th.bg),
            w,
        );
        y += 2;
    }

    // the list, and the selected puzzle's details beside it when there is room
    let bottom = h - 1;
    let side = if w >= 96 { (w / 3).clamp(36, 64) } else { 0 };
    list(ui, app, Rect::new(0, y, w - side, bottom - y));
    if side > 0 {
        details(ui, app, Rect::new(w - side, y, side, bottom - y));
    }
    let keys = match (app.focus, app.mode) {
        (Some(_), _) => "type to filter · Tab other field · ⏎ done",
        (_, Mode::Browse) => {
            "↑↓ select · ⏎ open · / search · a author · s c t o r filters · A by this author · d delete progress · Tab in progress · m menu · q quit"
        }
        (_, Mode::Continue) => {
            "↑↓ select · ⏎ open · d delete progress · Tab browse · m menu · q quit"
        }
    };
    ui.text(1, bottom, keys, ui.st(th.muted, th.bg), w - 1);
}

fn list(ui: &mut Ui, app: &mut App, r: Rect) {
    let th = ui.th;
    if app.items.is_empty() {
        let msg = match (&app.status.catalog_error, app.mode) {
            (Some(e), _) if app.status.catalog_count == 0 => e.as_str(),
            _ if app.waiting_for_catalog => "Downloading the puzzle list (≈5 MB)…",
            _ if app.loading => "Loading…",
            (_, Mode::Continue) => "Nothing in progress yet.",
            _ => "No puzzles match.",
        };
        ui.centered(r, r.y + r.height / 3, msg, ui.st(th.muted, th.bg));
        return;
    }
    // columns: mark, title, size, kind, author, ratings, id
    let wide = r.width >= 84;
    let author_w = (r.width / 5).clamp(10, 24);
    let fixed = 5 + 8 + 6 + author_w + 1 + if wide { 15 } else { 5 } + 9;
    let title_w = r.width.saturating_sub(fixed + 1).max(8);
    let cols = [
        1,
        6,
        6 + title_w,
        14 + title_w,
        20 + title_w,
        21 + title_w + author_w,
    ];
    let head = ui.st(th.muted, th.bg);
    ui.text(cols[1], r.y, "Title", head, title_w);
    ui.text(cols[2], r.y, "Size", head, 8);
    ui.text(cols[4], r.y, "Author", head, author_w);
    ui.text(
        cols[5],
        r.y,
        if wide { "Rate Diff Fans" } else { "Rate" },
        head,
        15,
    );
    ui.text(r.right().saturating_sub(3), r.y, "#", head, 2);

    let rows = r.height.saturating_sub(1) as usize;
    app.list_h = rows.max(1);
    app.sel = app.sel.min(app.items.len() - 1);
    app.scroll = app
        .scroll
        .clamp((app.sel + 1).saturating_sub(app.list_h), app.sel);
    for (row, (i, card)) in app
        .items
        .iter()
        .enumerate()
        .skip(app.scroll)
        .take(rows)
        .enumerate()
    {
        let y = r.y + 1 + row as u16;
        let selected = i == app.sel;
        let bg = if selected {
            blend(th.bg, th.accent, 0.3)
        } else {
            blend(th.bg, th.ink, (i % 2) as f32 * 0.03)
        };
        let line = Rect::new(r.x, y, r.width, 1);
        ui.fill(line, bg);
        ui.hit(line, Act::Row(i));
        let ink = ui
            .st(th.ink, bg)
            .add_modifier(if selected { BOLD } else { Modifier::empty() });
        let muted = ui.st(if selected { th.ink } else { th.muted }, bg);
        if card.solved {
            ui.text(cols[0], y, "✓", ui.st(th.ok, bg).add_modifier(BOLD), 1);
        }
        if let Some(pct) = card.progress {
            ui.text(
                cols[0] + 1,
                y,
                &format!("{pct}%"),
                ui.st(if selected { th.ink } else { th.accent }, bg),
                4,
            );
        }
        ui.text(
            cols[1],
            y,
            &ellipsis(card.name(), title_w - 1),
            ink,
            title_w,
        );
        ui.text(cols[2], y, &format!("{}×{}", card.w, card.h), muted, 8);
        if card.color {
            ui.text(cols[3], y, "color", muted, 6);
        }
        let author = ellipsis(&card.author, author_w);
        ui.text(cols[4], y, &author, muted, author_w);
        ui.hit(Rect::new(cols[4], y, width(&author), 1), Act::Author(i));
        let ratings = [card.rating, card.difficulty, card.fans];
        for (k, v) in ratings.iter().take(if wide { 3 } else { 1 }).enumerate() {
            ui.text(cols[5] + k as u16 * 5, y, &stars(*v), muted, 4);
        }
        let id = format!("#{}", card.id);
        ui.text(r.right().saturating_sub(width(&id) + 1), y, &id, muted, 9);
    }
}

/// A box the shape of the puzzle, for the ones with no picture to show yet.
fn placeholder(ui: &mut Ui, r: Rect, card: &Card, bg: Rgb) {
    let th = ui.th;
    let (cols, rows, _) = image_size(
        card.w as usize,
        card.h as usize,
        r.width.min(32),
        r.height.min(12),
    );
    let (cols, rows) = (cols.max(7).min(r.width), rows.max(1));
    let fill = blend(bg, th.ink, 0.1);
    let rect = Rect::new(r.x + (r.width - cols) / 2, r.y, cols, rows);
    ui.fill(rect, fill);
    ui.centered(
        rect,
        r.y + rows / 2,
        &format!("{}×{}", card.w, card.h),
        ui.st(th.muted, fill),
    );
}

fn details(ui: &mut Ui, app: &App, r: Rect) {
    let th = ui.th;
    ui.fill(r, th.panel);
    let Some(card) = app.items.get(app.sel) else {
        return;
    };
    let (x, w) = (r.x + 2, r.width.saturating_sub(4));
    let mut y = r.y + 1;
    for line in wrap(card.name(), w).iter().take(2) {
        ui.text(x, y, line, ui.st(th.ink, th.panel).add_modifier(BOLD), w);
        y += 1;
    }
    let muted = ui.st(th.muted, th.panel);
    if !card.title2.is_empty() && card.title2 != card.title {
        ui.text(x, y, &ellipsis(&card.title2, w), muted, w);
        y += 1;
    }
    let kind = if card.color { " · color" } else { "" };
    ui.text(
        x,
        y,
        &ellipsis(
            &format!(
                "#{} · {}×{}{kind} · by {}",
                card.id, card.w, card.h, card.author
            ),
            w,
        ),
        muted,
        w,
    );
    let (rating, difficulty, fans) = (stars(card.rating), stars(card.difficulty), stars(card.fans));
    ui.text(
        x,
        y + 1,
        &format!("★ {rating} rating · {difficulty} difficulty · ♥ {fans}"),
        muted,
        w,
    );
    y += 2;
    let mut sx = x;
    if card.solved {
        sx += ui.text(
            sx,
            y,
            "✓ solved",
            ui.st(th.ok, th.panel).add_modifier(BOLD),
            w,
        ) + 2;
    }
    if let Some(pct) = card.progress {
        ui.text(
            sx,
            y,
            &format!("{pct}% done"),
            ui.st(th.accent, th.panel).add_modifier(BOLD),
            w,
        );
    }
    y += 2;
    let pic = Rect::new(x, y, w, r.bottom().saturating_sub(y + 1));
    if pic.height == 0 {
        return;
    }
    match card.image().map(|key| app.previews.get(&key)) {
        Some(Some(Preview::Ready(img))) => ui.image(pic, img, th.panel),
        Some(Some(Preview::Failed)) | None => placeholder(ui, pic, card, th.panel),
        Some(_) => ui.centered(pic, pic.y + 1, &ui.spinner.to_string(), muted),
    }
}

// ------------------------------------------------------------------ login

fn login(ui: &mut Ui, app: &App) {
    let th = ui.th;
    let l = &app.login;
    let items = l.items();
    let w = 52.min(ui.area.width - 2);
    let sub = if l.register {
        "Create a Nonograms Katana account to sync your progress"
    } else {
        "Log in with your Nonograms Katana account to sync your progress"
    };
    let sub = wrap(sub, w - 6);
    let error = l
        .error
        .as_deref()
        .map(|e| wrap(e, w - 6))
        .unwrap_or_default();
    let h = 4 + sub.len() as u16 + items.len() as u16 * 2 + error.len() as u16;
    let r = Rect::new(
        (ui.area.width - w) / 2,
        ui.area.height.saturating_sub(h) / 2,
        w,
        h.min(ui.area.height),
    );
    ui.panel(r, th.panel);
    let inner = Rect::new(r.x + 3, r.y, w - 6, h);
    let mut y = r.y + 2;
    ui.centered(
        inner,
        y,
        "Katana Desktop",
        ui.st(th.ink, th.panel).add_modifier(BOLD),
    );
    for line in &sub {
        y += 1;
        ui.centered(inner, y, line, ui.st(th.muted, th.panel));
    }
    y += 2;
    for (i, item) in items.iter().enumerate() {
        let focused = i == l.focus;
        let line = Rect::new(inner.x, y, inner.width, 1);
        ui.hit(line, Act::LoginItem(i));
        let field = match item {
            LoginItem::Nickname => Some(("Nickname", &l.nickname, false)),
            LoginItem::Email => Some(("Email", &l.email, false)),
            LoginItem::Password => Some(("Password", &l.password, true)),
            _ => None,
        };
        if let Some((label, input, mask)) = field {
            ui.text(inner.x, y, label, ui.st(th.muted, th.panel), 9);
            ui.input(
                Rect::new(inner.x + 10, y, inner.width - 10, 1),
                input,
                focused,
                "",
                mask,
            );
            y += 2;
            continue;
        }
        if *item == LoginItem::Submit {
            for (k, line) in error.iter().enumerate() {
                ui.text(
                    inner.x,
                    y + k as u16,
                    line,
                    ui.st(th.error, th.panel),
                    inner.width,
                );
            }
            y += error.len() as u16 + if error.is_empty() { 0 } else { 1 };
            let label = match (l.busy, l.register) {
                (true, _) => ui.spinner.to_string(),
                (_, true) => "Create account".to_string(),
                _ => "Log in".to_string(),
            };
            let label = if focused {
                format!("› {label} ‹")
            } else {
                label
            };
            let line = Rect::new(inner.x, y, inner.width, 1);
            ui.fill(line, th.accent);
            ui.hit(line, Act::LoginItem(i));
            ui.centered(
                line,
                y,
                &label,
                ui.st(th.accent_ink, th.accent).add_modifier(BOLD),
            );
            y += 2;
            continue;
        }
        let (lead, link) = match (item, l.register) {
            (LoginItem::Switch, false) => ("No account yet? ", "Create one"),
            (LoginItem::Switch, true) => ("Already have an account? ", "Log in"),
            _ => ("", "Continue without an account"),
        };
        let x = inner.x + (inner.width.saturating_sub(width(lead) + width(link))) / 2;
        let x = x + ui.text(x, y, lead, ui.st(th.muted, th.panel), inner.width);
        let style = ui.st(th.accent, th.panel).add_modifier(if focused {
            BOLD | Modifier::UNDERLINED
        } else {
            Modifier::empty()
        });
        ui.text(x, y, link, style, inner.width);
        y += 1;
    }
}

// ------------------------------------------------------------------ play

const CONTROLS: [(&str, &str); 13] = [
    ("Click, right click", "fill, cross"),
    ("Drag", "paint a line"),
    ("Click a number", "cross it out"),
    ("Arrows, h j k l", "move (Shift: 5)"),
    ("Space", "fill → cross → empty"),
    ("Enter", "start / end a line"),
    ("1–0, Shift+1–0", "colours 1–20"),
    ("x", "cross tool"),
    ("u, U", "undo, redo"),
    ("Wheel, Ctrl+wheel", "scroll, zoom"),
    ("Middle drag", "pan"),
    ("+  −  f", "zoom, fit"),
    ("[  Esc", "sidebar, back"),
];

fn play(ui: &mut Ui, app: &mut App) {
    let th = ui.th;
    let (side, stage) = app.play_areas();
    ui.fill(side, th.panel);
    if app.settings.side_collapsed {
        rail(ui, app, side)
    } else {
        sidebar(ui, app, side)
    }
    match &app.game {
        Some(g) => board::draw(
            ui.buf,
            stage,
            g,
            &app.view,
            &th,
            &board::Look {
                hover: app.hover,
                auto_clues: app.settings.auto_clues,
                cross: app.cross,
            },
        ),
        None => ui.centered(
            stage,
            stage.height / 2,
            &format!("{} Loading…", ui.spinner),
            ui.st(th.muted, th.bg),
        ),
    }
    ui.hit(stage, Act::Stage);
}

/// Colour buttons, three columns each, wrapped to the sidebar's width; then the cross tool.
fn palette(ui: &mut Ui, app: &App, x0: u16, mut y: u16, w: u16) -> u16 {
    let th = ui.th;
    let Some(g) = &app.game else { return y };
    let mut x = x0;
    for k in 1..g.pal.len() {
        if x + 3 > x0 + w {
            (x, y) = (x0, y + 1);
        }
        let color = g.pal[k];
        let key = match k {
            1..=10 => format!("{}", k % 10),
            11..=20 => format!("⇧{}", k % 10),
            _ => String::new(),
        };
        let selected = !g.cross_tool && g.tool as usize == k;
        let label = if selected {
            format!("[{key:^1}]")
        } else {
            format!(" {key:<2}")
        };
        let ink = if luminance(color) > 0.55 {
            BLACK
        } else {
            WHITE
        };
        ui.text(
            x,
            y,
            &label,
            ui.st(ink, color)
                .add_modifier(if selected { BOLD } else { Modifier::empty() }),
            3,
        );
        ui.hit(Rect::new(x, y, 3, 1), Act::Tool(Some(k)));
        x += 3;
    }
    if x + 3 > x0 + w {
        (x, y) = (x0, y + 1);
    }
    let style = if g.cross_tool {
        ui.st(th.accent_ink, th.accent).add_modifier(BOLD)
    } else {
        ui.st(th.ink, th.card)
    };
    let label = if g.cross_tool {
        format!("[{}]", app.cross)
    } else {
        format!(" {} ", app.cross)
    };
    ui.text(x, y, &label, style, 3);
    ui.hit(Rect::new(x, y, 3, 1), Act::Tool(None));
    y + 1
}

fn sidebar(ui: &mut Ui, app: &App, r: Rect) {
    let th = ui.th;
    let (x, w, bottom) = (r.x + 1, r.width.saturating_sub(2), r.bottom());
    let ghost = ui.st(th.ink, th.panel);
    let muted = ui.st(th.muted, th.panel);
    let btn = ui.st(th.ink, th.card);
    let on = ui.st(th.accent_ink, th.accent).add_modifier(BOLD);
    ui.button(x, 0, "← Back", ghost, Act::Back);
    ui.button(r.right().saturating_sub(4), 0, "«", ghost, Act::Collapse);
    let (Some(g), Some(m)) = (&app.game, &app.meta) else {
        ui.text(x, 2, "Loading…", muted, w);
        return;
    };
    let mut y = 2;
    for line in wrap(m.name(), w).iter().take(2) {
        ui.text(x, y, line, ghost.add_modifier(BOLD), w);
        y += 1;
    }
    let kind = if m.color { " · color" } else { "" };
    let before = if m.solved {
        " · ✓ solved before"
    } else {
        ""
    };
    for line in wrap(
        &format!(
            "#{} · {}×{}{kind} · by {}{before}",
            m.id, m.w, m.h, m.author
        ),
        w,
    )
    .iter()
    .take(3)
    {
        ui.text(x, y, line, muted, w);
        y += 1;
    }
    ui.text(
        x,
        y + 1,
        &fmt_time(g.time),
        ui.st(th.accent, th.panel).add_modifier(BOLD),
        w,
    );
    y += 3;

    ui.text(x, y, "COLOURS", muted, w);
    y = palette(ui, app, x, y + 1, w) + 1;

    ui.text(x, y, "BOARD", muted, w);
    y += 1;
    let off = ui.st(th.muted, th.card);
    let dim = |can: bool| if can { btn } else { off };
    let bx = x + ui.button(x, y, "↶ Undo", dim(g.can_undo()), Act::Undo) + 1;
    ui.button(bx, y, "↷ Redo", dim(g.can_redo()), Act::Redo);
    y += 2;
    let mut bx = x + ui.button(x, y, "−", btn, Act::Zoom(-1)) + 1;
    bx += ui.button(
        bx,
        y,
        "Fit",
        if app.view.auto_fit { on } else { btn },
        Act::Fit,
    ) + 1;
    ui.button(bx, y, "+", btn, Act::Zoom(1));
    y += 2;
    for (key, checked, label, act) in [
        (
            "n",
            app.settings.auto_clues,
            "Cross out solved numbers",
            Act::AutoClues,
        ),
        (
            "e",
            app.settings.auto_cross,
            "Cross empty cells of finished lines",
            Act::AutoCross,
        ),
        (
            "g",
            app.settings.auto_gaps,
            "Cross gaps around solved numbers",
            Act::AutoGaps,
        ),
    ] {
        let lines = wrap(label, w.saturating_sub(6));
        ui.hit(Rect::new(x, y, w, lines.len() as u16), act);
        ui.text(x, y, key, ui.st(th.accent, th.panel).add_modifier(BOLD), 1);
        let mark = if checked {
            ui.st(th.accent, th.panel).add_modifier(BOLD)
        } else {
            muted
        };
        ui.text(x + 2, y, if checked { "[x]" } else { "[ ]" }, mark, 3);
        for line in &lines {
            ui.text(x + 6, y, line, ghost, w);
            y += 1;
        }
    }
    y += 1;
    let bx = x + ui.button(x, y, "c Check", btn, Act::Check) + 1;
    ui.button(bx, y, "R Reset", btn, Act::Reset);
    y += 2;

    // as much of the key reference as fits above the Help button
    if y + 3 < bottom {
        ui.text(x, y, "CONTROLS", muted, w);
        y += 1;
    }
    for (keys, what) in CONTROLS {
        if y + 2 >= bottom {
            break;
        }
        let what = if keys == "Space" && app.key_release {
            "…; hold and move to repeat"
        } else {
            what
        };
        let kw = ui.text(x, y, keys, ghost, w);
        ui.text(
            x + kw + 1,
            y,
            &ellipsis(what, w.saturating_sub(kw + 1)),
            muted,
            w,
        );
        y += 1;
    }
    help_button(ui, g.helps, Rect::new(x, bottom - 1, w, 1), "? Help");
}

fn help_button(ui: &mut Ui, helps: u32, r: Rect, label: &str) {
    let th = ui.th;
    let bg = blend(th.card, [0xff, 0xc5, 0x3d], 0.3);
    ui.fill(r, bg);
    ui.hit(r, Act::Help);
    let style = ui.st(th.ink, bg).add_modifier(BOLD);
    let label = if helps > 0 && r.width > 12 {
        format!("{label} · {helps} used")
    } else {
        label.to_string()
    };
    ui.centered(r, r.y, &label, style);
}

/// The collapsed sidebar: just the tools.
fn rail(ui: &mut Ui, app: &App, r: Rect) {
    let th = ui.th;
    let ghost = ui.st(th.ink, th.panel);
    let (x, bottom) = (r.x + 1, r.bottom());
    ui.button(x, 0, "←", ghost, Act::Back);
    ui.button(x, 1, "»", ghost, Act::Collapse);
    let Some(g) = &app.game else { return };
    let mut y = palette(ui, app, x, 3, 3) + 1;
    let on = ui.st(th.accent_ink, th.accent).add_modifier(BOLD);
    for (label, style, act) in [
        ("↶", ghost, Act::Undo),
        ("↷", ghost, Act::Redo),
        ("⤢", if app.view.auto_fit { on } else { ghost }, Act::Fit),
    ] {
        if y + 2 < bottom {
            ui.button(x, y, label, style, act);
            y += 1;
        }
    }
    if y + 2 < bottom {
        let time = fmt_time(g.time);
        ui.text(
            r.x + (r.width.saturating_sub(width(&time))) / 2,
            y + 1,
            &time,
            ui.st(th.muted, th.panel),
            r.width,
        );
    }
    let label = if g.helps > 0 {
        format!("?{}", g.helps)
    } else {
        "?".to_string()
    };
    help_button(ui, 0, Rect::new(r.x, bottom - 1, r.width, 1), &label);
}

// ------------------------------------------------------------------ overlays

fn menu(ui: &mut Ui, app: &App) {
    let th = ui.th;
    let Some(m) = &app.menu else { return };
    ui.hit(ui.area, Act::CloseMenu);
    let head: Vec<String> = m.head.iter().flat_map(|l| wrap(l, 44)).collect();
    let inner = m
        .items
        .iter()
        .map(|i| width(&i.0))
        .chain(head.iter().map(|l| width(l)))
        .max()
        .unwrap_or(0)
        + 2;
    let (w, h) = (inner + 2, head.len() as u16 + m.items.len() as u16 + 2);
    let x = if m.right {
        m.at.0.saturating_sub(w)
    } else {
        m.at.0
    };
    let r = Rect::new(
        x.min(ui.area.width.saturating_sub(w)),
        m.at.1.min(ui.area.height.saturating_sub(h)),
        w,
        h,
    );
    ui.panel(r, th.card);
    ui.hit(r, Act::Nothing);
    let mut y = r.y + 1;
    for line in &head {
        ui.text(r.x + 2, y, line, ui.st(th.muted, th.card), inner);
        y += 1;
    }
    for (i, (label, _)) in m.items.iter().enumerate() {
        let line = Rect::new(r.x + 1, y, inner, 1);
        let style = if i == m.sel {
            ui.fill(line, th.accent);
            ui.st(th.accent_ink, th.accent).add_modifier(BOLD)
        } else {
            ui.st(th.ink, th.card)
        };
        ui.text(line.x + 1, y, label, style, inner);
        ui.hit(line, Act::MenuItem(i));
        y += 1;
    }
}

fn modal(ui: &mut Ui, app: &App) {
    let th = ui.th;
    let Some(m) = &app.modal else { return };
    let (sw, sh) = (ui.area.width, ui.area.height);
    ui.hit(ui.area, Act::CloseModal);
    let img = match &m.img {
        ModalImg::None => None,
        ModalImg::Own(img) => Some(Some(img)),
        ModalImg::Key(key) => Some(match app.previews.get(key) {
            Some(Preview::Ready(img)) => Some(img),
            _ => None,
        }),
    };
    // the picture gets most of the window; the card grows around it
    let (max_w, max_rows) = ((sw * 4 / 5).saturating_sub(6).max(8), (sh * 13 / 20).max(3));
    let (cols, rows) = match img {
        None => (0, 0),
        Some(None) => (1, 1),
        Some(Some(img)) => {
            let (cols, rows, _) = image_size(img.w, img.h, max_w, max_rows);
            (cols, rows)
        }
    };
    let buttons: u16 = m.buttons.iter().map(|b| width(b.0) + 5).sum();
    let text_w = (sw - 8).min(cols.max(72));
    let text = wrap(&m.text, text_w);
    let widest = text
        .iter()
        .map(|l| width(l))
        .max()
        .unwrap_or(0)
        .max(width(&m.title));
    let w = (widest.max(cols).max(buttons) + 6).min(sw);
    let gap = (rows > 0) as u16;
    let h = (rows + gap + text.len() as u16 + 6).min(sh);
    let r = Rect::new((sw - w) / 2, (sh - h) / 2, w, h);
    ui.panel(r, th.card);
    ui.hit(r, Act::Nothing);
    let inner = Rect::new(r.x + 3, r.y + 1, w - 6, h - 2);
    let mut y = inner.y;
    match img {
        Some(Some(img)) => ui.image(Rect::new(inner.x, y, inner.width, rows), img, th.card),
        Some(None) => ui.centered(inner, y, &ui.spinner.to_string(), ui.st(th.muted, th.card)),
        None => {}
    }
    y += rows + gap;
    ui.centered(
        inner,
        y,
        &m.title,
        ui.st(th.ink, th.card).add_modifier(BOLD),
    );
    for line in &text {
        y += 1;
        ui.centered(inner, y, line, ui.st(th.muted, th.card));
    }
    y += 2;
    let mut x = inner.x + inner.width.saturating_sub(buttons) / 2;
    for (i, (label, kind, _)) in m.buttons.iter().enumerate() {
        let style = match kind {
            BtnStyle::Ghost => ui.st(th.ink, blend(th.card, th.ink, 0.12)),
            BtnStyle::Primary => ui.st(th.accent_ink, th.accent),
            BtnStyle::Danger => ui.st(WHITE, [0xc6, 0x28, 0x28]),
        };
        let selected = i == m.sel;
        let label = if selected {
            format!("› {label} ‹")
        } else {
            format!("  {label}  ")
        };
        let bw = ui.text(
            x,
            y,
            &label,
            style.add_modifier(if selected { BOLD } else { Modifier::empty() }),
            sw,
        );
        ui.hit(Rect::new(x, y, bw, 1), Act::ModalBtn(i));
        x += bw + 1;
    }
}
