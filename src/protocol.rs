//! Nonograms Katana server protocol (reverse engineered from the GWT web client).
//!
//! All endpoints live under https://ucdevs.com/ujc/ and speak big-endian Java
//! DataOutputStream-style binary (ints, u16-length-prefixed UTF-8 strings).

use std::collections::{BTreeSet, HashMap};
use std::io::{Cursor, Read, Write};

use base64::Engine;
use flate2::read::{DeflateDecoder, GzDecoder};
use flate2::write::GzEncoder;
use serde::Serialize;
use sha2::{Digest, Sha256};

pub const BASE: &str = "https://ucdevs.com/ujc/";
const UA: &str = "Mozilla/5.0 (X11; Linux x86_64) katana-desktop";
const PASSWORD_SALT: &str = "unrXCLIgzba8BkfxKaSsPMypR6vpYcuO";

const LOGIN_MAGIC: u32 = 0x1A8D_19C2;
const LIST_MAGIC: u32 = 0x1A78_04B1;
const SYNC_MAGIC: u32 = 0x1A1C_C80B;
const SYNC_SCORE: u32 = 0x1A1E_1B00;
const SYNC_TIME: u32 = 0x1DAF_29CF;
/// The only numeric category in the sync blob: solved user-created puzzles.
pub const DWL_CAT: &str = "dwl:";

const LANGS: [&str; 21] = [
    "", "en", "ru", "ja", "de", "fr", "es", "nl", "ko", "zh", "it", "pl", "cs", "uk", "iw", "sv",
    "sk", "fi", "pt", "el", "nb",
];

pub type Result<T> = std::result::Result<T, String>;

fn err<T>(msg: impl Into<String>) -> Result<T> {
    Err(msg.into())
}

// ------------------------------------------------------------------ random

/// Non-cryptographic randomness is all the protocol needs (device id, sync generation id).
pub fn random_u64() -> u64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    let mut h = RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
    );
    h.finish()
}

pub fn new_device_id() -> String {
    format!("{:016x}{:016x}", random_u64(), random_u64())
}

// ------------------------------------------------------------------ transport

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(std::time::Duration::from_secs(180)))
        .build()
        .into()
}

fn finish(resp: ureq::http::Response<ureq::Body>) -> Result<Vec<u8>> {
    let status = resp.status();
    let ctype = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = resp
        .into_body()
        .into_with_config()
        .limit(64 << 20)
        .read_to_vec()
        .map_err(|e| e.to_string())?;
    if !status.is_success() {
        return err(format!("server error {status}"));
    }
    if ctype.starts_with("text/html") {
        // the server reports problems ("Incorrect email/password", ...) as plain html text
        let text = String::from_utf8_lossy(&body);
        let text = text.replace("<br>", "\n");
        return err(text.trim().to_string());
    }
    Ok(body)
}

fn post_form(endpoint: &str, params: &[(&str, &str)]) -> Result<Vec<u8>> {
    let resp = agent()
        .post(format!("{BASE}{endpoint}"))
        .header("User-Agent", UA)
        .send_form(params.iter().copied())
        .map_err(|e| format!("network: {e}"))?;
    finish(resp)
}

fn get(url: &str) -> Result<Vec<u8>> {
    let resp = agent()
        .get(url)
        .header("User-Agent", UA)
        .call()
        .map_err(|e| format!("network: {e}"))?;
    finish(resp)
}

// ------------------------------------------------------------------ binary reader

struct Reader<'a> {
    b: &'a [u8],
    p: usize,
}

impl<'a> Reader<'a> {
    fn new(b: &'a [u8]) -> Self {
        Reader { b, p: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.p + n > self.b.len() {
            return err("unexpected end of data");
        }
        let s = &self.b[self.p..self.p + n];
        self.p += n;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16> {
        let s = self.take(2)?;
        Ok(u16::from_be_bytes([s[0], s[1]]))
    }
    fn u32(&mut self) -> Result<u32> {
        let s = self.take(4)?;
        Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn i32(&mut self) -> Result<i32> {
        Ok(self.u32()? as i32)
    }
    fn i64(&mut self) -> Result<i64> {
        let s = self.take(8)?;
        Ok(i64::from_be_bytes(s.try_into().unwrap()))
    }
    fn utf(&mut self) -> Result<String> {
        let n = self.u16()? as usize;
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
    fn str8(&mut self) -> Result<String> {
        let n = self.u8()? as usize;
        Ok(String::from_utf8_lossy(self.take(n)?).into_owned())
    }
    fn varid(&mut self) -> Result<u32> {
        let v = self.u16()? as u32;
        if v & 0x8000 != 0 {
            Ok((v & 0x7FFF) << 8 | self.u8()? as u32)
        } else {
            Ok(v)
        }
    }
}

fn put_utf(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u16).to_be_bytes());
    out.extend_from_slice(s.as_bytes());
}

// ------------------------------------------------------------------ auth

#[derive(Clone, Debug, Serialize, serde::Deserialize)]
pub struct Session {
    pub user_id: i32,
    pub token: String,
    pub nickname: String,
    pub device_id: String,
    #[serde(default)]
    pub email: String,
}

pub fn hash_password(password: &str) -> String {
    let d = Sha256::digest(format!("{PASSWORD_SALT}{password}").as_bytes());
    base64::engine::general_purpose::STANDARD.encode(d)
}

fn parse_login(body: &[u8]) -> Result<(i32, String, String)> {
    let mut r = Reader::new(body);
    if r.u32()? != LOGIN_MAGIC {
        return err("unexpected login response");
    }
    let status = r.u32()?;
    if status & 0xFFFF != 0 {
        return err(format!("login refused (code {})", status & 0xFFFF));
    }
    Ok((r.i32()?, r.utf()?, r.utf()?))
}

pub fn login(email: &str, password: &str, device_id: &str) -> Result<Session> {
    let email = email.trim().to_lowercase();
    let pw = hash_password(password);
    let body = post_form(
        "login.php",
        &[
            ("op", "login"),
            ("idn1", &email),
            ("idn2", &pw),
            ("uid", device_id),
            ("lang", "en"),
            ("gwt", "1"),
        ],
    )?;
    let (user_id, token, nickname) = parse_login(&body)?;
    Ok(Session {
        user_id,
        token,
        nickname,
        device_id: device_id.into(),
        email,
    })
}

pub fn register(email: &str, password: &str, nickname: &str, device_id: &str) -> Result<Session> {
    let email = email.trim().to_lowercase();
    let pw = hash_password(password);
    let body = post_form(
        "login.php",
        &[
            ("op", "register"),
            ("idn1", &email),
            ("idn2", &pw),
            ("uid", device_id),
            ("lang", "en"),
            ("gwt", "1"),
            ("nickname", nickname),
        ],
    )?;
    let (user_id, token, nick) = parse_login(&body)?;
    Ok(Session {
        user_id,
        token,
        nickname: nick,
        device_id: device_id.into(),
        email,
    })
}

pub fn logout(s: &Session) -> Result<()> {
    let uid = s.user_id.to_string();
    post_form(
        "login.php",
        &[
            ("op", "logout"),
            ("idn1", &uid),
            ("idn2", &s.token),
            ("gwt", "1"),
        ],
    )
    .map(|_| ())
}

/// Same rules as the official client.
pub fn validate_signup(email: &str, password: &str, nickname: Option<&str>) -> Result<()> {
    let e = email.trim();
    if e.len() < 5 || !e.contains('@') || !e.contains('.') {
        return err("Please enter a valid email address");
    }
    if password.chars().count() < 5 {
        return err("The password must be at least 5 characters");
    }
    if let Some(n) = nickname {
        let ok_chars = n
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || " _-+&?!():,.'\"".contains(c));
        if n.len() < 3 || n.len() > 32 || !ok_chars {
            return err("Nickname: 3–32 latin letters, digits or  _-+&?!():,.'\"");
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ catalog

#[derive(Clone, Debug, Serialize)]
pub struct Puzzle {
    pub id: u32,
    pub approved: bool,
    pub color: bool,
    /// colour puzzle whose background is not white (transparent pixels mark it)
    pub cbg: bool,
    pub w: u32,
    pub h: u32,
    pub rating: f32,
    pub difficulty: f32,
    pub fans: f32,
    pub time: u32,
    pub title: String,
    pub title2: String,
    pub lang: String,
    pub author: String,
    pub cats: Vec<u8>,
}

pub fn fetch_catalog_blob() -> Result<Vec<u8>> {
    get(&format!("{BASE}getlist10.php?aid=0&ts=0&gwt=1"))
}

fn signed(v: u32, bits: u32) -> i32 {
    if v & (1 << (bits - 1)) != 0 {
        v as i32 - (1 << bits)
    } else {
        v as i32
    }
}

fn read_ratings(r: &mut Reader, v10: bool) -> Result<(i32, i32, i32)> {
    if v10 {
        let o = r.u32()?;
        Ok((
            signed(o & 0xFFFF, 16),
            signed(o >> 16 & 255, 8),
            signed(o >> 24 & 255, 8),
        ))
    } else {
        Ok((signed(r.u16()? as u32, 16), signed(r.u8()? as u32, 8), 0))
    }
}

fn set_ratings(p: &mut Puzzle, r: (i32, i32, i32)) {
    p.rating = (r.0 & 0xFFFF) as f32 / 13107.0; // "shuriken": quality 0..5
    p.difficulty = (r.1 & 255) as f32 / 51.0; // "katana": difficulty 0..5
    p.fans = (r.2 & 255) as f32 / 51.0; // "fan": popularity 0..5
}

/// Mirror of the client's w9e(): normalise the (up to two) category ids.
fn cats(e: u8, f: u8) -> Vec<u8> {
    let e = if e > 50 { 0 } else { e };
    let f = if f > 50 || f == e || e == 0 { 0 } else { f };
    [e, f].into_iter().filter(|&c| c != 0).collect()
}

fn read_cats(r: &mut Reader) -> Result<(u8, u8)> {
    let mut e = r.u8()?;
    let mut f = 0;
    if e & 128 != 0 {
        e &= 127;
        f = r.u8()?;
    }
    Ok((e, f))
}

fn lang_code(l: u8, r: &mut Reader) -> Result<String> {
    if l >= 32 {
        let m = r.u8()?;
        return Ok(String::from_utf8_lossy(&[l, m]).into_owned());
    }
    Ok(LANGS.get(l as usize).copied().unwrap_or("").to_string())
}

/// Parse a getlist10 response. Mirrors the client's k6e().
pub fn parse_catalog(blob: &[u8]) -> Result<HashMap<u32, Puzzle>> {
    let mut r = Reader::new(blob);
    if r.u32()? != LIST_MAGIC {
        return err("bad catalog data");
    }
    let m = r.u32()?;
    let ver = m & 0xFFFF;
    if !(9..=10).contains(&ver) {
        return err(format!("unsupported catalog version {ver}"));
    }
    let v10 = ver >= 10;
    if m & 0x20000 != 0 {
        r.u32()?;
    }
    let mut puzzles = HashMap::new();
    loop {
        let tag = r.u32()?;
        if tag == 0 {
            break;
        }
        if tag != LIST_MAGIC {
            return err("bad catalog packet");
        }
        let f = r.u32()?;
        let kind = f >> 24 & 15;
        let size = if kind != 4 { r.u32()? as usize } else { 0 };
        if f & 0x10000 != 0 {
            let mut d = Vec::new();
            DeflateDecoder::new(r.take(size)?)
                .read_to_end(&mut d)
                .map_err(|e| e.to_string())?;
            parse_packet(&mut Reader::new(&d), kind, f, &mut puzzles, v10)?;
        } else {
            // uncompressed packets are read straight from the stream
            parse_packet(&mut r, kind, f, &mut puzzles, v10)?;
        }
        if kind == 4 {
            break;
        }
        r.take(16)?; // md5 of the packet
    }
    Ok(puzzles)
}

fn parse_packet(
    q: &mut Reader,
    kind: u32,
    f: u32,
    puzzles: &mut HashMap<u32, Puzzle>,
    v10: bool,
) -> Result<()> {
    match kind {
        1 => {
            q.u32()?; // aid
            q.u32()?; // ts
            parse_full(q, puzzles, v10)?;
        }
        4 => parse_full(q, puzzles, v10)?,
        2 => {
            q.u32()?;
            parse_delta(q, puzzles, v10)?;
        }
        3 => {
            q.u32()?;
            parse_ratings(q, puzzles, v10, f & 1 != 0)?;
        }
        5 => {
            // featured lists: 4 x (count, ids)
            for _ in 0..4 {
                for _ in 0..q.u8()? {
                    q.varid()?;
                }
            }
        }
        _ => return err(format!("unknown catalog packet type {kind}")),
    }
    Ok(())
}

fn parse_full(q: &mut Reader, puzzles: &mut HashMap<u32, Puzzle>, v10: bool) -> Result<()> {
    let n = q.u32()?;
    let mut id: u32 = 0;
    let mut t: u32 = 0;
    for _ in 0..n {
        let yb = q.u8()?;
        match yb & 3 {
            0 => id += 1,
            1 => id += 2,
            2 => id += q.u8()? as u32,
            _ => id = q.varid()?,
        }
        let p = q.u32()?;
        let zb = p & 255;
        let c = p >> 8 & 255;
        let gb = c & 48;
        let ratings = read_ratings(q, v10)?;
        match gb {
            0 => t = q.u32()?,
            16 => {
                t = t.wrapping_add(q.u8()? as u32 | (q.u8()? as u32) << 8 | (q.u8()? as u32) << 16)
            }
            32 => t = t.wrapping_add(q.u16()? as u32),
            _ => {
                let d = q.u8()? as u32 | (q.u8()? as u32) << 8 | (q.u8()? as u32) << 16;
                t = t.wrapping_add(d).wrapping_sub(0xFF_FFFF);
            }
        }
        let title = if yb & 4 != 0 {
            let rid = q.varid()?;
            puzzles
                .get(&rid)
                .map(|p| p.title.clone())
                .unwrap_or_default()
        } else {
            q.str8()?
        };
        let lang = if yb & 8 != 0 {
            let l = q.u8()?;
            lang_code(l, q)?
        } else {
            String::new()
        };
        let title2 = if yb & 16 != 0 {
            if yb & 32 != 0 {
                let rid = q.varid()?;
                puzzles
                    .get(&rid)
                    .map(|p| p.title2.clone())
                    .unwrap_or_default()
            } else {
                q.str8()?
            }
        } else {
            String::new()
        };
        let author = if yb & 64 != 0 {
            let rid = q.varid()?;
            puzzles
                .get(&rid)
                .map(|p| p.author.clone())
                .unwrap_or_default()
        } else {
            q.str8()?
        };
        let (e, fc) = if yb & 128 != 0 { read_cats(q)? } else { (0, 0) };
        let mut pz = Puzzle {
            id,
            approved: zb & 1 != 0,
            color: p >> 30 & 1 == 1,
            cbg: c & 64 != 0,
            w: p >> 16 & 127,
            h: p >> 23 & 127,
            rating: 0.0,
            difficulty: 0.0,
            fans: 0.0,
            time: t & !15,
            title,
            title2,
            lang,
            author,
            cats: cats(e, fc),
        };
        set_ratings(&mut pz, ratings);
        puzzles.insert(id, pz);
    }
    Ok(())
}

fn parse_delta(q: &mut Reader, puzzles: &mut HashMap<u32, Puzzle>, v10: bool) -> Result<()> {
    let n = q.u32()?;
    for _ in 0..n {
        let j = q.u8()?;
        let i = j & 63;
        let id = q.varid()?;
        let ratings = if matches!(i, 33 | 1 | 3 | 4) {
            read_ratings(q, v10)?
        } else {
            (0, 0, 0)
        };
        let (e, fc) = if matches!(i, 34 | 1 | 3 | 4) && j & 128 != 0 {
            read_cats(q)?
        } else {
            (0, 0)
        };
        if i & 32 != 0 {
            match i {
                33 => {
                    let zb = q.u8()?;
                    let t = q.u32()? & !15;
                    if let Some(p) = puzzles.get_mut(&id) {
                        p.approved = zb & 1 != 0;
                        set_ratings(p, ratings);
                        p.time = t;
                    }
                }
                34 => {
                    if let Some(p) = puzzles.get_mut(&id) {
                        p.cats = cats(e, fc);
                    }
                }
                32 => {
                    puzzles.remove(&id);
                }
                _ => {}
            }
            continue;
        }
        let yb = q.u8()?;
        let mut full = None;
        if i != 2 {
            let color = yb & 1 == 1;
            let c = q.u8()?;
            let w = q.u8()? as u32;
            let h = q.u8()? as u32;
            let zb = q.u8()?;
            let t = q.u32()? & !15;
            let author = q.str8()?;
            full = Some((color, c, w, h, zb, t, author));
        }
        let title = q.str8()?;
        let mut lang = String::new();
        if yb & 64 != 0 {
            if yb & 128 != 0 {
                let l = q.u8()?;
                lang = LANGS.get(l as usize).copied().unwrap_or("").to_string();
            } else {
                lang = String::from_utf8_lossy(q.take(2)?).into_owned();
            }
        }
        let title2 = if yb & 32 != 0 {
            q.str8()?
        } else {
            String::new()
        };
        match full {
            None => {
                if let Some(p) = puzzles.get_mut(&id) {
                    p.title = title;
                    p.title2 = title2;
                    p.lang = lang;
                }
            }
            Some((color, c, w, h, zb, t, author)) => {
                let mut p = Puzzle {
                    id,
                    approved: zb & 1 != 0,
                    color,
                    cbg: c & 64 != 0,
                    w,
                    h,
                    rating: 0.0,
                    difficulty: 0.0,
                    fans: 0.0,
                    time: t,
                    title,
                    title2,
                    lang,
                    author,
                    cats: cats(e, fc),
                };
                set_ratings(&mut p, ratings);
                puzzles.insert(id, p);
            }
        }
    }
    Ok(())
}

fn parse_ratings(
    q: &mut Reader,
    puzzles: &mut HashMap<u32, Puzzle>,
    v10: bool,
    small: bool,
) -> Result<()> {
    let n = q.u32()?;
    let mut id = 0u32;
    for _ in 0..n {
        if small {
            let l = q.u8()?;
            id = if l < 255 { id + l as u32 } else { q.varid()? };
        } else {
            id = q.varid()?;
        }
        let (a, b, c) = read_ratings(q, v10)?;
        if let Some(p) = puzzles.get_mut(&id) {
            if a != 0 {
                p.rating = (a & 0xFFFF) as f32 / 13107.0;
            }
            if b != 0 {
                p.difficulty = (b & 255) as f32 / 51.0;
            }
            if c != 0 {
                p.fans = (c & 255) as f32 / 51.0;
            }
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ puzzles

pub fn puzzle_image_url(id: u32) -> String {
    let id = id as u64;
    let f = id * 7 + 1_048_583;
    let g = ((id & 4095) * 69069) & 255;
    format!("{BASE}puzw/user/{:x}.png", (f << 8 | g) & 0xFFFF_FFFF)
}

pub fn fetch_puzzle_png(id: u32) -> Result<Vec<u8>> {
    get(&puzzle_image_url(id))
}

/// Decode to (w, h, RGBA pixels).
pub fn decode_png(data: &[u8]) -> Result<(u32, u32, Vec<[u8; 4]>)> {
    let mut dec = png::Decoder::new(Cursor::new(data));
    dec.set_transformations(
        png::Transformations::normalize_to_color8() | png::Transformations::ALPHA,
    );
    let mut reader = dec.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("png too large")?];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let buf = &buf[..info.buffer_size()];
    let px = match info.color_type {
        png::ColorType::Rgba => buf
            .chunks_exact(4)
            .map(|c| [c[0], c[1], c[2], c[3]])
            .collect(),
        png::ColorType::GrayscaleAlpha => buf
            .chunks_exact(2)
            .map(|c| [c[0], c[0], c[0], c[1]])
            .collect(),
        png::ColorType::Rgb => buf
            .chunks_exact(3)
            .map(|c| [c[0], c[1], c[2], 255])
            .collect(),
        png::ColorType::Grayscale => buf.iter().map(|&c| [c, c, c, 255]).collect(),
        other => return err(format!("unsupported png colour type {other:?}")),
    };
    Ok((info.width, info.height, px))
}

pub fn encode_png(w: u32, h: u32, rgba: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header().map_err(|e| e.to_string())?;
        wr.write_image_data(rgba).map_err(|e| e.to_string())?;
    }
    Ok(out)
}

#[derive(Clone, Debug, Serialize)]
pub struct Board {
    pub id: u32,
    pub w: u32,
    pub h: u32,
    /// palette[0] is the background; grid values index into it (0 = empty)
    pub palette: Vec<String>,
    pub grid: Vec<u8>,
}

/// Turn a puzzle PNG into a solution grid. Mirrors the client's j4e()/n3e().
pub fn build_board(id: u32, png: &[u8], color: bool, colored_bg: bool) -> Result<Board> {
    let (w, h, px) = decode_png(png)?;
    if !color {
        // B/W: a cell is filled when its blue channel < 128
        let grid = px.iter().map(|p| (p[2] < 128) as u8).collect();
        return Ok(Board {
            id,
            w,
            h,
            palette: vec!["#ffffff".into(), "#000000".into()],
            grid,
        });
    }
    let mut bg = [255u8, 255, 255];
    if colored_bg
        && let Some(p) = px.iter().find(|p| p[3] <= 192) {
            bg = [p[0], p[1], p[2]];
            if bg.iter().all(|&c| c >= 248) {
                bg = [255, 255, 255];
            }
        }
    let mut colors = vec![bg];
    let mut grid = Vec::with_capacity(px.len());
    for p in &px {
        if colored_bg && p[3] <= 192 {
            grid.push(0);
            continue;
        }
        let c = [p[0], p[1], p[2]];
        let idx = match colors.iter().position(|&x| x == c) {
            Some(i) => i,
            None => {
                colors.push(c);
                colors.len() - 1
            }
        };
        grid.push(idx.min(255) as u8);
    }
    let palette = colors
        .iter()
        .map(|c| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]))
        .collect();
    Ok(Board {
        id,
        w,
        h,
        palette,
        grid,
    })
}

// ------------------------------------------------------------------ sync

pub fn sync_get(s: &Session) -> Result<Vec<u8>> {
    let uid = s.user_id.to_string();
    post_form(
        "syncget.php",
        &[
            ("idn1", &uid),
            ("idn2", &s.token),
            ("uid", &s.device_id),
            ("gwt", "1"),
        ],
    )
}

pub fn sync_put(s: &Session, blob: &[u8]) -> Result<()> {
    let sgi = ((random_u64() % 0x7FFF_FFFE) + 1).to_string();
    let fields = [
        ("idn1", s.user_id.to_string()),
        ("idn2", s.token.clone()),
        ("uid", s.device_id.clone()),
        ("gwt", "1".into()),
        ("fsz", blob.len().to_string()),
        ("sgi", sgi),
    ];
    let mut boundary = String::from("xLBNqaM8vfLOaDTI");
    while blob
        .windows(boundary.len())
        .any(|w| w == boundary.as_bytes())
    {
        boundary.push((b'A' + (random_u64() % 26) as u8) as char);
    }
    let mut body = Vec::new();
    for (k, v) in &fields {
        write!(
            body,
            "--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\nContent-Type: text/plain; charset=UTF-8\r\n\r\n{v}\r\n"
        )
        .unwrap();
    }
    write!(
        body,
        "--{boundary}\r\nContent-Disposition: form-data; name=\"blob\"; filename=\"1\"\r\nContent-Type: application/octet-stream\r\nContent-Transfer-Encoding: binary\r\n\r\n"
    )
    .unwrap();
    body.extend_from_slice(blob);
    write!(body, "\r\n--{boundary}--\r\n").unwrap();
    let resp = agent()
        .post(format!("{BASE}syncput.php"))
        .header("User-Agent", UA)
        .header(
            "Content-Type",
            format!("multipart/form-data; boundary={boundary}"),
        )
        .send(&body[..])
        .map_err(|e| format!("network: {e}"))?;
    finish(resp).map(|_| ())
}

#[derive(Clone, Debug)]
pub enum CatItems {
    Ids(BTreeSet<u32>),
    Names(Vec<String>),
}

#[derive(Clone, Debug)]
pub struct Category {
    pub name: String,
    pub flag: i32,
    pub reserved: i32,
    pub items: CatItems,
}

/// The account's progress blob. Categories are kept in order and every section we
/// don't edit is preserved verbatim.
#[derive(Clone, Debug)]
pub struct SyncState {
    pub flags: u32,
    pub cats: Vec<Category>,
    pub score: i32,
    pub score_reserved: i32,
    /// sections between score and time (favourite authors, lists...), verbatim
    pub middle: Vec<u8>,
    pub time: i32,
    pub time_extra: i64,
    pub tail: Vec<u8>,
}

impl SyncState {
    pub fn parse(blob: &[u8]) -> Result<SyncState> {
        let mut s = SyncState {
            flags: 3,
            cats: vec![],
            score: 0,
            score_reserved: 0,
            middle: vec![],
            time: 0,
            time_extra: 0,
            tail: 0i32.to_be_bytes().to_vec(),
        };
        if blob.is_empty() || blob == [0] {
            return Ok(s); // fresh account
        }
        let mut r = Reader::new(blob);
        if r.u32()? != SYNC_MAGIC {
            return err("bad sync data");
        }
        s.flags = r.u32()?;
        if s.flags & 0xFFFF != 3 {
            return err(format!("unsupported sync version {}", s.flags & 0xFFFF));
        }
        let mut body = Vec::new();
        GzDecoder::new(&blob[8..])
            .read_to_end(&mut body)
            .map_err(|e| e.to_string())?;
        let mut q = Reader::new(&body);
        for _ in 0..q.u32()? {
            let name = q.utf()?;
            let flag = q.i32()?;
            let reserved = q.i32()?;
            let n = q.u32()?;
            let items = if name == DWL_CAT {
                let mut ids = BTreeSet::new();
                let mut v: u32 = 0;
                for _ in 0..n {
                    let b = q.u8()?;
                    if b & 128 == 0 {
                        v += b as u32 + 1;
                    } else {
                        v = ((b & 127) as u32) << 16 | q.u16()? as u32;
                    }
                    ids.insert(v);
                }
                CatItems::Ids(ids)
            } else {
                CatItems::Names((0..n).map(|_| q.utf()).collect::<Result<_>>()?)
            };
            s.cats.push(Category {
                name,
                flag,
                reserved,
                items,
            });
        }
        let start;
        if q.u32()? == SYNC_SCORE {
            s.score_reserved = q.i32()?;
            s.score = q.i32()?;
            start = q.p;
        } else {
            start = q.p - 4;
        }
        // the time section is always last: tag, int, long, int 0
        let tag = SYNC_TIME.to_be_bytes();
        let tail_len = 4 + 4 + 8 + 4;
        if body.len() >= start + tail_len && body[body.len() - tail_len..][..4] == tag {
            let idx = body.len() - tail_len;
            s.middle = body[start..idx].to_vec();
            let mut t = Reader::new(&body[idx + 4..]);
            s.time = t.i32()?;
            s.time_extra = t.i64()?;
            s.tail = body[idx + 16..].to_vec();
        } else {
            s.middle = body[start..].to_vec();
            s.tail.clear();
        }
        Ok(s)
    }

    pub fn body(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(self.cats.len() as u32).to_be_bytes());
        for c in &self.cats {
            put_utf(&mut out, &c.name);
            out.extend_from_slice(&c.flag.to_be_bytes());
            out.extend_from_slice(&c.reserved.to_be_bytes());
            match &c.items {
                CatItems::Ids(ids) => {
                    out.extend_from_slice(&(ids.len() as u32).to_be_bytes());
                    let mut prev = 0u32;
                    for &v in ids {
                        let d = v.wrapping_sub(prev);
                        if (1..=128).contains(&d) {
                            out.push((d - 1) as u8);
                        } else {
                            out.push((v >> 16 | 128) as u8);
                            out.extend_from_slice(&((v & 0xFFFF) as u16).to_be_bytes());
                        }
                        prev = v;
                    }
                }
                CatItems::Names(names) => {
                    out.extend_from_slice(&(names.len() as u32).to_be_bytes());
                    for n in names {
                        put_utf(&mut out, n);
                    }
                }
            }
        }
        out.extend_from_slice(&SYNC_SCORE.to_be_bytes());
        out.extend_from_slice(&self.score_reserved.to_be_bytes());
        out.extend_from_slice(&self.score.to_be_bytes());
        out.extend_from_slice(&self.middle);
        out.extend_from_slice(&SYNC_TIME.to_be_bytes());
        out.extend_from_slice(&self.time.to_be_bytes());
        out.extend_from_slice(&self.time_extra.to_be_bytes());
        if self.tail.is_empty() {
            out.extend_from_slice(&0i32.to_be_bytes());
        } else {
            out.extend_from_slice(&self.tail);
        }
        out
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&SYNC_MAGIC.to_be_bytes());
        out.extend_from_slice(&self.flags.to_be_bytes());
        let mut gz = GzEncoder::new(out, flate2::Compression::default());
        gz.write_all(&self.body()).unwrap();
        gz.finish().unwrap()
    }

    fn dwl(&mut self) -> &mut BTreeSet<u32> {
        let pos = match self.cats.iter().position(|c| c.name == DWL_CAT) {
            Some(p) => p,
            None => {
                self.cats.push(Category {
                    name: DWL_CAT.into(),
                    flag: 0,
                    reserved: 0,
                    items: CatItems::Ids(BTreeSet::new()),
                });
                self.cats.len() - 1
            }
        };
        match &mut self.cats[pos].items {
            CatItems::Ids(ids) => ids,
            CatItems::Names(_) => unreachable!("dwl: is always parsed as ids"),
        }
    }

    pub fn solved_ids(&self) -> BTreeSet<u32> {
        self.cats
            .iter()
            .find_map(|c| match (&c.name[..], &c.items) {
                (DWL_CAT, CatItems::Ids(ids)) => Some(ids.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    pub fn mark_solved(&mut self, id: u32) -> bool {
        self.dwl().insert(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_url_matches_client() {
        assert!(puzzle_image_url(1).ends_with("/10000ecd.png"));
        assert!(puzzle_image_url(100).ends_with("/1002c314.png"));
        assert!(puzzle_image_url(50000).ends_with("/15573710.png"));
    }

    #[test]
    fn delta_ids_roundtrip() {
        let mut s = SyncState::parse(&[]).unwrap();
        for id in [5, 6, 140, 70000, 251567] {
            s.mark_solved(id);
        }
        let back = SyncState::parse(&s.serialize()).unwrap();
        assert_eq!(back.solved_ids(), s.solved_ids());
        assert_eq!(back.body(), s.body());
    }

    /// Optional: real samples captured from the server (set KATANA_SAMPLES to a dir
    /// containing catalog.bin and sync.bin).
    #[test]
    fn real_samples() {
        let Ok(dir) = std::env::var("KATANA_SAMPLES") else {
            return;
        };
        let dir = std::path::Path::new(&dir);
        let cat = parse_catalog(&std::fs::read(dir.join("catalog.bin")).unwrap()).unwrap();
        assert!(cat.len() > 200_000);
        assert_eq!(cat[&1].title, "Pigeon");
        let blob = std::fs::read(dir.join("sync.bin")).unwrap();
        let s = SyncState::parse(&blob).unwrap();
        let mut orig = Vec::new();
        GzDecoder::new(&blob[8..]).read_to_end(&mut orig).unwrap();
        assert_eq!(s.body(), orig, "sync blob must round-trip byte for byte");
    }
}
