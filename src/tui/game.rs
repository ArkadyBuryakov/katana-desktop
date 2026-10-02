//! The board: a port of the game in web/app.js, with the same rules and the same saved format,
//! so a board started in one frontend continues in the other.

use std::collections::{BTreeSet, HashMap, VecDeque};
use std::rc::Rc;

use katana_desktop::protocol::{Board, random_u64};
use serde_json::{Value, json};

pub const EMPTY: i16 = 0;
pub const CROSS: i16 = -1;

pub type Rgb = [u8; 3];

/// One clue number: a block of `n` cells of colour `c`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Run {
    pub n: usize,
    pub c: i16,
}

pub fn runs_of(line: &[i16]) -> Vec<Run> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < line.len() {
        let c = line[i];
        if c > 0 {
            let mut j = i;
            while j < line.len() && line[j] == c {
                j += 1;
            }
            out.push(Run { n: j - i, c });
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

pub fn parse_hex(s: &str) -> Rgb {
    let n = u32::from_str_radix(s.trim_start_matches('#'), 16).unwrap_or(0);
    [(n >> 16) as u8, (n >> 8) as u8, n as u8]
}

fn color_distance(a: Rgb, b: Rgb) -> f32 {
    (0..3)
        .map(|i| (a[i] as f32 - b[i] as f32).powi(2))
        .sum::<f32>()
        .sqrt()
}

pub fn luminance(c: Rgb) -> f32 {
    (0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32) / 255.0
}

/// Which arrangements of a clue fit a line, see [`solve_line`].
pub struct LineSol<'a> {
    clue: &'a [Run],
    n: usize,
    w: usize,
    /// per colour: prefix count of cells that can't take that colour
    blocked: Vec<Vec<u32>>,
    /// per clue number: its colour's index in `blocked`
    which: Vec<usize>,
    can_empty: Vec<bool>,
    // forward: first k numbers fill [0, i); g: cell i-1 empty (or i = 0), e: number k-1 ends at i
    g: Vec<bool>,
    e: Vec<bool>,
    // backward: numbers k.. fill [i, n); gb: cell i empty (or i = n), eb: number k starts at i
    gb: Vec<bool>,
    eb: Vec<bool>,
}

impl LineSol<'_> {
    fn fits(&self, k: usize, s: usize) -> bool {
        let e = s + self.clue[k].n;
        if e > self.n {
            return false;
        }
        let a = &self.blocked[self.which[k]];
        a[e] == a[s]
    }
    fn same_as_prev(&self, k: usize) -> bool {
        k > 0 && self.clue[k - 1].c == self.clue[k].c
    }
    fn same_as_next(&self, k: usize) -> bool {
        k + 1 < self.clue.len() && self.clue[k + 1].c == self.clue[k].c
    }
    /// Can clue number k occupy exactly cells [s, s + n_k) in some valid arrangement?
    pub fn exactly(&self, k: usize, s: usize) -> bool {
        if !self.fits(k, s) {
            return false;
        }
        let (w, e) = (self.w, s + self.clue[k].n);
        let before = self.g[s * w + k] || (self.e[s * w + k] && !self.same_as_prev(k));
        let after = self.gb[e * w + k + 1] || (self.eb[e * w + k + 1] && !self.same_as_next(k));
        before && after
    }
    /// Can cell i stay empty in some valid arrangement?
    pub fn empty(&self, i: usize) -> bool {
        let w = self.w;
        self.can_empty[i]
            && (0..w).any(|k| {
                (self.g[i * w + k] || self.e[i * w + k])
                    && (self.gb[(i + 1) * w + k] || self.eb[(i + 1) * w + k])
            })
    }
}

/// Line feasibility for a (possibly colour) nonogram line.
/// line[i]: 0 unknown, -1 crossed, c > 0 filled with colour c. Same-colour neighbouring
/// clue numbers need a gap, different colours may touch.
/// Returns None when no arrangement fits.
#[allow(clippy::needless_range_loop)] // the tables are indexed by cell and clue number
pub fn solve_line<'a>(line: &[i16], clue: &'a [Run]) -> Option<LineSol<'a>> {
    let (n, m) = (line.len(), clue.len());
    let w = m + 1;
    let mut colors: Vec<i16> = Vec::new();
    let mut blocked = Vec::new();
    let mut which = Vec::with_capacity(m);
    for r in clue {
        let idx = match colors.iter().position(|&c| c == r.c) {
            Some(idx) => idx,
            None => {
                let mut a = vec![0u32; n + 1];
                for i in 0..n {
                    a[i + 1] = a[i] + (line[i] != EMPTY && line[i] != r.c) as u32;
                }
                colors.push(r.c);
                blocked.push(a);
                colors.len() - 1
            }
        };
        which.push(idx);
    }
    let mut s = LineSol {
        clue,
        n,
        w,
        blocked,
        which,
        can_empty: line.iter().map(|&v| v <= 0).collect(),
        g: vec![false; (n + 1) * w],
        e: vec![false; (n + 1) * w],
        gb: vec![false; (n + 1) * w],
        eb: vec![false; (n + 1) * w],
    };
    s.g[0] = true;
    for i in 0..=n {
        for k in 0..=m {
            let (g, e) = (s.g[i * w + k], s.e[i * w + k]);
            if !g && !e {
                continue;
            }
            if i < n && s.can_empty[i] {
                s.g[(i + 1) * w + k] = true;
            }
            if k < m && (g || !s.same_as_prev(k)) && s.fits(k, i) {
                s.e[(i + clue[k].n) * w + k + 1] = true;
            }
        }
    }
    if !s.g[n * w + m] && !s.e[n * w + m] {
        return None;
    }
    s.gb[n * w + m] = true;
    for i in (0..n).rev() {
        for k in (0..=m).rev() {
            if s.can_empty[i] && (s.gb[(i + 1) * w + k] || s.eb[(i + 1) * w + k]) {
                s.gb[i * w + k] = true;
            }
            if k < m && s.fits(k, i) {
                let e = i + clue[k].n;
                if s.gb[e * w + k + 1] || (s.eb[e * w + k + 1] && !s.same_as_next(k)) {
                    s.eb[i * w + k] = true;
                }
            }
        }
    }
    Some(s)
}

pub fn line_full(line: &[i16], clue: &[Run]) -> bool {
    runs_of(line) == clue
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Done {
    /// done[k]: clue number k is solved
    pub done: Vec<bool>,
    pub full: bool,
    /// cells that must be empty because of the solved numbers (the caller skips the ones
    /// that aren't unknown)
    pub gaps: Vec<usize>,
}

pub fn done_for(line: &[i16], clue: &[Run]) -> Done {
    let (m, n) = (clue.len(), line.len());
    let mut done = vec![false; m];
    let mut gaps = Vec::new();
    if line_full(line, clue) {
        return Done {
            done: vec![true; m],
            full: true,
            gaps: (0..n).collect(),
        };
    }
    let sol = if m == 0 { None } else { solve_line(line, clue) };
    // no solution: the line contradicts its clue, claim nothing
    let Some(sol) = sol else {
        return Done {
            done,
            full: false,
            gaps,
        };
    };
    // A block is a solved number when, across all valid arrangements, the only clue
    // placement covering it is one number sitting exactly on it. This catches blocks
    // closed by crosses/edges/other colours, and also full-length blocks next to
    // unknown cells that can't grow any further.
    // A block that several numbers could be, all of its own length, is complete all the
    // same: it needs a gap on each side where every candidate's neighbour shares its colour.
    let mut at = vec![0usize; m]; // where each solved number starts
    let mut i = 0;
    while i < n {
        let c = line[i];
        if c <= 0 {
            i += 1;
            continue;
        }
        let mut e = i;
        while e < n && line[e] == c {
            e += 1;
        }
        let len = e - i;
        let (mut found, mut count, mut exact) = (0, 0, true);
        let (mut gap_l, mut gap_r) = (true, true);
        'numbers: for k in 0..m {
            if clue[k].c != c || clue[k].n < len {
                continue;
            }
            for s in e.saturating_sub(clue[k].n)..=i {
                if !sol.exactly(k, s) {
                    continue;
                }
                if clue[k].n != len {
                    exact = false;
                    break 'numbers;
                }
                found = k;
                count += 1;
                if k > 0 && clue[k - 1].c != c {
                    gap_l = false;
                }
                if k + 1 < m && clue[k + 1].c != c {
                    gap_r = false;
                }
            }
        }
        if exact && count > 0 {
            if gap_l && i > 0 {
                gaps.push(i - 1);
            }
            if gap_r && e < n {
                gaps.push(e);
            }
            if count == 1 {
                done[found] = true;
                at[found] = i;
            }
        }
        i = e;
    }
    // nothing but gaps between two solved neighbours, and between the border and the number next to it
    for k in 0..m {
        if !done[k] {
            continue;
        }
        if k == 0 {
            gaps.extend(0..at[k]);
        }
        if k == m - 1 {
            gaps.extend(at[k] + clue[k].n..n);
        } else if done[k + 1] {
            gaps.extend(at[k] + clue[k].n..at[k + 1]);
        }
    }
    Done {
        done,
        full: false,
        gaps,
    }
}

/// The sidebar helpers that change what the board shows.
#[derive(Clone, Copy, Debug, Default)]
pub struct Opts {
    pub auto_cross: bool,
    pub auto_gaps: bool,
}

/// [(cell, before, after)]
pub type Diff = Vec<(usize, i16, i16)>;

/// (is row, index) of a line, and the [(cell, value)] it settles
type Deduction = (bool, usize, Vec<(usize, i16)>);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HintKind {
    Mistake,
    Deduce,
    Reveal,
}

/// What the last Help did.
#[derive(Clone, Debug)]
pub struct Hint {
    pub kind: HintKind,
    pub cells: Vec<usize>,
    /// (is row, index) of the line that gave the cells away
    pub line: Option<(bool, usize)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Cell,
    Row,
    Col,
}

/// A mouse stroke: a straight line from (sx, sy) towards (cx, cy), previewed live.
pub struct Drag {
    sx: usize,
    sy: usize,
    pub cx: usize,
    pub cy: usize,
    from: i16,
    to: i16,
    changed: Vec<(usize, i16)>,
    len: usize,
    horiz: bool,
    prev_rows: Vec<usize>,
    prev_cols: Vec<usize>,
}

/// A keyboard stroke: Space held while the cursor moves.
pub struct SpaceStroke {
    kind: Kind,
    value: i16,
    mark: bool,
    changed: Vec<(usize, i16)>,
    len: usize,
    horiz: Option<bool>,
    line: usize,
    lo: usize,
    hi: usize,
    at: (usize, usize),
}

pub struct Game {
    pub id: u32,
    pub w: usize,
    pub h: usize,
    pub sol: Vec<i16>,
    pub color: bool,
    /// pal[0] is the board background
    pub pal: Vec<Rgb>,
    /// fills that look like the board
    pub near_board: Vec<bool>,
    pub cells: Vec<i16>,
    pub time: u32,
    /// times Help was used on this board; undo and Reset don't take them back
    pub helps: u32,
    pub solved: bool,
    /// manually crossed-out clue numbers
    pub row_marks: Vec<BTreeSet<usize>>,
    pub col_marks: Vec<BTreeSet<usize>>,
    pub row_clues: Vec<Vec<Run>>,
    pub col_clues: Vec<Vec<Run>>,
    pub max_row: usize,
    pub max_col: usize,
    total_filled: usize,
    undo: Vec<Diff>,
    redo: Vec<Diff>,
    pub tool: i16,
    pub cross_tool: bool,
    /// negative x: a row clue number, negative y: a column clue number
    pub cursor: (i32, i32),
    /// keyboard cursor visible
    pub kb: bool,
    pub drag: Option<Drag>,
    pub space: Option<SpaceStroke>,
    /// mistakes shown by Check
    pub flash: Option<Vec<usize>>,
    pub hint: Option<Hint>,
    /// auto crosses around solved numbers, see spread_gaps
    gaps: Vec<bool>,
    line_cache: Vec<HashMap<Vec<i16>, Rc<Done>>>,
    row_full: Vec<bool>,
    col_full: Vec<bool>,
    pub row_done: Vec<Rc<Done>>,
    pub col_done: Vec<Rc<Done>>,
    auto_cross_on: bool,
    pub opts: Opts,
    /// unsaved changes
    pub dirty: bool,
    /// set once when the last move finished the puzzle
    pub just_solved: bool,
}

impl Game {
    pub fn load(board: &Board, color: bool, progress: &Value, opts: Opts) -> Game {
        let (w, h) = (board.w as usize, board.h as usize);
        let sol: Vec<i16> = board.grid.iter().map(|&v| v as i16).collect();
        let mut pal: Vec<Rgb> = board.palette.iter().map(|c| parse_hex(c)).collect();
        if !color && pal.len() > 1 {
            pal[1] = [0x2b, 0x26, 0x20];
        }
        let near_board = pal
            .iter()
            .map(|&p| color_distance(p, pal[0]) < 64.0)
            .collect();
        let row_clues: Vec<Vec<Run>> = (0..h).map(|y| runs_of(&sol[y * w..(y + 1) * w])).collect();
        let col_clues: Vec<Vec<Run>> = (0..w)
            .map(|x| runs_of(&(0..h).map(|y| sol[y * w + x]).collect::<Vec<_>>()))
            .collect();
        let mut g = Game {
            id: board.id,
            w,
            h,
            color,
            pal,
            near_board,
            cells: vec![EMPTY; w * h],
            time: 0,
            helps: 0,
            solved: false,
            row_marks: vec![BTreeSet::new(); h],
            col_marks: vec![BTreeSet::new(); w],
            max_row: row_clues.iter().map(Vec::len).max().unwrap_or(0).max(1),
            max_col: col_clues.iter().map(Vec::len).max().unwrap_or(0).max(1),
            total_filled: sol.iter().filter(|&&v| v > 0).count(),
            row_clues,
            col_clues,
            sol,
            undo: Vec::new(),
            redo: Vec::new(),
            tool: 1,
            cross_tool: false,
            cursor: (0, 0),
            kb: false,
            drag: None,
            space: None,
            flash: None,
            hint: None,
            gaps: vec![false; w * h],
            line_cache: vec![HashMap::new(); w + h],
            row_full: Vec::new(),
            col_full: Vec::new(),
            row_done: Vec::new(),
            col_done: Vec::new(),
            auto_cross_on: false,
            opts,
            dirty: false,
            just_solved: false,
        };
        g.restore(progress);
        g.update_done(None);
        g
    }

    fn restore(&mut self, p: &Value) {
        let cells = p.get("cells").and_then(Value::as_array);
        let solved = p.get("solved").and_then(Value::as_bool).unwrap_or(false);
        let Some(cells) = cells.filter(|c| c.len() == self.w * self.h && !solved) else {
            return;
        };
        let colors = self.pal.len() as i64;
        for (cell, v) in self.cells.iter_mut().zip(cells) {
            let v = v.as_i64().unwrap_or(0);
            *cell = if v == -1 || (v > 0 && v < colors) {
                v as i16
            } else {
                EMPTY
            };
        }
        let num = |k: &str| p.get(k).and_then(Value::as_f64).unwrap_or(0.0).max(0.0) as u32;
        self.time = num("time");
        self.helps = num("helps");
        for (key, marks, clues) in [
            ("r", &mut self.row_marks, &self.row_clues),
            ("c", &mut self.col_marks, &self.col_clues),
        ] {
            let Some(saved) = p
                .pointer(&format!("/marks/{key}"))
                .and_then(Value::as_object)
            else {
                continue;
            };
            for (line, ks) in saved {
                let Ok(line) = line.parse::<usize>() else {
                    continue;
                };
                let (Some(set), Some(ks)) = (marks.get_mut(line), ks.as_array()) else {
                    continue;
                };
                let ks = ks.iter().filter_map(|k| Some(k.as_u64()? as usize));
                set.extend(ks.filter(|&k| k < clues[line].len()));
            }
        }
    }

    // ---- clue completion

    /// What cell i at (x, y) shows. Crosses filling the empty cells of finished lines, and the
    /// ones around solved numbers, are never stored: they are derived from row_full/col_full
    /// and gaps, so they disappear as soon as the line stops matching its clue and
    /// undo/redo/saves only ever see what the player did.
    pub fn view(&self, x: usize, y: usize) -> i16 {
        self.view_auto(x, y, true)
    }

    fn view_auto(&self, x: usize, y: usize, auto: bool) -> i16 {
        let i = y * self.w + x;
        let v = self.cells[i];
        if v != EMPTY || !auto {
            return v;
        }
        if self.gaps[i] || (self.auto_cross_on && (self.row_full[y] || self.col_full[x])) {
            CROSS
        } else {
            v
        }
    }

    /// Raw player cells of a line; with auto: also the auto crosses.
    fn line_state(&self, is_row: bool, i: usize, auto: bool) -> Vec<i16> {
        if is_row {
            (0..self.w).map(|k| self.view_auto(k, i, auto)).collect()
        } else {
            (0..self.h).map(|k| self.view_auto(i, k, auto)).collect()
        }
    }

    /// Recompute which numbers are solved, for the given (rows, columns) or for all lines.
    pub fn update_done(&mut self, only: Option<(&[usize], &[usize])>) {
        self.auto_cross_on = self.opts.auto_cross && !self.solved;
        let gaps_on = self.opts.auto_gaps && !self.solved;
        let only = only.filter(|_| !self.row_full.is_empty());
        let all = only.is_none();
        let (mut rows, mut cols) = match only {
            Some((r, c)) => (r.to_vec(), c.to_vec()),
            None => ((0..self.h).collect(), (0..self.w).collect()),
        };
        rows.sort_unstable();
        rows.dedup();
        cols.sort_unstable();
        cols.dedup();
        if all {
            self.row_full = vec![false; self.h];
            self.col_full = vec![false; self.w];
            let none = Rc::new(Done::default());
            self.row_done = vec![none.clone(); self.h];
            self.col_done = vec![none; self.w];
        }
        // fullness only depends on the player's own cells
        let (mut rows_flipped, mut cols_flipped) = (all, all);
        for &y in &rows {
            let f = line_full(&self.line_state(true, y, false), &self.row_clues[y]);
            if f != self.row_full[y] {
                self.row_full[y] = f;
                rows_flipped = true;
            }
        }
        for &x in &cols {
            let f = line_full(&self.line_state(false, x, false), &self.col_clues[x]);
            if f != self.col_full[x] {
                self.col_full[x] = f;
                cols_flipped = true;
            }
        }
        if gaps_on {
            self.spread_gaps();
            return;
        }
        if all {
            self.gaps.fill(false);
        }
        // a row finishing or unfinishing changes the auto crosses seen by every column, and vice versa
        if rows_flipped && self.auto_cross_on {
            cols = (0..self.w).collect();
        }
        if cols_flipped && self.auto_cross_on {
            rows = (0..self.h).collect();
        }
        for y in rows {
            self.row_done[y] = Rc::new(done_for(
                &self.line_state(true, y, true),
                &self.row_clues[y],
            ));
        }
        for x in cols {
            self.col_done[x] = Rc::new(done_for(
                &self.line_state(false, x, true),
                &self.col_clues[x],
            ));
        }
    }

    /// Auto crosses around solved numbers. A new cross can finish a number in the crossing line
    /// (or in its own), so lines are redone until nothing changes. Every move starts over from
    /// no crosses, so a line the move didn't touch goes through the states it went through last
    /// time: those come from the cache.
    fn spread_gaps(&mut self) {
        let (w, h) = (self.w, self.h);
        self.gaps.fill(false);
        // [rows, columns] still to do, and whether each line is queued
        let mut todo = [(0..h).collect::<VecDeque<_>>(), (0..w).collect()];
        let mut queued = [vec![true; h], vec![true; w]];
        while !todo[0].is_empty() || !todo[1].is_empty() {
            for side in 0..2 {
                let is_row = side == 0;
                while let Some(i) = todo[side].pop_front() {
                    queued[side][i] = false;
                    let line = self.line_state(is_row, i, true);
                    let seen = &mut self.line_cache[if is_row { i } else { h + i }];
                    let res = match seen.get(&line) {
                        Some(res) => res.clone(),
                        None => {
                            if seen.len() >= 16 {
                                seen.clear();
                            }
                            let clue = if is_row {
                                &self.row_clues[i]
                            } else {
                                &self.col_clues[i]
                            };
                            let res = Rc::new(done_for(&line, clue));
                            seen.insert(line.clone(), res.clone());
                            res
                        }
                    };
                    for &k in &res.gaps {
                        if line[k] != EMPTY {
                            continue;
                        }
                        self.gaps[if is_row { i * w + k } else { k * w + i }] = true;
                        for (s, l) in [(side, i), (1 - side, k)] {
                            if !queued[s][l] {
                                queued[s][l] = true;
                                todo[s].push_back(l);
                            }
                        }
                    }
                    if is_row {
                        self.row_done[i] = res;
                    } else {
                        self.col_done[i] = res;
                    }
                }
            }
        }
    }

    pub fn toggle_mark(&mut self, kind: Kind, idx: usize, k: usize, value: Option<bool>) -> bool {
        let set = match kind {
            Kind::Row => &mut self.row_marks[idx],
            _ => &mut self.col_marks[idx],
        };
        let on = value.unwrap_or(!set.contains(&k));
        if on {
            set.insert(k);
        } else {
            set.remove(&k);
        }
        self.dirty = true;
        on
    }

    // ---- mouse strokes

    /// Length of the run of cells showing the same as (x, y), along its row or its column.
    fn run_len(&self, x: usize, y: usize, horiz: bool) -> usize {
        let (dx, dy) = (horiz as i32, !horiz as i32);
        let v = self.view(x, y);
        let same = |x: i32, y: i32| {
            x >= 0
                && y >= 0
                && (x as usize) < self.w
                && (y as usize) < self.h
                && self.view(x as usize, y as usize) == v
        };
        let (x, y) = (x as i32, y as i32);
        let mut n = 1;
        for dir in [1, -1] {
            let mut k = 1;
            while same(x + dir * k * dx, y + dir * k * dy) {
                n += 1;
                k += 1;
            }
        }
        n
    }

    fn apply_stroke(&mut self) {
        let Some(mut d) = self.drag.take() else {
            return;
        };
        for &(i, before) in &d.changed {
            self.cells[i] = before; // undo the previous preview
        }
        d.changed.clear();
        let (sx, sy) = (d.sx, d.sy);
        let (mut cx, mut cy) = (d.cx, d.cy);
        if cx.abs_diff(sx) >= cy.abs_diff(sy) {
            cy = sy;
        } else {
            cx = sx;
        }
        d.horiz = cy == sy;
        let (x0, x1, y0, y1) = (sx.min(cx), sx.max(cx), sy.min(cy), sy.max(cy));
        d.len = x1 - x0 + y1 - y0 + 1;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let i = y * self.w + x;
                if self.cells[i] == d.from {
                    d.changed.push((i, self.cells[i]));
                    self.cells[i] = d.to;
                }
            }
        }
        let rows: Vec<usize> = (y0..=y1).collect();
        let cols: Vec<usize> = (x0..=x1).collect();
        let all_rows = [&rows[..], &d.prev_rows[..]].concat();
        let all_cols = [&cols[..], &d.prev_cols[..]].concat();
        self.update_done(Some((&all_rows, &all_cols)));
        d.prev_rows = rows;
        d.prev_cols = cols;
        self.drag = Some(d);
    }

    pub fn start_stroke(&mut self, x: usize, y: usize, cross: bool) {
        let cur = self.cells[y * self.w + x];
        let val = if cross { CROSS } else { self.tool };
        let erase = cur == val;
        self.drag = Some(Drag {
            sx: x,
            sy: y,
            cx: x,
            cy: y,
            from: if erase { val } else { cur },
            to: if erase { EMPTY } else { val },
            changed: Vec::new(),
            len: 1,
            horiz: true,
            prev_rows: Vec::new(),
            prev_cols: Vec::new(),
        });
        self.apply_stroke();
    }

    pub fn move_stroke(&mut self, x: usize, y: usize) {
        if let Some(d) = &mut self.drag
            && (d.cx, d.cy) != (x, y)
        {
            d.cx = x;
            d.cy = y;
            self.apply_stroke();
        }
    }

    pub fn end_stroke(&mut self) {
        if let Some(d) = self.drag.take()
            && !d.changed.is_empty()
        {
            let diff = d
                .changed
                .iter()
                .map(|&(i, before)| (i, before, self.cells[i]))
                .collect();
            self.commit(diff, false, None);
        }
    }

    // ---- keyboard cursor

    /// Space cycles: empty → fill (current colour) → cross → empty.
    fn cycle_value(&self, cur: i16) -> i16 {
        if cur == EMPTY {
            if self.cross_tool { CROSS } else { self.tool }
        } else if cur == CROSS {
            EMPTY
        } else if cur == self.tool || self.cross_tool {
            CROSS
        } else {
            self.tool
        }
    }

    pub fn cursor_kind(&self) -> Kind {
        match self.cursor {
            (x, _) if x < 0 => Kind::Row,
            (_, y) if y < 0 => Kind::Col,
            _ => Kind::Cell,
        }
    }

    pub fn move_cursor(&mut self, dx: i32, dy: i32) {
        let (mut x, mut y) = self.cursor;
        let (sx, sy) = (dx.signum(), dy.signum());
        for _ in 0..dx.abs().max(dy.abs()) {
            let (mut nx, mut ny) = (x + sx, y + sy);
            if (nx < 0 && ny < 0) || nx >= self.w as i32 || ny >= self.h as i32 {
                break;
            }
            if nx < 0 {
                // row clue area: only where a number exists
                let len = self.row_clues[ny as usize].len() as i32;
                if sx < 0 && nx < -len {
                    break;
                }
                if sy != 0 {
                    nx = nx.max(-len).min(if len == 0 { 0 } else { -1 });
                }
            }
            if ny < 0 {
                // column clue area
                let len = self.col_clues[nx as usize].len() as i32;
                if sy < 0 && ny < -len {
                    break;
                }
                if sx != 0 {
                    ny = ny.max(-len).min(if len == 0 { 0 } else { -1 });
                }
            }
            (x, y) = (nx, ny);
            self.cursor = (x, y);
            if self.space.is_some() {
                self.apply_space(false, sx != 0);
            }
        }
        self.kb = true;
    }

    /// Space pressed (first) or the cursor moved (horizontally or not) while it is held.
    pub fn apply_space(&mut self, first: bool, horiz: bool) {
        let (cx, cy) = self.cursor;
        let kind = self.cursor_kind();
        if kind == Kind::Cell {
            let (x, y) = (cx as usize, cy as usize);
            let i = y * self.w + x;
            if first {
                self.space = Some(SpaceStroke {
                    kind,
                    value: self.cycle_value(self.cells[i]),
                    mark: false,
                    changed: Vec::new(),
                    len: 1,
                    horiz: None,
                    line: 0,
                    lo: 0,
                    hi: 0,
                    at: (x, y),
                });
            }
            let Some(st) = self.space.as_mut().filter(|st| st.kind == Kind::Cell) else {
                return;
            };
            if !first {
                // the counted length starts over, from the cell the cursor turned at, when the
                // movement changes direction
                let (p, line) = if horiz { (x, y) } else { (y, x) };
                if st.horiz != Some(horiz) || st.line != line {
                    st.horiz = Some(horiz);
                    st.line = line;
                    let (at_p, at_line) = if horiz { st.at } else { (st.at.1, st.at.0) };
                    st.lo = if at_line == line { at_p } else { p };
                    st.hi = st.lo;
                }
                st.lo = st.lo.min(p);
                st.hi = st.hi.max(p);
                st.len = st.hi - st.lo + 1;
            }
            st.at = (x, y);
            if self.cells[i] == st.value {
                return;
            }
            if !st.changed.iter().any(|&(c, _)| c == i) {
                st.changed.push((i, self.cells[i]));
            }
            self.cells[i] = st.value;
            self.update_done(Some((&[y], &[x])));
        } else {
            let (idx, k) = match kind {
                Kind::Row => (cy as usize, self.row_clues[cy as usize].len() as i32 + cx),
                _ => (cx as usize, self.col_clues[cx as usize].len() as i32 + cy),
            };
            if k < 0 {
                return;
            }
            let k = k as usize;
            if first {
                let mark = self.toggle_mark(kind, idx, k, None);
                self.space = Some(SpaceStroke {
                    kind,
                    value: EMPTY,
                    mark,
                    changed: Vec::new(),
                    len: 1,
                    horiz: None,
                    line: 0,
                    lo: 0,
                    hi: 0,
                    at: (0, 0),
                });
            } else if let Some(mark) = self
                .space
                .as_ref()
                .filter(|st| st.kind == kind)
                .map(|st| st.mark)
            {
                self.toggle_mark(kind, idx, k, Some(mark));
            }
        }
    }

    pub fn end_space(&mut self) {
        if let Some(st) = self.space.take()
            && !st.changed.is_empty()
        {
            let diff = st
                .changed
                .iter()
                .map(|&(i, before)| (i, before, self.cells[i]))
                .collect();
            self.commit(diff, false, None);
        }
    }

    /// For the corner while a stroke is being made: its length, and the length of the run it
    /// ended up part of when that is longer.
    pub fn stroke_text(&self) -> Option<String> {
        let (len, erasing, x, y, horiz) = if let Some(d) = &self.drag {
            (d.len, d.to == EMPTY, d.sx, d.sy, d.horiz)
        } else if let Some(st) = self.space.as_ref().filter(|st| st.kind == Kind::Cell) {
            (
                st.len,
                st.value == EMPTY,
                st.at.0,
                st.at.1,
                st.horiz.unwrap_or(true),
            )
        } else {
            return None;
        };
        if len < 2 {
            return None;
        }
        let run = if erasing {
            len
        } else {
            self.run_len(x, y, horiz)
        };
        Some(if run == len {
            len.to_string()
        } else {
            format!("{len}/{run}")
        })
    }

    // ---- history / persistence

    fn commit(&mut self, diff: Diff, from_history: bool, hint: Option<Hint>) {
        self.hint = hint; // a Help highlight lasts until the next move
        if !from_history {
            self.undo.push(diff);
            self.redo.clear();
        }
        self.dirty = true;
        self.check_win();
    }

    pub fn history(&mut self, back: bool) {
        let Some(diff) = (if back {
            self.undo.pop()
        } else {
            self.redo.pop()
        }) else {
            return;
        };
        for &(i, before, after) in &diff {
            self.cells[i] = if back { before } else { after };
        }
        if back {
            self.redo.push(diff);
        } else {
            self.undo.push(diff);
        }
        self.update_done(None);
        self.commit(Vec::new(), true, None);
    }

    pub fn reset(&mut self) {
        let diff: Diff = (0..self.cells.len())
            .filter(|&i| self.cells[i] != EMPTY)
            .map(|i| (i, self.cells[i], EMPTY))
            .collect();
        self.row_marks.iter_mut().for_each(BTreeSet::clear);
        self.col_marks.iter_mut().for_each(BTreeSet::clear);
        self.cells.fill(EMPTY);
        self.hint = None;
        self.update_done(None);
        if !diff.is_empty() {
            self.undo.push(diff);
            self.redo.clear();
        }
        self.dirty = true;
    }

    fn mistakes(&self) -> Vec<usize> {
        (0..self.cells.len())
            .filter(|&i| {
                let (v, s) = (self.cells[i], self.sol[i]);
                (v > 0 && v != s) || (v == CROSS && s > 0)
            })
            .collect()
    }

    /// Flash the mistakes; returns how many there are.
    pub fn check(&mut self) -> usize {
        let bad = self.mistakes();
        let n = bad.len();
        self.flash = (n > 0).then_some(bad);
        n
    }

    // ---- help

    /// Every run of unknown cells that one line alone settles.
    fn deductions(&self) -> Vec<Deduction> {
        let mut out: Vec<Deduction> = Vec::new();
        for is_row in [true, false] {
            let clues = if is_row {
                &self.row_clues
            } else {
                &self.col_clues
            };
            for (idx, clue) in clues.iter().enumerate() {
                let line = self.line_state(is_row, idx, true);
                let n = line.len();
                let Some(sol) = solve_line(&line, clue) else {
                    continue;
                };
                // the colour each cell may take: 0 none, c just that one, -1 several
                let mut can = vec![0i16; n];
                for (k, &Run { n: len, c }) in clue.iter().enumerate() {
                    let mut from = 0;
                    for s in 0..=n.saturating_sub(len) {
                        if !sol.exactly(k, s) {
                            continue;
                        }
                        for cell in &mut can[s.max(from)..s + len] {
                            *cell = if *cell == 0 || *cell == c { c } else { -1 };
                        }
                        from = s + len;
                    }
                }
                let mut last = 0;
                for j in 0..n {
                    let mut v = 0;
                    if line[j] == EMPTY {
                        if can[j] == 0 {
                            v = CROSS;
                        } else if can[j] > 0 && !sol.empty(j) {
                            v = can[j];
                        }
                    }
                    if v != 0 {
                        if v != last {
                            out.push((is_row, idx, Vec::new()));
                        }
                        let cell = if is_row {
                            idx * self.w + j
                        } else {
                            j * self.w + idx
                        };
                        out.last_mut().unwrap().2.push((cell, v));
                    }
                    last = v;
                }
            }
        }
        out
    }

    /// Does the first that applies: fix a mistake, settle something a single line gives away,
    /// reveal a random cell. Returns what to tell the player and the first cell it touched.
    pub fn help(&mut self) -> Option<(String, usize)> {
        if self.solved || self.drag.is_some() || self.space.is_some() {
            return None;
        }
        let or_cross = |v: i16| if v > 0 { v } else { CROSS };
        let pick = |n: usize| (random_u64() % n as u64) as usize;
        let bad = self.mistakes();
        let (msg, kind, line, cells) = if let Some(&i) = bad.first() {
            let msg = match bad.len() {
                1 => "Fixed a mistake".to_string(),
                n => format!("Fixed a mistake, {} more left", n - 1),
            };
            (
                msg,
                HintKind::Mistake,
                None,
                vec![(i, or_cross(self.sol[i]))],
            )
        } else {
            let mut found = self.deductions();
            if !found.is_empty() {
                let (is_row, idx, cells) = found.swap_remove(pick(found.len()));
                let msg = format!(
                    "{} {} gives this away",
                    if is_row { "Row" } else { "Column" },
                    idx + 1
                );
                (msg, HintKind::Deduce, Some((is_row, idx)), cells)
            } else {
                let open: Vec<usize> = (0..self.cells.len())
                    .filter(|&i| self.view(i % self.w, i / self.w) == EMPTY)
                    .collect();
                if open.is_empty() {
                    return None;
                }
                let i = open[pick(open.len())];
                let msg = "No line gives anything away: revealed a cell".to_string();
                (
                    msg,
                    HintKind::Reveal,
                    None,
                    vec![(i, or_cross(self.sol[i]))],
                )
            }
        };
        self.helps += 1;
        let diff: Diff = cells.iter().map(|&(i, v)| (i, self.cells[i], v)).collect();
        for &(i, v) in &cells {
            self.cells[i] = v;
        }
        self.update_done(None);
        let hint = Hint {
            kind,
            cells: cells.iter().map(|c| c.0).collect(),
            line,
        };
        let first = cells[0].0;
        self.commit(diff, false, Some(hint));
        Some((msg, first))
    }

    fn check_win(&mut self) {
        if self.solved
            || self
                .cells
                .iter()
                .zip(&self.sol)
                .any(|(&v, &s)| v.max(0) != s)
        {
            return;
        }
        self.solved = true;
        self.just_solved = true;
        for v in &mut self.cells {
            if *v == CROSS {
                *v = EMPTY;
            }
        }
        self.update_done(None);
    }

    /// The saved board, in the format the web UI reads and writes.
    pub fn snapshot(&self) -> Value {
        let marks = |sets: &[BTreeSet<usize>]| -> Value {
            let filled = sets.iter().enumerate().filter(|(_, s)| !s.is_empty());
            Value::Object(filled.map(|(i, s)| (i.to_string(), json!(s))).collect())
        };
        let filled = self.cells.iter().filter(|&&v| v > 0).count();
        json!({
            "cells": self.cells,
            "time": self.time,
            "helps": self.helps,
            "solved": self.solved,
            "marks": { "r": marks(&self.row_marks), "c": marks(&self.col_marks) },
            "pct": (filled * 100 / self.total_filled.max(1)).min(99),
        })
    }

    /// Colour k (1-based); None toggles the cross tool.
    pub fn select_tool(&mut self, t: Option<usize>) {
        match t {
            None => self.cross_tool = !self.cross_tool,
            Some(t) if t >= 1 && t < self.pal.len() => {
                self.tool = t as i16;
                self.cross_tool = false;
            }
            _ => {}
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clue(ns: &[usize]) -> Vec<Run> {
        ns.iter().map(|&n| Run { n, c: 1 }).collect()
    }

    /// A board from rows of text: '.' empty, digits are colours.
    fn board(rows: &[&str]) -> Board {
        let grid: Vec<u8> = rows
            .iter()
            .flat_map(|r| r.bytes().map(|b| b.saturating_sub(b'0')))
            .collect();
        let colors = *grid.iter().max().unwrap() as usize;
        let mut palette = vec![
            "#ffffff".to_string(),
            "#000000".into(),
            "#ff0000".into(),
            "#0000ff".into(),
        ];
        palette.truncate(colors + 1);
        Board {
            id: 1,
            w: rows[0].len() as u32,
            h: rows.len() as u32,
            palette,
            grid,
        }
    }

    fn game(rows: &[&str], opts: Opts) -> Game {
        let b = board(rows);
        let color = b.palette.len() > 2;
        Game::load(&b, color, &Value::Null, opts)
    }

    #[test]
    fn runs() {
        assert_eq!(
            runs_of(&[1, 1, 0, 2, 2, 1, -1, 1]),
            vec![
                Run { n: 2, c: 1 },
                Run { n: 2, c: 2 },
                Run { n: 1, c: 1 },
                Run { n: 1, c: 1 }
            ]
        );
    }

    #[test]
    fn line_solver() {
        // 3 in 5 cells: only the middle cell is certain
        let c = clue(&[3]);
        let s = solve_line(&[0; 5], &c).unwrap();
        assert!(s.exactly(0, 0) && s.exactly(0, 1) && s.exactly(0, 2) && !s.exactly(0, 3));
        assert!(s.empty(0) && s.empty(1) && !s.empty(2) && s.empty(3));
        // a cross in the middle leaves no room
        assert!(solve_line(&[0, 0, -1, 0, 0], &c).is_none());
        // same colours need a gap, different ones may touch
        assert!(solve_line(&[0; 3], &clue(&[2, 1])).is_none());
        let two = [Run { n: 2, c: 1 }, Run { n: 1, c: 2 }];
        assert!(solve_line(&[0; 3], &two).unwrap().exactly(1, 2));
    }

    #[test]
    fn solved_numbers() {
        // "1 1" with the first block closed by the border and a cross
        let d = done_for(&[1, -1, 0, 0, 0], &clue(&[1, 1]));
        assert_eq!(d.done, [true, false]);
        // a full-length block can't grow: both neighbours are gaps
        let d = done_for(&[0, 1, 1, 1, 0, 0, 0, 0], &clue(&[3, 1]));
        assert_eq!(d.done, [true, false]);
        assert_eq!(d.gaps, [0, 4, 0]);
        // a finished line: everything else is a gap
        let d = done_for(&[0, 1, 1, 1, 0], &clue(&[3]));
        assert!(d.full && d.done == [true] && d.gaps == [0, 1, 2, 3, 4]);
        // a line that contradicts its clue claims nothing
        let d = done_for(&[1, 1, 1, 1, 0], &clue(&[2]));
        assert_eq!(
            d,
            Done {
                done: vec![false],
                full: false,
                gaps: vec![]
            }
        );
    }

    #[test]
    fn strokes_undo_and_win() {
        let mut g = game(&["11.", ".1.", "..."], Opts::default());
        g.start_stroke(0, 0, false);
        g.move_stroke(2, 1); // mostly horizontal: stays on row 0
        assert_eq!(g.stroke_text().as_deref(), Some("3"));
        g.end_stroke();
        assert_eq!(&g.cells[..3], [1, 1, 1]);
        g.history(true);
        assert_eq!(&g.cells[..3], [0, 0, 0]);
        g.history(false);
        assert_eq!(&g.cells[..3], [1, 1, 1]);
        // erasing starts from a cell that already has the tool's value
        g.start_stroke(2, 0, false);
        g.end_stroke();
        assert_eq!(&g.cells[..3], [1, 1, 0]);
        assert_eq!(g.check(), 0);
        g.start_stroke(1, 1, true);
        g.end_stroke();
        assert_eq!(g.check(), 1);
        assert!(!g.solved);
        g.start_stroke(1, 1, true); // erases the cross
        g.end_stroke();
        g.start_stroke(1, 1, false);
        g.end_stroke();
        assert!(g.solved && g.just_solved);
        assert_eq!(g.snapshot()["solved"], true);
    }

    #[test]
    fn keyboard_cursor_and_space() {
        let mut g = game(&["11.", ".1.", "..."], Opts::default());
        // up from the board goes into the column clues, but not past the last number
        g.move_cursor(0, -5);
        assert_eq!(g.cursor, (0, -1));
        g.apply_space(true, false);
        g.end_space();
        assert!(g.col_marks[0].contains(&0));
        g.move_cursor(0, 1);
        g.move_cursor(-5, 0);
        assert_eq!(g.cursor, (-1, 0));
        // the third row has no numbers: moving down from a row clue lands on the board
        g.move_cursor(0, 2);
        assert_eq!(g.cursor, (0, 2));
        // hold space and move: the same mark is repeated
        g.cursor = (0, 0);
        g.apply_space(true, false);
        g.move_cursor(1, 0);
        assert_eq!(g.stroke_text().as_deref(), Some("2"));
        g.end_space();
        assert_eq!(&g.cells[..3], [1, 1, 0]);
        // fill → cross → empty
        for want in [CROSS, EMPTY, 1] {
            g.apply_space(true, false);
            g.end_space();
            assert_eq!(g.cells[1], want);
        }
    }

    #[test]
    fn auto_crosses_are_never_stored() {
        let opts = Opts {
            auto_cross: true,
            auto_gaps: true,
        };
        let mut g = game(&["11.", "...", "..1"], opts);
        g.start_stroke(0, 0, false);
        g.move_stroke(1, 0);
        g.end_stroke();
        assert_eq!(g.view(2, 0), CROSS); // the rest of the finished row
        assert_eq!(g.view(0, 1), CROSS); // below the finished column
        assert_eq!(g.cells.iter().filter(|&&v| v == CROSS).count(), 0);
        g.history(true);
        assert_eq!(g.view(2, 0), EMPTY);
    }

    #[test]
    fn help_fixes_then_deduces() {
        let mut g = game(&["111", "...", "1.1"], Opts::default());
        g.start_stroke(1, 1, false);
        g.end_stroke();
        let (msg, cell) = g.help().unwrap();
        assert_eq!((msg.as_str(), cell), ("Fixed a mistake", 4));
        assert_eq!(g.cells[4], CROSS);
        while !g.solved {
            let before = g.cells.clone();
            g.help().unwrap();
            assert_ne!(g.cells, before);
            assert_eq!(g.check(), 0);
        }
        assert!(g.helps >= 2);
    }

    #[test]
    fn saved_board_round_trips() {
        let mut g = game(&["12.", ".1.", "3.."], Opts::default());
        g.start_stroke(0, 0, false);
        g.end_stroke();
        g.select_tool(Some(2));
        g.start_stroke(1, 0, false);
        g.end_stroke();
        g.start_stroke(2, 2, true);
        g.end_stroke();
        g.toggle_mark(Kind::Row, 0, 1, None);
        g.time = 42;
        let saved = g.snapshot();
        assert_eq!(saved["pct"], 50);
        let back = Game::load(
            &board(&["12.", ".1.", "3.."]),
            true,
            &saved,
            Opts::default(),
        );
        assert_eq!(back.cells, g.cells);
        assert_eq!(back.time, 42);
        assert!(back.row_marks[0].contains(&1));
    }
}
