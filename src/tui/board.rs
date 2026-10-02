//! The board on screen: zoom, scrolling, the pinned clue bands, and drawing all of it.
//! Everything is in terminal cells relative to the stage (the area right of the sidebar).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

use crate::game::{CROSS, EMPTY, Game, HintKind, Kind, Run, luminance};
use crate::theme::{BLACK, Rgb, Theme, WHITE, blend};

/// Board cell sizes: a terminal cell is about twice as tall as it is wide.
const ZOOMS: [(i32, i32); 4] = [(2, 1), (4, 2), (6, 3), (8, 4)];
/// Empty column between the row clues and the grid.
const PAD: i32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hit {
    pub kind: Kind,
    /// the cursor position of what was hit: negative inside the clue bands
    pub x: i32,
    pub y: i32,
    /// the clue number's index, for the bands
    pub k: usize,
}

pub struct Bands {
    w: i32,
    h: i32,
    bx: i32,
    by: i32,
    right: i32,
    bottom: i32,
}

#[derive(Default)]
pub struct View {
    pub zoom: usize,
    pub auto_fit: bool,
    /// where the grid's top left corner is
    gx: i32,
    gy: i32,
    vw: i32,
    vh: i32,
    placed: bool,
}

impl View {
    pub fn new(zoom: usize, auto_fit: bool) -> View {
        View {
            zoom,
            auto_fit,
            ..View::default()
        }
    }
    fn cell(&self) -> (i32, i32) {
        ZOOMS[self.zoom.min(ZOOMS.len() - 1)]
    }
    /// Width of one row clue number.
    fn box_w(&self) -> i32 {
        self.cell().0.clamp(3, 4)
    }
    fn clue_w(&self, g: &Game) -> i32 {
        g.max_row as i32 * self.box_w() + PAD
    }
    fn clue_h(&self, g: &Game) -> i32 {
        g.max_col as i32
    }

    /// The largest zoom that shows the whole puzzle, or the smallest one.
    pub fn fit(&mut self, g: &Game) {
        self.zoom = 0;
        for z in (0..ZOOMS.len()).rev() {
            self.zoom = z;
            let (cw, ch) = self.cell();
            if self.clue_w(g) + g.w as i32 * cw <= self.vw
                && self.clue_h(g) + g.h as i32 * ch <= self.vh
            {
                break;
            }
        }
        self.placed = false;
        self.clamp(g);
    }

    pub fn resize(&mut self, g: &Game, vw: u16, vh: u16) {
        (self.vw, self.vh) = (vw as i32, vh as i32);
        if self.auto_fit {
            self.fit(g)
        } else {
            self.clamp(g)
        }
    }

    pub fn set_auto_fit(&mut self, g: &Game, on: bool) {
        self.auto_fit = on;
        if on {
            self.fit(g);
        }
    }

    /// Zoom in or out, keeping the board under (cx, cy) (or the middle of the stage) in place.
    pub fn zoom_by(&mut self, g: &Game, dir: i32, at: Option<(i32, i32)>) {
        self.auto_fit = false;
        let (cw, ch) = self.cell();
        self.zoom = (self.zoom as i32 + dir).clamp(0, ZOOMS.len() as i32 - 1) as usize;
        let (ncw, nch) = self.cell();
        let (cx, cy) = at.unwrap_or((self.vw / 2, self.vh / 2));
        self.gx = cx - (cx - self.gx) * ncw / cw;
        self.gy = cy - (cy - self.gy) * nch / ch;
        self.clamp(g);
    }

    pub fn scroll(&mut self, g: &Game, dx: i32, dy: i32) {
        self.gx -= dx;
        self.gy -= dy;
        self.clamp(g);
    }

    pub fn origin(&self) -> (i32, i32) {
        (self.gx, self.gy)
    }

    /// Middle-drag: put the grid where it was plus the pointer's movement.
    pub fn pan_to(&mut self, g: &Game, gx: i32, gy: i32) {
        self.auto_fit = false;
        (self.gx, self.gy) = (gx, gy);
        self.clamp(g);
    }

    fn clamp(&mut self, g: &Game) {
        let (cw, ch) = self.cell();
        let (bw, bh) = (self.clue_w(g), self.clue_h(g));
        let (gw, gh) = (g.w as i32 * cw, g.h as i32 * ch);
        let (gx, gy) = if self.placed {
            (self.gx, self.gy)
        } else {
            (bw, bh)
        };
        self.placed = true;
        self.gx = if bw + gw <= self.vw {
            (self.vw - bw - gw) / 2 + bw
        } else {
            bw.min(gx.max(self.vw - gw))
        };
        self.gy = if bh + gh <= self.vh {
            (self.vh - bh - gh) / 2 + bh
        } else {
            bh.min(gy.max(self.vh - gh))
        };
    }

    /// Clue bands follow the grid but stay pinned to the stage edge when scrolled.
    /// A pinned band is capped at 40% of the stage (showing the clues nearest the grid).
    fn bands(&self, g: &Game) -> Bands {
        let (w, h) = (self.clue_w(g), self.clue_h(g));
        let right = self.gx.max(w.min(self.vw * 2 / 5));
        let bottom = self.gy.max(h.min(self.vh * 2 / 5));
        Bands {
            w,
            h,
            bx: right - w,
            by: bottom - h,
            right,
            bottom,
        }
    }

    /// The board cell at a stage position, clamped to the board: for strokes that leave it.
    pub fn cell_at(&self, g: &Game, mx: i32, my: i32) -> (usize, usize) {
        let (cw, ch) = self.cell();
        let x = (mx - self.gx).div_euclid(cw).clamp(0, g.w as i32 - 1);
        let y = (my - self.gy).div_euclid(ch).clamp(0, g.h as i32 - 1);
        (x as usize, y as usize)
    }

    /// What is under the pointer.
    pub fn hit(&self, g: &Game, mx: i32, my: i32) -> Option<Hit> {
        let b = self.bands(g);
        let (cw, ch) = self.cell();
        let x = (mx - self.gx).div_euclid(cw);
        let y = (my - self.gy).div_euclid(ch);
        let (in_x, in_y) = (x >= 0 && x < g.w as i32, y >= 0 && y < g.h as i32);
        if mx >= b.right && my >= b.bottom {
            return (in_x && in_y).then_some(Hit {
                kind: Kind::Cell,
                x,
                y,
                k: 0,
            });
        }
        if my < b.bottom && mx >= b.right && in_x {
            let len = g.col_clues[x as usize].len() as i32;
            let k = len - (b.bottom - my);
            return (k >= 0 && k < len).then_some(Hit {
                kind: Kind::Col,
                x,
                y: k - len,
                k: k as usize,
            });
        }
        if mx < b.right && my >= b.bottom && in_y {
            let len = g.row_clues[y as usize].len() as i32;
            let k = len - (b.right - PAD - mx + self.box_w() - 1).div_euclid(self.box_w());
            return (k >= 0 && k < len).then_some(Hit {
                kind: Kind::Row,
                x: k - len,
                y,
                k: k as usize,
            });
        }
        None
    }

    /// Scroll so that the cursor (or the cell a Help touched) is on screen.
    pub fn reveal(&mut self, g: &Game, (x, y): (i32, i32)) {
        let b = self.bands(g);
        let (cw, ch) = self.cell();
        let (px, py) = (self.gx + x.max(0) * cw, self.gy + y.max(0) * ch);
        if x >= 0 {
            if px < b.right {
                self.gx += b.right - px + cw;
            } else if px + cw > self.vw {
                self.gx -= px + cw - self.vw + cw;
            }
        } else {
            self.gx = self.gx.max(b.w); // onto a row clue: show the whole band
        }
        if y >= 0 {
            if py < b.bottom {
                self.gy += b.bottom - py + ch;
            } else if py + ch > self.vh {
                self.gy -= py + ch - self.vh + ch;
            }
        } else {
            self.gy = self.gy.max(b.h);
        }
        self.clamp(g);
    }
}

/// Drawing clipped to a rectangle of the stage.
struct Canvas<'a> {
    buf: &'a mut Buffer,
    area: Rect,
    th: &'a Theme,
    /// x0, y0, x1, y1
    clip: (i32, i32, i32, i32),
}

impl Canvas<'_> {
    fn clip(&mut self, x0: i32, y0: i32, x1: i32, y1: i32) {
        let (w, h) = (self.area.width as i32, self.area.height as i32);
        self.clip = (x0.max(0), y0.max(0), x1.min(w), y1.min(h));
    }
    fn put(
        &mut self,
        x: i32,
        y: i32,
        sym: Option<char>,
        fg: Option<Rgb>,
        bg: Option<Rgb>,
        modifier: Modifier,
    ) {
        let (x0, y0, x1, y1) = self.clip;
        if x < x0 || y < y0 || x >= x1 || y >= y1 {
            return;
        }
        let cell = &mut self.buf[(self.area.x + x as u16, self.area.y + y as u16)];
        if let Some(sym) = sym {
            cell.set_char(sym);
            cell.modifier = modifier;
        }
        if let Some(fg) = fg {
            cell.fg = self.th.c(fg);
        }
        if let Some(bg) = bg {
            cell.bg = self.th.c(bg);
        }
    }
    fn fill(&mut self, x: i32, y: i32, w: i32, h: i32, bg: Rgb) {
        for yy in y.max(self.clip.1)..(y + h).min(self.clip.3) {
            for xx in x.max(self.clip.0)..(x + w).min(self.clip.2) {
                self.put(xx, yy, Some(' '), None, Some(bg), Modifier::empty());
            }
        }
    }
    fn text(&mut self, x: i32, y: i32, s: &str, fg: Rgb, modifier: Modifier) {
        for (i, ch) in s.chars().enumerate() {
            self.put(x + i as i32, y, Some(ch), Some(fg), None, modifier);
        }
    }
    /// Mark a box without hiding what is in it: brackets on one-row cells, corners on taller ones.
    fn outline(&mut self, x: i32, y: i32, w: i32, h: i32, color: Rgb) {
        let m = Modifier::BOLD;
        if h == 1 {
            self.put(x, y, Some('['), Some(color), None, m);
            self.put(x + w - 1, y, Some(']'), Some(color), None, m);
        } else {
            self.put(x, y, Some('┌'), Some(color), None, m);
            self.put(x + w - 1, y, Some('┐'), Some(color), None, m);
            self.put(x, y + h - 1, Some('└'), Some(color), None, m);
            self.put(x + w - 1, y + h - 1, Some('┘'), Some(color), None, m);
        }
    }
}

fn contrast(c: Rgb) -> Rgb {
    if luminance(c) > 0.5 { BLACK } else { WHITE }
}

fn hint_color(kind: HintKind) -> Rgb {
    match kind {
        HintKind::Mistake => [0xe5, 0x48, 0x4d],
        HintKind::Deduce => [0x30, 0xa4, 0x6c],
        HintKind::Reveal => [0x00, 0x90, 0xff],
    }
}

/// What the board shows besides the game itself.
pub struct Look {
    /// the cursor position under the mouse pointer
    pub hover: Option<(i32, i32)>,
    /// strike through solved numbers
    pub auto_clues: bool,
    /// the character of a crossed cell
    pub cross: char,
}

pub fn draw(buf: &mut Buffer, area: Rect, g: &Game, v: &View, th: &Theme, look: &Look) {
    let Look {
        hover,
        auto_clues,
        cross,
    } = *look;
    let (vw, vh) = (area.width as i32, area.height as i32);
    let mut c = Canvas {
        buf,
        area,
        th,
        clip: (0, 0, vw, vh),
    };
    c.fill(0, 0, vw, vh, th.bg);

    let (cw, ch) = v.cell();
    let (w, h) = (g.w as i32, g.h as i32);
    let (gx, gy) = (v.gx, v.gy);
    let (x0, x1) = (
        (-gx).div_euclid(cw).max(0),
        (vw - gx + cw - 1).div_euclid(cw).min(w),
    );
    let (y0, y1) = (
        (-gy).div_euclid(ch).max(0),
        (vh - gy + ch - 1).div_euclid(ch).min(h),
    );
    // the board uses the puzzle's own background colour (white unless the author picked another);
    // shading, crosses and outlines adapt to it rather than to the app theme
    let board = g.pal[0];
    let live = !g.solved;
    let focus = if g.kb { Some(g.cursor) } else { hover }.filter(|_| live);
    let (fx, fy) = focus.unwrap_or((-99, -99));
    let hint = g.hint.as_ref().filter(|_| live);
    let hint_line = hint.and_then(|h| Some((h.line?, hint_color(h.kind))));

    // cells: there is no room for grid lines, so unknown cells and their 5x5 blocks are told
    // apart by shade; filled and crossed cells keep their exact colour
    for y in y0..y1 {
        for x in x0..x1 {
            let (ux, uy) = (x as usize, y as usize);
            let val = g.view(ux, uy);
            let mut bg = if val > 0 { g.pal[val as usize] } else { board };
            if live {
                if val <= 0 && (x == fx || y == fy) {
                    bg = if luminance(board) > 0.5 {
                        blend(bg, th.accent, 0.16)
                    } else {
                        blend(bg, WHITE, 0.16)
                    };
                }
                if val == EMPTY {
                    let shade = ((x / 5 + y / 5) % 2) as f32 * 0.08 + ((x + y) % 2) as f32 * 0.035;
                    bg = blend(bg, contrast(bg), shade);
                }
                if let Some(((is_row, line), color)) = hint_line
                    && (if is_row { uy } else { ux }) == line
                {
                    bg = blend(bg, color, 0.2);
                }
            }
            let (px, py) = (gx + x * cw, gy + y * ch);
            c.fill(px, py, cw, ch, bg);
            let ink = blend(bg, contrast(bg), 0.45);
            if val > 0 && live && g.near_board[val as usize] {
                // fills that look like the board get a texture so they stay visible
                for dy in 0..ch {
                    for dx in 0..cw {
                        c.put(
                            px + dx,
                            py + dy,
                            Some('░'),
                            Some(ink),
                            None,
                            Modifier::empty(),
                        );
                    }
                }
            } else if val == CROSS && live {
                if ch == 1 {
                    // one character and a space: pairs like "><" turn into ligatures in a row
                    c.put(
                        px + cw / 2 - 1,
                        py,
                        Some(cross),
                        Some(ink),
                        None,
                        Modifier::empty(),
                    );
                } else {
                    // two diagonals over ch rows and as many columns
                    let left = px + (cw - ch) / 2;
                    for d in 0..ch {
                        let sym = if 2 * d + 1 == ch { '╳' } else { '╲' };
                        c.put(
                            left + d,
                            py + d,
                            Some(sym),
                            Some(ink),
                            None,
                            Modifier::empty(),
                        );
                        if 2 * d + 1 != ch {
                            c.put(
                                left + ch - 1 - d,
                                py + d,
                                Some('╱'),
                                Some(ink),
                                None,
                                Modifier::empty(),
                            );
                        }
                    }
                }
            }
        }
    }
    let cell_box = |i: usize| (gx + (i % g.w) as i32 * cw, gy + (i / g.w) as i32 * ch);
    // an outline in a colour that shows on the cell it is drawn on
    let mark = |c: &mut Canvas, i: usize, color: Rgb| {
        let (px, py) = cell_box(i);
        let val = g.view(i % g.w, i / g.w);
        let under = if val > 0 { g.pal[val as usize] } else { board };
        let near = (0..3)
            .map(|k| (under[k].abs_diff(color[k]) as u32).pow(2))
            .sum::<u32>()
            < 130 * 130;
        c.outline(px, py, cw, ch, if near { contrast(under) } else { color });
    };
    for &i in g.flash.iter().flatten() {
        mark(&mut c, i, [0xe5, 0x48, 0x4d]);
    }
    if let Some(hint) = hint {
        for &i in &hint.cells {
            mark(&mut c, i, hint_color(hint.kind));
        }
    }
    if g.kb && live && g.cursor_kind() == Kind::Cell {
        mark(
            &mut c,
            g.cursor.1 as usize * g.w + g.cursor.0 as usize,
            th.accent,
        );
    }

    // clue bands
    let b = v.bands(g);
    let (top_y, left_x) = (b.by.max(0), b.bx.max(0));
    // the bands stop where the board does
    let (board_r, board_b) = (vw.min(gx + w * cw), vh.min(gy + h * ch));
    // lines are told apart the way cells are; the focused one and the one the last Help worked from stand out
    let band_bg = |i: i32, is_row: bool| -> Rgb {
        if i == if is_row { fy } else { fx } {
            return blend(th.panel, th.accent, 0.3);
        }
        if let Some(((hint_row, line), color)) = hint_line
            && hint_row == is_row
            && line as i32 == i
        {
            return blend(th.panel, color, 0.3);
        }
        blend(
            th.panel,
            th.ink,
            (i / 5 % 2) as f32 * 0.07 + (i % 2) as f32 * 0.05,
        )
    };
    let cursor = if g.kb && live { g.cursor } else { (0, 0) };
    let clue = |c: &mut Canvas,
                item: Run,
                auto: bool,
                marked: bool,
                at: (i32, i32, i32, i32),
                band: Rgb,
                hl: bool,
                cur: bool| {
        let (x, y, bw, bh) = at;
        let marked = marked || (auto && auto_clues);
        let dim = auto || marked;
        let (mut fg, mut bg) = (
            if dim {
                blend(band, th.muted, 0.75)
            } else {
                th.ink
            },
            band,
        );
        if g.color {
            let col = g.pal[item.c as usize];
            let ink = if luminance(col) > 0.55 { BLACK } else { WHITE };
            bg = if dim { blend(bg, col, 0.3) } else { col };
            fg = if dim { blend(bg, ink, 0.55) } else { ink };
        }
        if cur {
            (fg, bg) = (th.accent_ink, th.accent);
        }
        let boxed = g.color || cur;
        if boxed {
            // half a cell of the band stays free at the right and at the bottom, where there
            // is room for it, so that neighbouring boxes of one colour don't merge
            c.fill(x, y, bw, bh, bg);
            let (gap_x, gap_y) = (bw >= 4, bh >= 2);
            for dy in 0..bh {
                for dx in 0..bw {
                    let sym = match (gap_x && dx == bw - 1, gap_y && dy == bh - 1) {
                        (true, true) => '▘',
                        (true, false) => '▌',
                        (false, true) => '▀',
                        _ => continue,
                    };
                    c.put(
                        x + dx,
                        y + dy,
                        Some(sym),
                        Some(bg),
                        Some(band),
                        Modifier::empty(),
                    );
                }
            }
        }
        let mut m = Modifier::empty();
        m.set(Modifier::BOLD, (hl && !dim) || cur);
        m.set(Modifier::CROSSED_OUT, marked);
        let mut text = item.n.to_string();
        if text.len() as i32 > bw {
            // no room for three digits: the last two, underlined
            text = text[text.len() - bw as usize..].to_string();
            m |= Modifier::UNDERLINED;
        }
        let len = text.len() as i32;
        // right-aligned in narrow boxes; centred in wide ones, beside the gap if there is one
        let off = if bw <= 3 {
            bw - len
        } else {
            (bw - len + !boxed as i32) / 2
        };
        c.text(
            x + off,
            y + if boxed { (bh - 1) / 2 } else { bh / 2 },
            &text,
            fg,
            m,
        );
    };

    // top clues
    c.clip(left_x, top_y, board_r, b.bottom);
    c.fill(left_x, top_y, vw, vh, th.panel);
    c.clip(b.right, top_y, board_r, b.bottom);
    for x in x0..x1 {
        let ux = x as usize;
        let (items, done, marks) = (&g.col_clues[ux], &g.col_done[ux], &g.col_marks[ux]);
        let (px, bg) = (gx + x * cw, band_bg(x, false));
        c.fill(px, top_y, cw, b.bottom - top_y, bg);
        let len = items.len() as i32;
        for (k, &item) in items.iter().enumerate() {
            let k32 = k as i32;
            let at = (px, b.bottom - (len - k32), cw, 1);
            let cur = g.kb && live && cursor == (x, k32 - len);
            clue(
                &mut c,
                item,
                done.done[k],
                marks.contains(&k),
                at,
                bg,
                fx == x,
                cur,
            );
        }
    }
    // left clues
    c.clip(left_x, b.bottom, b.right, board_b);
    c.fill(left_x, b.bottom, vw, vh, th.panel);
    let bw = v.box_w();
    for y in y0..y1 {
        let uy = y as usize;
        let (items, done, marks) = (&g.row_clues[uy], &g.row_done[uy], &g.row_marks[uy]);
        let (py, bg) = (gy + y * ch, band_bg(y, true));
        c.fill(left_x, py, b.right - left_x, ch, bg);
        let len = items.len() as i32;
        for (k, &item) in items.iter().enumerate() {
            let k32 = k as i32;
            let at = (b.bx + b.w - PAD - (len - k32) * bw, py, bw, ch);
            let cur = g.kb && live && cursor == (k32 - len, y);
            clue(
                &mut c,
                item,
                done.done[k],
                marks.contains(&k),
                at,
                bg,
                fy == y,
                cur,
            );
        }
    }

    // corner: size / position / stroke length
    c.clip(left_x, top_y, b.right, b.bottom);
    c.fill(left_x, top_y, vw, vh, th.panel);
    let (text, fg, m) = match (g.stroke_text(), focus) {
        (Some(len), _) => (len, th.ink, Modifier::BOLD),
        (None, Some((x, y))) if x >= 0 && y >= 0 => {
            (format!("{}, {}", x + 1, y + 1), th.muted, Modifier::empty())
        }
        _ => (format!("{}×{}", g.w, g.h), th.muted, Modifier::empty()),
    };
    let len = text.chars().count() as i32;
    let (kw, kh) = (b.right - left_x, b.bottom - top_y);
    c.text(
        left_x + ((kw - PAD - len) / 2).max(0),
        top_y + (kh - 1) / 2,
        &text,
        fg,
        m,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::Opts;
    use katana_desktop::protocol::Board;

    fn game(w: u32, h: u32) -> Game {
        let board = Board {
            id: 1,
            w,
            h,
            palette: vec!["#ffffff".into(), "#000000".into()],
            // a diagonal: one number per line
            grid: (0..w * h).map(|i| (i % w == i / w % w) as u8).collect(),
        };
        Game::load(&board, false, &serde_json::Value::Null, Opts::default())
    }

    #[test]
    fn fit_picks_the_largest_zoom_and_centres() {
        let g = game(5, 5);
        let mut v = View {
            auto_fit: true,
            ..View::default()
        };
        v.resize(&g, 80, 24);
        assert_eq!(v.cell(), (8, 4)); // 4 + 5*8 columns, 1 + 5*4 rows
        assert_eq!(v.origin(), ((80 - 5 - 40) / 2 + 5, (24 - 1 - 20) / 2 + 1));
        v.resize(&g, 30, 12);
        assert_eq!(v.cell(), (4, 2));
    }

    #[test]
    fn hit_finds_cells_and_clue_numbers() {
        let g = game(5, 5);
        let mut v = View::default();
        v.resize(&g, 14, 6); // exactly the puzzle at 2x1: clues 3+1 wide, 1 high
        assert_eq!(v.origin(), (4, 1));
        assert_eq!(
            v.hit(&g, 4, 1),
            Some(Hit {
                kind: Kind::Cell,
                x: 0,
                y: 0,
                k: 0
            })
        );
        assert_eq!(
            v.hit(&g, 13, 5),
            Some(Hit {
                kind: Kind::Cell,
                x: 4,
                y: 4,
                k: 0
            })
        );
        assert_eq!(
            v.hit(&g, 7, 0),
            Some(Hit {
                kind: Kind::Col,
                x: 1,
                y: -1,
                k: 0
            })
        );
        for mx in 0..3 {
            assert_eq!(
                v.hit(&g, mx, 3),
                Some(Hit {
                    kind: Kind::Row,
                    x: -1,
                    y: 2,
                    k: 0
                })
            );
        }
        assert_eq!(v.hit(&g, 3, 3), None); // the gap before the grid
        assert_eq!(v.hit(&g, 0, 0), None); // the corner
    }

    #[test]
    fn a_big_board_scrolls_under_pinned_clues() {
        let g = game(40, 40);
        let mut v = View::default();
        v.resize(&g, 40, 20);
        assert_eq!(v.origin(), (4, 1));
        v.reveal(&g, (39, 39));
        let (gx, gy) = v.origin();
        assert_eq!((gx + 40 * 2, gy + 40), (40, 20)); // the far corner is at the stage's
        // the clues stay on screen, over the scrolled grid
        assert_eq!(
            v.hit(&g, 0, 19).map(|h| (h.kind, h.y)),
            Some((Kind::Row, 39))
        );
        assert_eq!(
            v.hit(&g, 39, 0).map(|h| (h.kind, h.x)),
            Some((Kind::Col, 39))
        );
        v.scroll(&g, -1000, -1000);
        assert_eq!(v.origin(), (4, 1));
    }
}
