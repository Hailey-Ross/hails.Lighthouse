//! Server logs, for servers running on the same machine as Lighthouse.
//!
//! Each server can have a `log_dir` (its install or data folder). The scanner looks for log files
//! under it; the viewer shows the end of one file, optionally filtered. Files are chosen by their
//! position in the scan, never by a path from the browser, so nothing outside `log_dir` is readable.

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path as FsPath, PathBuf};
use std::time::UNIX_EPOCH;

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use lighthouse_core::{GameKind, ServerConfig};
use maud::{Markup, PreEscaped, html};
use serde::Deserialize;

use super::server_name;
use crate::auth::{CurrentUser, forbidden};
use crate::module::{Module, PermissionDef};
use crate::util::{fmt_ago, fmt_bytes, now_unix};
use crate::{AppState, layout};

const VIEW: &str = "logs.view";
/// How deep to look under `log_dir`, and how many entries to visit at most.
const MAX_DEPTH: usize = 5;
const MAX_VISITS: usize = 20_000;
const MAX_FILES: usize = 60;
/// Only the end of a file is read.
const TAIL_BYTES: u64 = 2 * 1024 * 1024;
const DEFAULT_LINES: usize = 300;
const MAX_LINES: usize = 5000;

/// Folders that are big and never hold logs.
const SKIP_DIRS: &[&str] = &[
    "Mods",
    "mods",
    "Saves",
    "saves",
    "Data",
    "steamapps",
    "Engine",
    "Content",
    "workshop",
    "db",
    "Saved/SaveGames",
];

pub struct Logs;

impl Module for Logs {
    fn id(&self) -> &'static str {
        "logs"
    }
    fn title(&self) -> &'static str {
        "Logs"
    }
    fn icon(&self) -> &'static str {
        "M6 3h9l4 4v14H6z M14 3v5h5 M9 12h7 M9 16h7"
    }
    fn view_permission(&self) -> &'static str {
        VIEW
    }
    fn permissions(&self) -> Vec<PermissionDef> {
        vec![PermissionDef {
            key: VIEW,
            label: "Read server log files",
        }]
    }
    fn routes(&self) -> Router<AppState> {
        Router::new()
            .route("/", get(list))
            .route("/{id}", get(files))
            .route("/{id}/view", get(view))
    }
}

// ---------- scanning ----------

#[derive(Debug, Clone)]
pub(crate) struct LogFile {
    pub path: PathBuf,
    /// Path relative to `log_dir`, for display.
    pub shown: String,
    pub size: u64,
    pub modified: i64,
}

/// Does this file name look like a log for `game`?
pub(crate) fn is_log_name(name: &str, game: GameKind) -> bool {
    let lower = name.to_ascii_lowercase();
    let generic =
        lower.ends_with(".log") || (lower.ends_with(".txt") && (lower.contains("log") || lower.contains("console")));
    let specific = if game.is(GameKind::SEVEN_DAYS_TO_DIE) {
        lower.starts_with("output_log") // output_log__2026-11-10__19-00-00.txt
    } else if game.is(GameKind::PROJECT_ZOMBOID) {
        lower.ends_with(".txt") && (lower.contains("debuglog") || lower.contains("chat") || lower.contains("user"))
    } else {
        false
    };
    generic || specific
}

/// Find log files under `root`, newest first.
pub(crate) fn scan(root: &FsPath, game: GameKind) -> Vec<LogFile> {
    let Ok(root) = root.canonicalize() else {
        return Vec::new();
    };
    let mut found = Vec::new();
    let mut stack = vec![(root.clone(), 0usize)];
    let mut visits = 0;
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            visits += 1;
            if visits > MAX_VISITS {
                break;
            }
            let Ok(meta) = entry.metadata() else { continue };
            let path = entry.path();
            // Don't follow links out of the folder.
            if entry.file_type().is_ok_and(|t| t.is_symlink()) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if meta.is_dir() {
                let rel = path
                    .strip_prefix(&root)
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                if depth < MAX_DEPTH && !SKIP_DIRS.iter().any(|s| *s == name || *s == rel) {
                    stack.push((path, depth + 1));
                }
            } else if meta.is_file() && is_log_name(&name, game) {
                let modified = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_secs() as i64);
                let shown = path
                    .strip_prefix(&root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                found.push(LogFile {
                    path,
                    shown,
                    size: meta.len(),
                    modified,
                });
            }
        }
    }
    found.sort_by(|a, b| b.modified.cmp(&a.modified).then_with(|| a.shown.cmp(&b.shown)));
    found.truncate(MAX_FILES);
    found
}

/// The last `lines` lines of a file (reading at most [`TAIL_BYTES`]), optionally only lines
/// containing `filter` (case-insensitive).
pub(crate) fn tail(path: &FsPath, lines: usize, filter: Option<&str>) -> std::io::Result<Vec<String>> {
    let mut file = fs::File::open(path)?;
    let len = file.metadata()?.len();
    let start = len.saturating_sub(TAIL_BYTES);
    file.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::new();
    file.take(TAIL_BYTES).read_to_end(&mut buf)?;
    let text = String::from_utf8_lossy(&buf);
    let mut all: Vec<&str> = text.lines().collect();
    if start > 0 && !all.is_empty() {
        all.remove(0); // probably cut in half
    }
    let needle = filter.map(str::to_lowercase).filter(|f| !f.is_empty());
    let kept: Vec<String> = all
        .into_iter()
        .filter(|l| needle.as_ref().is_none_or(|n| l.to_lowercase().contains(n)))
        .map(str::to_string)
        .collect();
    let skip = kept.len().saturating_sub(lines);
    Ok(kept.into_iter().skip(skip).collect())
}

// ---------- pages ----------

fn find_server<'a>(state: &'a AppState, user: &lighthouse_store::User, id: &str) -> Option<&'a ServerConfig> {
    state
        .ctx
        .config
        .servers
        .iter()
        .find(|s| s.id == id && user.sees_server(&s.id))
}

async fn scan_server(server: &ServerConfig) -> Vec<LogFile> {
    let Some(dir) = server.log_dir.clone() else {
        return Vec::new();
    };
    let game = server.game;
    tokio::task::spawn_blocking(move || scan(FsPath::new(&dir), game))
        .await
        .unwrap_or_default()
}

async fn list(State(state): State<AppState>, CurrentUser(user): CurrentUser) -> Markup {
    let servers = state.visible_servers(&user);
    let with_logs: Vec<_> = servers.iter().filter(|s| s.log_dir.is_some()).collect();
    layout::page(
        &state,
        &user,
        "logs",
        "Logs",
        html! {
            @if with_logs.is_empty() {
                div.notice {
                    p { "No server has a log folder set." }
                    p { "If a game server runs on this machine, add its install folder to its entry in the config file, e.g.:" }
                    pre.example { "[[servers]]\nid = \"athens\"\n...\nlog_dir = \"C:/Games/7DaysToDieServer\"" }
                    p.muted { "Project Zomboid keeps its logs in the Zomboid folder in the user's home (Zomboid/Logs)." }
                }
            } @else {
                div.table-wrap { table {
                    thead { tr { th { "Server" } th { "Game" } th { "Folder" } } }
                    tbody { @for s in &with_logs { tr {
                        td { a href={ "/logs/" (s.id) } { (server_name(s, state.ctx.monitor.status(&s.id).as_ref())) } }
                        td { (s.game) }
                        td.muted { (s.log_dir.as_deref().unwrap_or_default()) }
                    } } }
                } }
            }
        },
    )
}

async fn files(State(state): State<AppState>, CurrentUser(user): CurrentUser, Path(id): Path<String>) -> Response {
    let Some(server) = find_server(&state, &user, &id) else {
        return forbidden(&state, &user);
    };
    let found = scan_server(server).await;
    let now = now_unix();
    let name = server_name(server, state.ctx.monitor.status(&server.id).as_ref());
    layout::page(
        &state,
        &user,
        "logs",
        &format!("Logs for {name}"),
        html! {
            p.muted { (server.log_dir.as_deref().unwrap_or_default()) }
            @if found.is_empty() {
                p.notice { "No log files found in this folder. Check the path, and that Lighthouse may read it." }
            } @else {
                div.table-wrap { table {
                    thead { tr { th { "File" } th.num { "Size" } th { "Changed" } } }
                    tbody { @for (i, f) in found.iter().enumerate() { tr {
                        td { a href={ "/logs/" (server.id) "/view?file=" (i) } { (f.shown) } }
                        td.num { (fmt_bytes(f.size)) }
                        td { (fmt_ago(now, f.modified)) }
                    } } }
                } }
            }
            a.back href="/logs" { "Back to all servers" }
        },
    )
    .into_response()
}

#[derive(Deserialize)]
struct ViewQuery {
    file: usize,
    lines: Option<usize>,
    q: Option<String>,
}

fn highlight(line: &str, needle: Option<&str>) -> Markup {
    let Some(n) = needle.filter(|n| !n.is_empty()) else {
        return html! { (line) "\n" };
    };
    let lower = line.to_lowercase();
    let n = n.to_lowercase();
    // Only highlight when lowercasing kept byte offsets (plain ASCII-ish lines).
    if lower.len() != line.len() {
        return html! { (line) "\n" };
    }
    let mut out = String::new();
    let mut i = 0;
    while let Some(pos) = lower[i..].find(&n) {
        let s = i + pos;
        out.push_str(&maud::html! { (line[i..s]) }.into_string());
        out.push_str(&maud::html! { mark { (line[s..s + n.len()]) } }.into_string());
        i = s + n.len();
    }
    out.push_str(&maud::html! { (line[i..]) }.into_string());
    html! { (PreEscaped(out)) "\n" }
}

async fn view(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
    Query(query): Query<ViewQuery>,
) -> Response {
    let Some(server) = find_server(&state, &user, &id) else {
        return forbidden(&state, &user);
    };
    let found = scan_server(server).await;
    let Some(file) = found.get(query.file).cloned() else {
        return axum::response::Redirect::to(&format!("/logs/{}", server.id)).into_response();
    };
    let lines = query.lines.unwrap_or(DEFAULT_LINES).clamp(10, MAX_LINES);
    let filter = query.q.clone().filter(|q| !q.trim().is_empty());
    let path = file.path.clone();
    let f = filter.clone();
    let content = tokio::task::spawn_blocking(move || tail(&path, lines, f.as_deref()))
        .await
        .unwrap_or_else(|e| Err(std::io::Error::other(e)));

    layout::page(
        &state,
        &user,
        "logs",
        &file.shown,
        html! {
            form.inline method="get" action={ "/logs/" (server.id) "/view" } {
                input type="hidden" name="file" value=(query.file);
                input name="q" placeholder="Filter (e.g. error, player name)" value=[filter.as_deref()];
                select name="lines" {
                    @for n in [100, 300, 1000, 5000] {
                        option value=(n) selected[n == lines] { "Last " (n) " lines" }
                    }
                }
                button type="submit" { "Show" }
                a href={ "/logs/" (server.id) } { "Back to the file list" }
            }
            @match content {
                Ok(rows) if rows.is_empty() => p.muted { "Nothing to show." },
                Ok(rows) => pre.log { @for l in &rows { (highlight(l, filter.as_deref())) } },
                Err(e) => p.error { "Could not read the file: " (e) },
            }
        },
    )
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_names() {
        let sdtd = GameKind::get(GameKind::SEVEN_DAYS_TO_DIE);
        let pz = GameKind::get(GameKind::PROJECT_ZOMBOID);
        assert!(is_log_name("output_log__2026-11-10__19-00-00.txt", sdtd));
        assert!(is_log_name("Vein.log", GameKind::get(GameKind::VEIN)));
        assert!(is_log_name("server-console.txt", pz));
        assert!(is_log_name("10-11-26_19-00_DebugLog-server.txt", pz));
        assert!(!is_log_name("serverconfig.xml", sdtd));
        assert!(!is_log_name("readme.txt", sdtd));
    }

    #[test]
    fn scan_and_tail() {
        let dir = std::env::temp_dir().join(format!("lighthouse-logs-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("Logs")).unwrap();
        fs::create_dir_all(dir.join("Mods")).unwrap();
        fs::write(dir.join("Logs/server.log"), "one\nTwo error\nthree\nfour ERROR\n").unwrap();
        fs::write(dir.join("Mods/skip.log"), "x").unwrap();
        fs::write(dir.join("notes.txt"), "x").unwrap();

        let found = scan(&dir, GameKind::get(GameKind::VEIN));
        assert_eq!(
            found.iter().map(|f| f.shown.as_str()).collect::<Vec<_>>(),
            ["Logs/server.log"]
        );
        assert_eq!(tail(&found[0].path, 2, None).unwrap(), ["three", "four ERROR"]);
        assert_eq!(
            tail(&found[0].path, 10, Some("error")).unwrap(),
            ["Two error", "four ERROR"]
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn highlight_escapes_html() {
        let h = highlight("<b>error</b>", Some("error")).into_string();
        assert_eq!(h, "&lt;b&gt;<mark>error</mark>&lt;/b&gt;\n");
    }
}
