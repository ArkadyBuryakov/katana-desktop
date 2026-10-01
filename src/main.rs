//! Katana Desktop: a native client for Nonograms Katana user-created puzzles.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod protocol;
mod store;

use std::borrow::Cow;
use std::sync::Arc;

use serde_json::{Value, json};
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy};
use tao::window::WindowBuilder;
use wry::WebViewBuilder;
use wry::http::{Request, Response, header::CONTENT_TYPE};

use store::Store;

const SCHEME: &str = "katana";
/// Where the custom scheme is served: WebView2 (Windows) maps custom schemes to
/// `https://<scheme>.localhost`, WebKit (macOS/Linux) uses `<scheme>://localhost`.
#[cfg(target_os = "windows")]
const START_URL: &str = "https://katana.localhost/";
#[cfg(not(target_os = "windows"))]
const START_URL: &str = "katana://localhost/";

fn is_app_url(url: &str) -> bool {
    url.starts_with(START_URL) || url.starts_with("about:")
}
const INDEX: &[u8] = include_bytes!("../web/index.html");
const APP_JS: &[u8] = include_bytes!("../web/app.js");
const STYLE: &[u8] = include_bytes!("../web/style.css");
const ICON: &[u8] = include_bytes!("../web/icon.png");

enum UserEvent {
    /// (request id, Ok(json) | Err(message))
    Reply(u64, Result<Value, String>),
}

fn respond(status: u16, ctype: &str, body: Vec<u8>) -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, ctype)
        .header("Cache-Control", "no-cache")
        .body(Cow::Owned(body))
        .unwrap()
}

/// Static assets and images over the custom `katana://` scheme.
fn serve(store: &Store, req: &Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
    let path = req.uri().path();
    let png = |r: Result<Vec<u8>, String>| match r {
        Ok(b) => respond(200, "image/png", b),
        Err(e) => respond(404, "text/plain", e.into_bytes()),
    };
    let id_of = |prefix: &str| {
        path.strip_prefix(prefix)?
            .strip_suffix(".png")?
            .parse::<u32>()
            .ok()
    };
    match path {
        "/" | "/index.html" => respond(200, "text/html; charset=utf-8", INDEX.to_vec()),
        "/app.js" => respond(200, "text/javascript; charset=utf-8", APP_JS.to_vec()),
        "/style.css" => respond(200, "text/css; charset=utf-8", STYLE.to_vec()),
        "/icon.png" => respond(200, "image/png", ICON.to_vec()),
        _ => {
            if let Some(id) = id_of("/image/") {
                // the solution picture: the UI only asks for it for solved puzzles
                png(store.puzzle_png(id))
            } else if let Some(id) = id_of("/thumb/") {
                png(store.progress_thumb(id))
            } else {
                respond(404, "text/plain", b"not found".to_vec())
            }
        }
    }
}

fn arg_u32(args: &Value, k: &str) -> Result<u32, String> {
    args.get(k)
        .and_then(Value::as_u64)
        .map(|v| v as u32)
        .ok_or_else(|| format!("missing {k}"))
}
fn arg_str<'a>(args: &'a Value, k: &str) -> &'a str {
    args.get(k).and_then(Value::as_str).unwrap_or("")
}

/// Commands sent from the UI with `window.ipc.postMessage`.
fn dispatch(store: &Arc<Store>, cmd: &str, args: &Value) -> Result<Value, String> {
    match cmd {
        "status" => Ok(store.status()),
        "login" => store.login(arg_str(args, "email"), arg_str(args, "password")),
        "register" => store.register(
            arg_str(args, "email"),
            arg_str(args, "password"),
            arg_str(args, "nickname"),
        ),
        "logout" => store.logout(args.get("force").and_then(Value::as_bool).unwrap_or(false)),
        "sync" => store.sync().map(|n| json!({ "pushed": n })),
        "catalog" => Ok(store.search(args)),
        "refreshCatalog" => {
            let s = store.clone();
            std::thread::spawn(move || s.load_catalog(true));
            Ok(json!(true))
        }
        "puzzle" => store.puzzle(arg_u32(args, "id")?),
        "saveProgress" => store
            .save_progress(arg_u32(args, "id")?, &args["data"])
            .map(|_| json!(true)),
        "solved" => {
            let secs = args.get("seconds").and_then(Value::as_i64).unwrap_or(0) as i32;
            store
                .mark_solved(arg_u32(args, "id")?, secs)
                .map(|new| json!({ "new": new }))
        }
        "openExternal" => {
            open_external(arg_str(args, "url"));
            Ok(json!(true))
        }
        _ => Err(format!("unknown command {cmd}")),
    }
}

fn open_external(url: &str) {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return;
    }
    #[cfg(target_os = "macos")]
    let opener = "open";
    // explorer hands URLs to the default browser without cmd.exe quoting pitfalls
    #[cfg(target_os = "windows")]
    let opener = "explorer";
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let opener = "xdg-open";
    let _ = std::process::Command::new(opener).arg(url).spawn();
}

fn handle_ipc(store: &Arc<Store>, proxy: &EventLoopProxy<UserEvent>, msg: &str) {
    let Ok(v) = serde_json::from_str::<Value>(msg) else {
        return;
    };
    let id = v.get("id").and_then(Value::as_u64).unwrap_or(0);
    let cmd = v
        .get("cmd")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    #[cfg(debug_assertions)]
    eprintln!("ipc: {cmd}");
    let args = v.get("args").cloned().unwrap_or(Value::Null);
    let store = store.clone();
    let proxy = proxy.clone();
    // network and disk work happens off the UI thread
    std::thread::spawn(move || {
        let r = dispatch(&store, &cmd, &args);
        let _ = proxy.send_event(UserEvent::Reply(id, r));
    });
}

/// Debug builds only: serve the UI over plain HTTP so it can be driven by a test browser.
/// `KATANA_DEV_HTTP=8766 cargo run`; the page's IPC is shimmed to `POST /ipc`.
#[cfg(debug_assertions)]
fn dev_http(store: Arc<Store>, port: u16) {
    use std::io::{BufRead, BufReader, Read, Write};
    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).expect("dev http bind");
    eprintln!("dev http on http://127.0.0.1:{port}/");
    for conn in listener.incoming().flatten() {
        let store = store.clone();
        std::thread::spawn(move || {
            let mut rd = BufReader::new(conn.try_clone().unwrap());
            let mut line = String::new();
            if rd.read_line(&mut line).is_err() {
                return;
            }
            let mut parts = line.split_whitespace();
            let (method, target) = (
                parts.next().unwrap_or(""),
                parts.next().unwrap_or("/").to_string(),
            );
            let mut len = 0;
            loop {
                let mut h = String::new();
                if rd.read_line(&mut h).is_err() || h.trim().is_empty() {
                    break;
                }
                if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0; len];
            let _ = rd.read_exact(&mut body);
            let (status, ctype, out): (u16, String, Vec<u8>) =
                if method == "POST" && target == "/ipc" {
                    let v: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
                    let r = dispatch(&store, v["cmd"].as_str().unwrap_or(""), &v["args"]);
                    let payload = match r {
                        Ok(v) => json!({ "ok": true, "payload": v }),
                        Err(e) => json!({ "ok": false, "payload": e }),
                    };
                    (
                        200,
                        "application/json".into(),
                        payload.to_string().into_bytes(),
                    )
                } else {
                    let path = target.split('?').next().unwrap_or("/").to_string();
                    let req = Request::builder()
                        .uri(format!("{SCHEME}://localhost{path}"))
                        .body(Vec::new())
                        .unwrap();
                    let resp = serve(&store, &req);
                    let ctype = resp
                        .headers()
                        .get(CONTENT_TYPE)
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or("")
                        .to_string();
                    (resp.status().as_u16(), ctype, resp.body().to_vec())
                };
            let mut conn = conn;
            let _ = write!(
                conn,
                "HTTP/1.1 {status} OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                out.len()
            );
            let _ = conn.write_all(&out);
        });
    }
}

#[cfg(target_os = "macos")]
fn mac_menu() -> muda::Menu {
    use muda::{Menu, PredefinedMenuItem, Submenu};
    let menu = Menu::new();
    let app = Submenu::with_items(
        "Katana Desktop",
        true,
        &[
            &PredefinedMenuItem::about(None, None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::quit(None),
        ],
    )
    .unwrap();
    // without an Edit menu, Cmd+C/V/A don't work in WKWebView text fields
    let edit = Submenu::with_items(
        "Edit",
        true,
        &[
            &PredefinedMenuItem::undo(None),
            &PredefinedMenuItem::redo(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::cut(None),
            &PredefinedMenuItem::copy(None),
            &PredefinedMenuItem::paste(None),
            &PredefinedMenuItem::select_all(None),
        ],
    )
    .unwrap();
    menu.append_items(&[&app, &edit]).unwrap();
    menu.init_for_nsapp();
    menu
}

fn window_icon() -> Option<tao::window::Icon> {
    let (w, h, px) = protocol::decode_png(ICON).ok()?;
    tao::window::Icon::from_rgba(px.concat(), w, h).ok()
}

fn main() -> wry::Result<()> {
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

    #[cfg(debug_assertions)]
    if let Some(port) = std::env::var("KATANA_DEV_HTTP")
        .ok()
        .and_then(|p| p.parse().ok())
    {
        let s = store.clone();
        std::thread::spawn(move || dev_http(s, port));
        if std::env::var_os("KATANA_HEADLESS").is_some() {
            loop {
                std::thread::park();
            }
        }
    }

    let event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    #[cfg(target_os = "macos")]
    let _menu = mac_menu();
    let proxy = event_loop.create_proxy();
    let window = WindowBuilder::new()
        .with_title("Katana Desktop")
        .with_inner_size(tao::dpi::LogicalSize::new(1280.0, 860.0))
        .with_min_inner_size(tao::dpi::LogicalSize::new(640.0, 480.0))
        .with_window_icon(window_icon())
        .build(&event_loop)
        .expect("failed to create window");

    let serve_store = store.clone();
    let ipc_store = store.clone();
    // keep the webview's own storage (localStorage etc.) inside our data dir
    let mut web_context = wry::WebContext::new(Some(store.dir().join("webview")));
    let builder = WebViewBuilder::new_with_web_context(&mut web_context)
        .with_custom_protocol(SCHEME.into(), move |_id, req| serve(&serve_store, &req))
        .with_ipc_handler(move |req| handle_ipc(&ipc_store, &proxy, req.body()))
        .with_navigation_handler(|url| {
            if is_app_url(&url) {
                return true;
            }
            open_external(&url);
            false
        })
        .with_new_window_req_handler(|url, _| {
            open_external(&url);
            wry::NewWindowResponse::Deny
        })
        .with_devtools(cfg!(debug_assertions))
        .with_url(START_URL);
    #[cfg(target_os = "windows")]
    let builder = {
        use wry::WebViewBuilderExtWindows;
        // serve as https://katana.localhost and drop browser shortcuts (F5, Ctrl+P, Ctrl+F...)
        builder
            .with_https_scheme(true)
            .with_browser_accelerator_keys(false)
    };

    #[cfg(any(target_os = "windows", target_os = "macos"))]
    let webview = builder.build(&window)?;
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let webview = {
        use tao::platform::unix::WindowExtUnix;
        use wry::WebViewBuilderExtUnix;
        builder.build_gtk(window.default_vbox().unwrap())?
    };

    event_loop.run(move |event, _, control_flow| {
        *control_flow = ControlFlow::Wait;
        match event {
            Event::UserEvent(UserEvent::Reply(id, r)) => {
                let (ok, payload) = match r {
                    Ok(v) => (true, v),
                    Err(e) => (false, Value::String(e)),
                };
                let _ = webview.evaluate_script(&format!("window.__ipcReply({id},{ok},{payload})"));
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                // The page saves after every move, so there's nothing to flush. End the process
                // directly: tearing the webview down (or calling into it) from here can stall on
                // some WebView2 setups, which would leave a window that won't close. WebView2's
                // helper processes exit on their own once their host is gone.
                window.set_visible(false);
                std::process::exit(0);
            }
            _ => {}
        }
    });
}
