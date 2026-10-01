//! Local state: session, account progress mirror, catalog cache, in-progress boards.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::protocol::{self as p, Puzzle, Result, Session};

const CATALOG_MAX_AGE: Duration = Duration::from_secs(12 * 3600);

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &PathBuf) -> Option<T> {
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn write_atomic(path: &PathBuf, data: &[u8]) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    // unique per call: the same image can be fetched by two requests at once
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let tmp = path.with_extension(format!("tmp{}", SEQ.fetch_add(1, Ordering::Relaxed)));
    fs::write(&tmp, data).map_err(|e| e.to_string())?;
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}

fn write_json<T: Serialize>(path: &PathBuf, v: &T) -> Result<()> {
    write_atomic(path, &serde_json::to_vec(v).unwrap())
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Pending {
    pub id: u32,
    pub score: i32,
    pub seconds: i32,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct AccountState {
    pub solved: BTreeSet<u32>,
    pub pending: Vec<Pending>,
    pub score: Option<i32>,
    pub last_sync: f64,
    pub sync_error: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ProgressInfo {
    pub pct: u32,
    pub t: f64,
    pub solved: bool,
}

#[derive(Default)]
pub struct Catalog {
    pub by_id: HashMap<u32, Puzzle>,
    /// newest first
    pub list: Vec<u32>,
    pub time: f64,
    pub loading: bool,
    pub error: Option<String>,
}

pub struct Store {
    dir: PathBuf,
    pub session: Mutex<Option<Session>>,
    pub account: Mutex<AccountState>,
    pub progress: Mutex<HashMap<u32, ProgressInfo>>,
    pub catalog: RwLock<Catalog>,
    sync_lock: Mutex<()>,
}

impl Store {
    pub fn open() -> Arc<Store> {
        let dir = std::env::var_os("KATANA_DATA")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                dirs::data_local_dir() // same as data_dir on Linux/macOS; %LOCALAPPDATA% on Windows
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join("katana-desktop")
            });
        let _ = fs::create_dir_all(dir.join("puzzles"));
        let _ = fs::create_dir_all(dir.join("progress"));
        let progress: HashMap<String, ProgressInfo> =
            read_json(&dir.join("progress.json")).unwrap_or_default();
        Arc::new(Store {
            session: Mutex::new(read_json(&dir.join("session.json"))),
            account: Mutex::new(read_json(&dir.join("account.json")).unwrap_or_default()),
            progress: Mutex::new(
                progress
                    .into_iter()
                    .filter_map(|(k, v)| Some((k.parse().ok()?, v)))
                    .collect(),
            ),
            catalog: RwLock::new(Catalog::default()),
            sync_lock: Mutex::new(()),
            dir,
        })
    }

    pub fn dir(&self) -> &PathBuf {
        &self.dir
    }

    fn save_account(&self, a: &AccountState) {
        let _ = write_json(&self.dir.join("account.json"), a);
    }

    pub fn status(&self) -> Value {
        let s = self.session.lock().unwrap().clone();
        let a = self.account.lock().unwrap().clone();
        let c = self.catalog.read().unwrap();
        json!({
            "logged_in": s.is_some(),
            "nickname": s.as_ref().map(|s| s.nickname.clone()),
            "email": s.as_ref().map(|s| s.email.clone()),
            "solved_count": a.solved.len(),
            "pending": a.pending.len(),
            "score": a.score,
            "last_sync": a.last_sync,
            "sync_error": a.sync_error,
            "catalog_count": c.by_id.len(),
            "catalog_time": c.time,
            "catalog_loading": c.loading,
            "catalog_error": c.error,
            "data_dir": self.dir.display().to_string(),
        })
    }

    // ------------------------------------------------------------ account

    fn set_session(&self, s: Option<Session>) {
        match &s {
            Some(s) => {
                let _ = write_json(&self.dir.join("session.json"), s);
            }
            None => {
                let _ = fs::remove_file(self.dir.join("session.json"));
            }
        }
        *self.session.lock().unwrap() = s;
    }

    fn device_id(&self) -> String {
        let path = self.dir.join("device_id");
        if let Ok(id) = fs::read_to_string(&path)
            && id.trim().len() == 32
        {
            return id.trim().to_string();
        }
        let id = p::new_device_id();
        let _ = fs::write(&path, &id);
        id
    }

    pub fn login(&self, email: &str, password: &str) -> Result<Value> {
        if email.trim().is_empty() || password.is_empty() {
            return Err("Enter your email and password".into());
        }
        let s = p::login(email, password, &self.device_id())?;
        self.switch_account(s);
        let _ = self.sync(); // a sync failure is shown in the status bar, not as a login error
        Ok(self.status())
    }

    pub fn register(&self, email: &str, password: &str, nickname: &str) -> Result<Value> {
        p::validate_signup(email, password, Some(nickname))?;
        let s = p::register(email, password, nickname.trim(), &self.device_id())?;
        self.switch_account(s);
        let _ = self.sync(); // a sync failure is shown in the status bar, not as a login error
        Ok(self.status())
    }

    /// A different account must not inherit the previous one's solved list. Solves made
    /// without an account are kept as pending, so the first sync uploads them.
    fn switch_account(&self, s: Session) {
        let prev = self.session.lock().unwrap().as_ref().map(|s| s.user_id);
        if prev.is_some_and(|p| p != s.user_id) {
            let mut a = self.account.lock().unwrap();
            *a = AccountState::default();
            self.save_account(&a);
        }
        self.set_session(Some(s));
    }

    pub fn logout(&self, force: bool) -> Result<Value> {
        let pending = self.account.lock().unwrap().pending.len();
        if pending > 0 && !force {
            // try to push first; refuse to drop unsynced solves silently
            if self.sync().is_err() || !self.account.lock().unwrap().pending.is_empty() {
                return Err(format!("{pending} solved puzzle(s) are not synced yet"));
            }
        }
        if let Some(s) = self.session.lock().unwrap().clone() {
            let _ = p::logout(&s); // server-side token invalidation is best effort
        }
        self.set_session(None);
        let mut a = self.account.lock().unwrap();
        *a = AccountState::default();
        self.save_account(&a);
        drop(a);
        Ok(self.status())
    }

    fn sync_inner(&self) -> Result<usize> {
        let sess = self
            .session
            .lock()
            .unwrap()
            .clone()
            .ok_or("Not logged in")?;
        let blob = p::sync_get(&sess)?;
        let mut st = p::SyncState::parse(&blob)?;
        let pending = self.account.lock().unwrap().pending.clone();
        let mut pushed = 0;
        for item in &pending {
            if st.mark_solved(item.id) {
                st.score = st.score.saturating_add(item.score);
                st.time = st.time.saturating_add(item.seconds);
                pushed += 1;
            }
        }
        if pushed > 0 {
            p::sync_put(&sess, &st.serialize())?;
        }
        let mut a = self.account.lock().unwrap();
        let done: BTreeSet<u32> = pending.iter().map(|i| i.id).collect();
        a.pending.retain(|i| !done.contains(&i.id));
        a.solved = st.solved_ids();
        let still_pending: Vec<u32> = a.pending.iter().map(|i| i.id).collect();
        a.solved.extend(still_pending);
        a.score = Some(st.score);
        a.last_sync = now();
        a.sync_error = None;
        self.save_account(&a);
        Ok(pushed)
    }

    pub fn sync(&self) -> Result<usize> {
        let _guard = self.sync_lock.lock().unwrap();
        let r = self.sync_inner();
        if let Err(e) = &r {
            let mut a = self.account.lock().unwrap();
            a.sync_error = Some(e.clone());
            self.save_account(&a);
        }
        r
    }

    pub fn mark_solved(self: &Arc<Self>, id: u32, seconds: i32) -> Result<bool> {
        let board = self.board(id)?;
        let cells = board.grid.iter().filter(|&&v| v != 0).count() as i32;
        {
            let mut a = self.account.lock().unwrap();
            if a.solved.contains(&id) {
                return Ok(false);
            }
            a.solved.insert(id);
            a.pending.push(Pending {
                id,
                score: cells,
                seconds,
            });
            self.save_account(&a);
        }
        if self.session.lock().unwrap().is_some() {
            let me = self.clone();
            std::thread::spawn(move || {
                let _ = me.sync();
            });
        }
        Ok(true)
    }

    // ------------------------------------------------------------ catalog

    pub fn load_catalog(&self, refresh: bool) {
        {
            let mut c = self.catalog.write().unwrap();
            if c.loading {
                return;
            }
            c.loading = true;
        }
        let path = self.dir.join("catalog.bin");
        let result = (|| -> Result<(HashMap<u32, Puzzle>, f64)> {
            let stale = fs::metadata(&path)
                .and_then(|m| m.modified())
                .map(|t| t.elapsed().unwrap_or_default() > CATALOG_MAX_AGE)
                .unwrap_or(true);
            if refresh || stale {
                match p::fetch_catalog_blob().and_then(|b| p::parse_catalog(&b).map(|_| b)) {
                    Ok(blob) => write_atomic(&path, &blob)?,
                    // offline: fall back to the cached copy if there is one
                    Err(e) if path.exists() => eprintln!("catalog refresh failed: {e}"),
                    Err(e) => return Err(e),
                }
            }
            let blob = fs::read(&path).map_err(|e| e.to_string())?;
            let mtime = fs::metadata(&path)
                .and_then(|m| m.modified())
                .map(|t| {
                    t.duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs_f64()
                })
                .unwrap_or(0.0);
            Ok((p::parse_catalog(&blob)?, mtime))
        })();
        let mut c = self.catalog.write().unwrap();
        c.loading = false;
        match result {
            Ok((by_id, time)) => {
                let mut list: Vec<u32> = by_id.keys().copied().collect();
                list.sort_unstable_by(|a, b| b.cmp(a));
                c.by_id = by_id;
                c.list = list;
                c.time = time;
                c.error = None;
            }
            Err(e) => c.error = Some(e),
        }
    }

    pub fn search(&self, q: &Value) -> Value {
        let s = |k: &str| q.get(k).and_then(Value::as_str).unwrap_or("").to_string();
        let n = |k: &str, d: f64| {
            q.get(k)
                .and_then(|v| v.as_f64().or_else(|| v.as_str()?.parse().ok()))
                .unwrap_or(d)
        };
        let text = s("q").trim().to_lowercase();
        let author = s("author").trim().to_lowercase();
        let color = s("color");
        let status = s("status");
        let sort = s("sort");
        let (min, max) = (n("min", 0.0) as u32, n("max", 999.0) as u32);
        let min_rating = n("rating", 0.0) as f32;
        let page = n("page", 0.0) as usize;
        let per = (n("per", 60.0) as usize).clamp(1, 200);

        let solved = self.account.lock().unwrap().solved.clone();
        let progress = self.progress.lock().unwrap().clone();
        let cat = self.catalog.read().unwrap();

        let ids: Vec<u32> = if status == "started" {
            let mut v: Vec<(&u32, &ProgressInfo)> =
                progress.iter().filter(|(_, p)| !p.solved).collect();
            v.sort_by(|a, b| b.1.t.total_cmp(&a.1.t));
            v.into_iter()
                .map(|(id, _)| *id)
                .filter(|id| cat.by_id.contains_key(id))
                .collect()
        } else {
            cat.list.clone()
        };
        let mut out: Vec<&Puzzle> = ids
            .iter()
            .filter_map(|id| cat.by_id.get(id))
            .filter(|p| match &color[..] {
                "bw" => !p.color,
                "color" => p.color,
                _ => true,
            })
            .filter(|p| {
                let big = p.w.max(p.h);
                big >= min && big <= max && p.rating >= min_rating
            })
            .filter(|p| match &status[..] {
                "solved" => solved.contains(&p.id),
                "unsolved" => !solved.contains(&p.id),
                _ => true,
            })
            .filter(|p| {
                text.is_empty()
                    || p.title.to_lowercase().contains(&text)
                    || p.title2.to_lowercase().contains(&text)
                    || text.trim_start_matches('#') == p.id.to_string()
            })
            .filter(|p| author.is_empty() || p.author.to_lowercase() == author)
            .collect();
        if status != "started" {
            let key = |p: &Puzzle| -> f32 {
                match &sort[..] {
                    "rating" => -p.rating,
                    "fans" => -p.fans,
                    "difficulty" => -p.difficulty,
                    "easy" => {
                        if p.difficulty > 0.0 {
                            p.difficulty
                        } else {
                            9.0
                        }
                    }
                    "big" => -((p.w * p.h) as f32),
                    "small" => (p.w * p.h) as f32,
                    "oldest" => p.id as f32,
                    _ => -(p.id as f32),
                }
            };
            out.sort_by(|a, b| key(a).total_cmp(&key(b)));
        }
        let total = out.len();
        let items: Vec<Value> = out
            .into_iter()
            .skip(page * per)
            .take(per)
            .map(|p| {
                let mut v = serde_json::to_value(p).unwrap();
                v["solved"] = json!(solved.contains(&p.id));
                v["progress"] = match progress.get(&p.id) {
                    Some(pr) if !pr.solved => json!(pr.pct),
                    _ => Value::Null,
                };
                v
            })
            .collect();
        json!({ "total": total, "page": page, "per": per, "items": items })
    }

    // ------------------------------------------------------------ puzzles

    pub fn puzzle_png(&self, id: u32) -> Result<Vec<u8>> {
        let path = self.dir.join("puzzles").join(format!("{id}.png"));
        if let Ok(b) = fs::read(&path) {
            return Ok(b);
        }
        let b = p::fetch_puzzle_png(id)?;
        write_atomic(&path, &b)?;
        Ok(b)
    }

    pub fn board(&self, id: u32) -> Result<p::Board> {
        let meta = self
            .catalog
            .read()
            .unwrap()
            .by_id
            .get(&id)
            .cloned()
            .ok_or("Unknown puzzle")?;
        let b = p::build_board(id, &self.puzzle_png(id)?, meta.color, meta.cbg)?;
        if b.w != meta.w || b.h != meta.h {
            return Err("Puzzle image does not match the catalog".into());
        }
        Ok(b)
    }

    pub fn puzzle(&self, id: u32) -> Result<Value> {
        let board = self.board(id)?;
        let mut meta = serde_json::to_value(self.catalog.read().unwrap().by_id.get(&id)).unwrap();
        meta["solved"] = json!(self.account.lock().unwrap().solved.contains(&id));
        Ok(json!({ "meta": meta, "puzzle": board, "progress": self.get_progress(id) }))
    }

    fn progress_path(&self, id: u32) -> PathBuf {
        self.dir.join("progress").join(format!("{id}.json"))
    }

    pub fn get_progress(&self, id: u32) -> Value {
        read_json(&self.progress_path(id)).unwrap_or(Value::Null)
    }

    pub fn save_progress(&self, id: u32, data: &Value) -> Result<()> {
        write_json(&self.progress_path(id), data)?;
        let mut idx = self.progress.lock().unwrap();
        idx.insert(
            id,
            ProgressInfo {
                pct: data.get("pct").and_then(Value::as_u64).unwrap_or(0) as u32,
                t: now(),
                solved: data.get("solved").and_then(Value::as_bool).unwrap_or(false),
            },
        );
        let as_str: HashMap<String, &ProgressInfo> =
            idx.iter().map(|(k, v)| (k.to_string(), v)).collect();
        write_json(&self.dir.join("progress.json"), &as_str)
    }

    pub fn delete_progress(&self, id: u32) -> Result<()> {
        let mut idx = self.progress.lock().unwrap();
        idx.remove(&id);
        match fs::remove_file(self.progress_path(id)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.to_string()),
            _ => {}
        }
        let as_str: HashMap<String, &ProgressInfo> =
            idx.iter().map(|(k, v)| (k.to_string(), v)).collect();
        write_json(&self.dir.join("progress.json"), &as_str)
    }

    /// Thumbnail of the player's current board (never reveals the solution).
    pub fn progress_thumb(&self, id: u32) -> Result<Vec<u8>> {
        let board = self.board(id)?;
        let prog = self.get_progress(id);
        let cells: Vec<i64> = prog
            .get("cells")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(|v| v.as_i64().unwrap_or(0)).collect())
            .unwrap_or_default();
        let hex = |s: &str| -> [u8; 3] {
            let n = u32::from_str_radix(s.trim_start_matches('#'), 16).unwrap_or(0);
            [(n >> 16) as u8, (n >> 8) as u8, n as u8]
        };
        let pal: Vec<[u8; 3]> = board.palette.iter().map(|c| hex(c)).collect();
        let mut rgba = Vec::with_capacity((board.w * board.h * 4) as usize);
        for i in 0..(board.w * board.h) as usize {
            let v = cells.get(i).copied().unwrap_or(0);
            let px = match v {
                v if v > 0 && (v as usize) < pal.len() => {
                    let c = if board.palette.len() == 2 {
                        [43, 38, 32]
                    } else {
                        pal[v as usize]
                    };
                    [c[0], c[1], c[2], 255]
                }
                -1 => [214, 206, 192, 255],                  // crossed
                _ => [pal[0][0], pal[0][1], pal[0][2], 255], // unknown: the puzzle background
            };
            rgba.extend_from_slice(&px);
        }
        p::encode_png(board.w, board.h, &rgba)
    }
}
