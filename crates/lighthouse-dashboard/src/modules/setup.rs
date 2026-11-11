//! Setup: add, change and remove game servers and Discord bots from the dashboard.
//!
//! Changes are written to `lighthouse.toml` (read fresh for every change, validated as a whole
//! before writing) and Lighthouse restarts to apply them. Bots are set up from just their token:
//! the page shows who the bot is, its invite link, and the channels it can post in.
//!
//! This page shows and stores tokens and passwords, so its permission is meant for owners only.

use std::collections::HashSet;
use std::path::Path as FsPath;
use std::time::Duration;

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use lighthouse_core::{
    BotConfig, BotMode, Config, GameKind, IngestConfig, LocalControl, NativeApi, Protocol, ServerConfig, Telnet,
};
use lighthouse_discord::setup as discord;
use maud::{Markup, PreEscaped, html};

use super::controls::{Flash, flash, urlencode};
use super::moderation::form_pairs;
use crate::auth::{CurrentUser, forbidden};
use crate::module::{Module, PermissionDef};
use crate::{AppState, layout};

const MANAGE: &str = "setup.manage";

pub struct Setup;

impl Module for Setup {
    fn id(&self) -> &'static str {
        "setup"
    }
    fn title(&self) -> &'static str {
        "Setup"
    }
    fn icon(&self) -> &'static str {
        // sliders
        "M4 6h10 M18 6h2 M4 12h4 M12 12h8 M4 18h12 M20 18h0 M16 4v4 M10 10v4 M18 16v4"
    }
    fn view_permission(&self) -> &'static str {
        MANAGE
    }
    fn permissions(&self) -> Vec<PermissionDef> {
        vec![PermissionDef {
            key: MANAGE,
            label: "Add and change servers and bots (sees tokens and passwords)",
        }]
    }
    fn routes(&self) -> Router<AppState> {
        Router::new()
            .route("/", get(page))
            .route("/server/new", get(new_server))
            .route("/server/{id}", get(edit_server))
            .route("/server/save", post(save_server))
            .route("/server/{id}/remove", post(remove_server))
            .route("/bot/new", get(new_bot))
            .route("/bot/check", post(check_bot))
            .route("/bot/save", post(save_bot))
            .route("/bot/{index}/remove", post(remove_bot))
            .route("/mods/on", post(mods_on))
            .route("/mods/off", post(mods_off))
    }
}

// ---------- config file ----------

/// The config as it is on disk now (it may have changed since Lighthouse started).
fn current(state: &AppState) -> Result<Config, String> {
    match &state.ctx.config_path {
        Some(path) => Config::load(path).map_err(|e| e.to_string()),
        None => Err("this Lighthouse wasn't started from a config file".to_string()),
    }
}

/// Validate, write and restart. Returns the problem if the config isn't valid.
fn apply(state: &AppState, config: &Config) -> Result<(), String> {
    config.validate().map_err(|e| e.to_string())?;
    let path = state.ctx.config_path.as_deref().ok_or("no config file")?;
    config.save(path).map_err(|e| e.to_string())?;
    restrict_permissions(path);
    if let Some(restart) = state.ctx.restart.clone() {
        // Let the response reach the browser first.
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(800)).await;
            restart();
        });
    }
    Ok(())
}

/// The config holds tokens: readable by its owner only (Unix).
fn restrict_permissions(path: &FsPath) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

fn restarting_page(state: &AppState, user: &lighthouse_store::User, what: &str) -> Response {
    let restarts = state.ctx.restart.is_some();
    layout::page_with_head(
        state,
        user,
        "setup",
        "Saved",
        html! { @if restarts { meta http-equiv="refresh" content="8;url=/setup"; } },
        html! {
            p.notice { (what) " saved to the config file." }
            @if restarts {
                p { "Lighthouse is restarting to apply it. This page reloads in a few seconds." }
            } @else {
                p { "Restart Lighthouse to apply it." }
            }
            a.back href="/setup" { "Back to Setup" }
        },
    )
    .into_response()
}

fn mask(secret: &str) -> String {
    let n = secret.chars().count();
    if n <= 8 {
        "••••".to_string()
    } else {
        format!("{}••••", secret.chars().take(4).collect::<String>())
    }
}

// ---------- overview ----------

async fn page(State(state): State<AppState>, CurrentUser(user): CurrentUser, Query(f): Query<Flash>) -> Markup {
    let config = current(&state);
    layout::page(
        &state,
        &user,
        "setup",
        "Setup",
        html! {
            (flash(&f))
            @match &config {
                Err(e) => p.error { "Could not read the config file: " (e) },
                Ok(config) => {
                    h2 { "Game servers" }
                    @if config.servers.is_empty() { p.muted { "No servers yet." } } @else {
                        div.table-wrap { table {
                            thead { tr { th { "Name" } th { "Game" } th { "Address" } th { "Extras" } th {} } }
                            tbody { @for s in &config.servers { tr {
                                td { a href={ "/setup/server/" (s.id) } { (s.name.as_deref().unwrap_or(&s.id)) } br; small.muted { (s.id) } }
                                td { (s.game) }
                                td { code { (s.host) ":" (s.game_port) } @if let Some(q) = s.query_port { " " small.muted { "status on " (q) } } }
                                td.muted { (extras(s)) }
                                td { form method="post" action={ "/setup/server/" (s.id) "/remove" } onsubmit="return confirm('Remove this server?')" {
                                    button.danger type="submit" { "Remove" }
                                } }
                            } } }
                        } }
                    }
                    a.button href="/setup/server/new" { "Add a server" }

                    h2 { "Discord bots" }
                    p.muted {
                        @match config.mode {
                            BotMode::Mega => "One bot shows every server. ",
                            BotMode::PerGame => "One bot per game. ",
                        }
                        "Switch with " code { "lighthouse bot-mode" } " on the machine running Lighthouse."
                    }
                    @if config.bots.is_empty() { p.muted { "No bots yet." } } @else {
                        div.table-wrap { table {
                            thead { tr { th { "Shows" } th { "Channel" } th { "Token" } th {} } }
                            tbody { @for (i, b) in config.bots.iter().enumerate() { tr {
                                td { @match b.game { Some(g) => (g), None => "All games" } @if let Some(t) = &b.title { br; small.muted { (t) } } }
                                td { code { (b.status_channel_id) } }
                                td.muted { @if let Some(t) = &b.token { (mask(t)) } @else if let Some(v) = &b.token_env { "from " code { (v) } } }
                                td { form method="post" action={ "/setup/bot/" (i) "/remove" } onsubmit="return confirm('Remove this bot? Its status post stays in Discord.')" {
                                    button.danger type="submit" { "Remove" }
                                } }
                            } } }
                        } }
                    }
                    a.button href="/setup/bot/new" { "Add a bot" }
                    (mods_section(config))
                    p.muted { "Saving a change restarts Lighthouse; the status posts carry on where they were." }
                }
            }
        },
    )
}

fn extras(s: &ServerConfig) -> String {
    let mut out = Vec::new();
    match &s.native {
        Some(NativeApi::SevenDaysWebApi { .. }) => out.push("web API"),
        Some(NativeApi::Rcon { .. }) => out.push("RCON"),
        Some(NativeApi::VeinHttp { .. }) => out.push("HTTP API"),
        None => {}
    }
    if s.telnet.is_some() {
        out.push("telnet");
    }
    if s.control.is_some() {
        out.push("local commands");
    }
    if s.log_dir.is_some() {
        out.push("log folder");
    }
    out.join(", ")
}

// ---------- servers ----------

/// The server form's fields, as strings (what the browser sent, or what's in the config).
#[derive(Default, Clone)]
struct ServerForm {
    original_id: String,
    id: String,
    game: String,
    name: String,
    host: String,
    game_port: String,
    query_port: String,
    show_address: bool,
    show_players: bool,
    alerts: bool,
    api: String,
    api_port: String,
    api_user: String,
    api_secret: String,
    telnet_port: String,
    telnet_password: String,
    control_start: String,
    control_stop: String,
    control_restart: String,
    log_dir: String,
    status_file: String,
    process: String,
    action: String,
}

impl ServerForm {
    fn from_server(s: &ServerConfig) -> Self {
        let mut f = ServerForm {
            original_id: s.id.clone(),
            id: s.id.clone(),
            game: s.game.id().to_string(),
            name: s.name.clone().unwrap_or_default(),
            host: s.host.clone(),
            game_port: s.game_port.to_string(),
            query_port: s.query_port.map(|p| p.to_string()).unwrap_or_default(),
            show_address: s.show_address,
            show_players: s.show_players,
            alerts: s.alerts,
            log_dir: s.log_dir.clone().unwrap_or_default(),
            status_file: s.status_file.clone().unwrap_or_default(),
            process: s.process.clone().unwrap_or_default(),
            ..Default::default()
        };
        match &s.native {
            Some(NativeApi::SevenDaysWebApi { port, token_name, .. }) => {
                f.api = "web".into();
                f.api_port = port.to_string();
                f.api_user = token_name.clone();
            }
            Some(NativeApi::Rcon { port, .. }) => {
                f.api = "rcon".into();
                f.api_port = port.to_string();
            }
            Some(NativeApi::VeinHttp { port }) => {
                f.api = "vein".into();
                f.api_port = port.to_string();
            }
            None => {}
        }
        if let Some(t) = &s.telnet {
            f.telnet_port = t.port.to_string();
        }
        if let Some(c) = &s.control {
            f.control_start = c.start.clone().unwrap_or_default();
            f.control_stop = c.stop.clone().unwrap_or_default();
            f.control_restart = c.restart.clone().unwrap_or_default();
        }
        f
    }

    fn parse(body: &str) -> Self {
        let mut f = ServerForm::default();
        for (k, v) in form_pairs(body) {
            let v = v.trim().to_string();
            match k.as_str() {
                "original_id" => f.original_id = v,
                "id" => f.id = v,
                "game" => f.game = v,
                "name" => f.name = v,
                "host" => f.host = v,
                "game_port" => f.game_port = v,
                "query_port" => f.query_port = v,
                "show_address" => f.show_address = true,
                "show_players" => f.show_players = true,
                "alerts" => f.alerts = true,
                "api" => f.api = v,
                "api_port" => f.api_port = v,
                "api_user" => f.api_user = v,
                "api_secret" => f.api_secret = v,
                "telnet_port" => f.telnet_port = v,
                "telnet_password" => f.telnet_password = v,
                "control_start" => f.control_start = v,
                "control_stop" => f.control_stop = v,
                "control_restart" => f.control_restart = v,
                "log_dir" => f.log_dir = v,
                "status_file" => f.status_file = v,
                "process" => f.process = v,
                "action" => f.action = v,
                _ => {}
            }
        }
        f
    }

    /// Build the server, keeping secrets from `old` where the form left them empty.
    fn to_server(&self, old: Option<&ServerConfig>, taken: &HashSet<String>) -> Result<ServerConfig, String> {
        let game = GameKind::from_id(&self.game).ok_or("Pick a game.")?;
        let host = self.host.trim();
        if host.is_empty() || host.contains(char::is_whitespace) || (host.contains(':') && !host.contains("::")) {
            return Err("Enter the server's address without a port (the port has its own box).".into());
        }
        let port = |s: &str, what: &str| -> Result<Option<u16>, String> {
            if s.is_empty() {
                Ok(None)
            } else {
                s.parse::<u16>()
                    .ok()
                    .filter(|p| *p > 0)
                    .map(Some)
                    .ok_or(format!("{what} must be a number from 1 to 65535."))
            }
        };
        let game_port = port(&self.game_port, "The port")?.unwrap_or(game.default_game_port());
        let id = if self.id.is_empty() {
            slug(&self.name, game, taken)
        } else {
            self.id.clone()
        };
        if !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err("The id may only use letters, digits, - and _.".into());
        }
        if taken.contains(&id) {
            return Err(format!("Another server already has the id \"{id}\"."));
        }
        let some = |s: &str| (!s.is_empty()).then(|| s.to_string());
        let mut s = ServerConfig::new(id, game, host, game_port);
        s.name = some(&self.name);
        s.query_port = port(&self.query_port, "The status port")?;
        s.show_address = self.show_address;
        s.show_players = self.show_players;
        s.alerts = self.alerts;
        s.log_dir = some(&self.log_dir);
        s.status_file = some(&self.status_file);
        s.process = some(&self.process);

        let old_native = old.and_then(|o| o.native.clone());
        let api_port = port(&self.api_port, "The API port")?;
        s.native = match self.api.as_str() {
            "web" => {
                let token_secret = match (&self.api_secret, &old_native) {
                    (s, _) if !s.is_empty() => s.clone(),
                    (_, Some(NativeApi::SevenDaysWebApi { token_secret, .. })) => token_secret.clone(),
                    _ => return Err("Enter the web API token secret.".into()),
                };
                Some(NativeApi::SevenDaysWebApi {
                    port: api_port.unwrap_or(8080),
                    token_name: some(&self.api_user).unwrap_or_else(|| "lighthouse".into()),
                    token_secret,
                })
            }
            "rcon" => {
                let password = match (&self.api_secret, &old_native) {
                    (s, _) if !s.is_empty() => s.clone(),
                    (_, Some(NativeApi::Rcon { password, .. })) => password.clone(),
                    _ => return Err("Enter the RCON password.".into()),
                };
                Some(NativeApi::Rcon {
                    port: api_port.unwrap_or(27015),
                    password,
                })
            }
            "vein" => Some(NativeApi::VeinHttp {
                port: api_port.unwrap_or(8085),
            }),
            _ => None,
        };
        if let Some(tp) = port(&self.telnet_port, "The telnet port")? {
            let password = if !self.telnet_password.is_empty() {
                self.telnet_password.clone()
            } else {
                old.and_then(|o| o.telnet.as_ref())
                    .map(|t| t.password.clone())
                    .ok_or("Enter the telnet password.")?
            };
            s.telnet = Some(Telnet { port: tp, password });
        }
        let control = LocalControl {
            start: some(&self.control_start),
            stop: some(&self.control_stop),
            restart: some(&self.control_restart),
        };
        s.control = (control != LocalControl::default()).then_some(control);
        Ok(s)
    }
}

/// An id from the display name: `Harbour Light PvE!` -> `harbour-light-pve`.
fn slug(name: &str, game: GameKind, taken: &HashSet<String>) -> String {
    let mut base = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            base.push(c.to_ascii_lowercase());
        } else if !base.is_empty() && !base.ends_with('-') {
            base.push('-');
        }
    }
    let base = base.trim_end_matches('-').to_string();
    let base = if base.is_empty() {
        game.short_name().to_ascii_lowercase().replace('_', "-")
    } else {
        base
    };
    if !taken.contains(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}-{n}"))
        .find(|id| !taken.contains(id))
        .unwrap_or(base)
}

/// Shows the extras that fit the chosen game.
const SERVER_FORM_SCRIPT: &str = r#"
(() => {
  const game = document.getElementById('game'), api = document.getElementById('api');
  const show = () => {
    const g = game.value;
    document.querySelectorAll('[data-games]').forEach(el => {
      el.hidden = !el.dataset.games.split(' ').includes(g);
    });
    document.querySelectorAll('[data-api]').forEach(el => {
      el.hidden = !el.dataset.api.split(' ').includes(api.value);
    });
  };
  game.addEventListener('change', show); api.addEventListener('change', show); show();
})();
"#;

fn server_form(state: &AppState, user: &lighthouse_store::User, f: &ServerForm, message: Option<Markup>) -> Response {
    let editing = !f.original_id.is_empty();
    let mut games = GameKind::all();
    games.sort_by_key(|g| (g.is_experimental(), g.display_name()));
    let selected = GameKind::from_id(&f.game);
    let checked = |b: bool| b.then_some("checked");
    layout::page(
        state,
        user,
        "setup",
        if editing { "Edit server" } else { "Add a server" },
        html! {
            @if let Some(m) = message { (m) }
            form method="post" action="/setup/server/save" autocomplete="off" {
                input type="hidden" name="original_id" value=(f.original_id);
                label { "Game"
                    select #game name="game" required {
                        option value="" { "Pick a game" }
                        @for g in &games {
                            option value=(g.id()) selected[selected == Some(*g)] { (g.display_name()) @if g.is_experimental() { " (experimental)" } }
                        }
                    }
                }
                label { "Display name" input name="name" value=(f.name) placeholder="Shown on the status post; empty = the name the server reports"; }
                label { "Address" input name="host" value=(f.host) placeholder="IP address or hostname, no port" required; }
                label { "Port players connect to" input name="game_port" value=(f.game_port) inputmode="numeric" placeholder="Empty = the game's default"; }
                label { "Status port (only if it differs)" input name="query_port" value=(f.query_port) inputmode="numeric" placeholder="Usually empty"; }
                label.check { input type="checkbox" name="show_address" checked=[checked(f.show_address)]; "Show the join address on the status post" }
                label.check { input type="checkbox" name="show_players" checked=[checked(f.show_players)]; "List player names (where the game shares them)" }
                label.check { input type="checkbox" name="alerts" checked=[checked(f.alerts)]; "Post an alert when it goes down or comes back" }

                fieldset {
                    legend { "Game API (optional)" }
                    p.muted.small { "Real uptime, player names, controls and a live console. See the guide: docs/mods.md." }
                    label { "Kind"
                        select #api name="api" {
                            option value="" selected[f.api.is_empty()] { "None" }
                            option value="web" selected[f.api == "web"] { "7 Days to Die web API" }
                            option value="rcon" selected[f.api == "rcon"] { "Project Zomboid RCON" }
                            option value="vein" selected[f.api == "vein"] { "VEIN HTTP API" }
                        }
                    }
                    div data-api="web rcon vein" {
                        label { "API port" input name="api_port" value=(f.api_port) inputmode="numeric" placeholder="8080 (7DTD), 27015 (RCON), 8085 (VEIN)"; }
                    }
                    div data-api="web" {
                        label { "Token name" input name="api_user" value=(f.api_user) placeholder="lighthouse"; }
                        p.muted.small { "In the server console: " code { "webtokens add lighthouse <secret> 0" } " (secret: letters, digits and _ only)." }
                    }
                    div data-api="web rcon" {
                        label { "Token secret / RCON password"
                            input type="password" name="api_secret" placeholder=(if editing { "Leave empty to keep the saved one" } else { "" });
                        }
                    }
                }
                fieldset data-games="seven_days_to_die" {
                    legend { "7 Days to Die telnet (optional, for controls without the web API)" }
                    label { "Telnet port" input name="telnet_port" value=(f.telnet_port) inputmode="numeric" placeholder="8081"; }
                    label { "Telnet password" input type="password" name="telnet_password" placeholder=(if editing { "Leave empty to keep the saved one" } else { "" }); }
                }
                fieldset {
                    legend { "On this machine (optional)" }
                    p.muted.small { "Only when the game server runs where Lighthouse runs." }
                    label { "Log folder" input name="log_dir" value=(f.log_dir) placeholder="The server's install or data folder"; }
                    label { "Program name (for real uptime)" input name="process" value=(f.process) placeholder="e.g. VeinServer"; }
                    label { "Mod status file (Project Zomboid)" input name="status_file" value=(f.status_file) placeholder=".../Zomboid/Lua/lighthouse_status.json"; }
                    label { "Start command" input name="control_start" value=(f.control_start) placeholder="e.g. systemctl start vein"; }
                    label { "Stop command" input name="control_stop" value=(f.control_stop); }
                    label { "Restart command" input name="control_restart" value=(f.control_restart); }
                }
                details { summary { "Advanced" }
                    label { "Id (letters, digits, - and _; used in the database and by mods)"
                        input name="id" value=(f.id) placeholder="Made from the name when empty";
                    }
                }
                div.inline {
                    button type="submit" name="action" value="test" { "Test" }
                    button.primary type="submit" name="action" value="save" { "Save" }
                    a href="/setup" { "Cancel" }
                }
            }
            script { (PreEscaped(SERVER_FORM_SCRIPT)) }
        },
    )
    .into_response()
}

async fn new_server(State(state): State<AppState>, CurrentUser(user): CurrentUser) -> Response {
    let f = ServerForm {
        show_address: true,
        show_players: true,
        ..Default::default()
    };
    server_form(&state, &user, &f, None)
}

async fn edit_server(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> Response {
    let config = match current(&state) {
        Ok(c) => c,
        Err(e) => return Redirect::to(&format!("/setup?err={}", urlencode(&e))).into_response(),
    };
    match config.servers.iter().find(|s| s.id == id) {
        Some(s) => server_form(&state, &user, &ServerForm::from_server(s), None),
        None => Redirect::to("/setup").into_response(),
    }
}

async fn save_server(State(state): State<AppState>, CurrentUser(user): CurrentUser, body: String) -> Response {
    if !user.can(MANAGE) {
        return forbidden(&state, &user);
    }
    let f = ServerForm::parse(&body);
    let mut config = match current(&state) {
        Ok(c) => c,
        Err(e) => return server_form(&state, &user, &f, Some(html! { p.error { (e) } })),
    };
    let old_index = config
        .servers
        .iter()
        .position(|s| !f.original_id.is_empty() && s.id == f.original_id);
    let taken: HashSet<String> = config
        .servers
        .iter()
        .filter(|s| s.id != f.original_id)
        .map(|s| s.id.clone())
        .collect();
    let server = match f.to_server(old_index.map(|i| &config.servers[i]), &taken) {
        Ok(s) => s,
        Err(e) => return server_form(&state, &user, &f, Some(html! { p.error { (e) } })),
    };

    if f.action == "test" {
        let result = test_server(&server).await;
        return server_form(&state, &user, &f, Some(result));
    }

    match old_index {
        Some(i) => config.servers[i] = server.clone(),
        None => config.servers.push(server.clone()),
    }
    match apply(&state, &config) {
        Ok(()) => {
            tracing::info!(user = %user.username, server = %server.id, "server saved from the dashboard");
            restarting_page(
                &state,
                &user,
                &format!("Server \"{}\"", server.name.as_deref().unwrap_or(&server.id)),
            )
        }
        Err(e) => server_form(&state, &user, &f, Some(html! { p.error { "Not saved: " (e) } })),
    }
}

/// Query the server now, as the Test button does.
async fn test_server(server: &ServerConfig) -> Markup {
    let now = crate::util::now_unix();
    let adapter = lighthouse_games::adapter_for(server.game);
    let query = adapter.query(server, now).await;
    let api = lighthouse_games::check_native(server).await;
    html! {
        @match &query {
            Ok(s) => p.notice {
                "✓ Found \"" (s.name.as_deref().unwrap_or("unnamed server")) "\""
                @if let (Some(n), Some(m)) = (s.players_online, s.players_max) { ", " (n) "/" (m) " players" }
                @if let Some(v) = &s.version { ", version " (v) }
                "."
            },
            Err(e) => p.error { "✗ No answer from " (server.host) ":" (server.query_port.unwrap_or(server.game.query_port_for(server.game_port))) " (" (e) "). Check the address and port, or the status port." },
        }
        @match &api {
            Ok(None) => {},
            Ok(Some(n)) => p.notice { "✓ The game API answered (" (n) " players listed)." },
            Err(e) => p.error { "✗ The game API didn't answer: " (e) },
        }
    }
}

async fn remove_server(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
) -> Response {
    if !user.can(MANAGE) {
        return forbidden(&state, &user);
    }
    let mut config = match current(&state) {
        Ok(c) => c,
        Err(e) => return Redirect::to(&format!("/setup?err={}", urlencode(&e))).into_response(),
    };
    config.servers.retain(|s| s.id != id);
    match apply(&state, &config) {
        Ok(()) => restarting_page(&state, &user, &format!("Removal of \"{id}\"")),
        Err(e) => Redirect::to(&format!("/setup?err={}", urlencode(&format!("Not removed: {e}")))).into_response(),
    }
}

// ---------- game mods ----------

/// The heartbeat port the FiveM/RedM resource and the 7 Days to Die mod report to.
fn mods_section(config: &Config) -> Markup {
    let uses_mods = config
        .servers
        .iter()
        .any(|s| s.game.protocol() == Protocol::FiveM || s.game.is(GameKind::SEVEN_DAYS_TO_DIE));
    html! {
        h2 { "Game mods" }
        p.muted {
            "The FiveM/RedM resource and the 7 Days to Die mod send Lighthouse their status (players, uptime, "
            "txAdmin bans, the console). They need this switched on. "
            a href="https://github.com/Hailey-Ross/hails.Lighthouse/blob/main/docs/mods.md" target="_blank" rel="noopener" { "Guide" }
        }
        @match &config.ingest {
            None => {
                @if !uses_mods { p.muted.small { "None of your servers needs it yet." } }
                form method="post" action="/setup/mods/on" { button type="submit" { "Turn on" } }
            }
            Some(ingest) => {
                @let port = ingest.bind.rsplit(':').next().unwrap_or("8651");
                p.notice { "On, listening on port " b { (port) } "." }
                p { "For FiveM or RedM, add these lines to the server's " code { "server.cfg" } " (replace "
                    i { "this-computer" } " with this computer's public address):" }
                pre.example {
                    "set lighthouse_url \"http://this-computer:" (port) "\"\n"
                    "set lighthouse_secret \"" (ingest.secret) "\"\n"
                    "set lighthouse_server_id \"<the server's id from the list above>\"\n"
                    "add_ace resource.lighthouse command.quit allow\n"
                    "ensure lighthouse"
                }
                p.muted.small {
                    "The game server has to reach this computer on port " (port) ". On a home internet connection, "
                    "forward that port to this computer in your router. Keep the secret private."
                }
                form method="post" action="/setup/mods/off" onsubmit="return confirm('Turn off? Mods stop reporting until it is on again.')" {
                    button.danger type="submit" { "Turn off" }
                }
            }
        }
    }
}

async fn mods_on(State(state): State<AppState>, CurrentUser(user): CurrentUser) -> Response {
    if !user.can(MANAGE) {
        return forbidden(&state, &user);
    }
    let mut config = match current(&state) {
        Ok(c) => c,
        Err(e) => return Redirect::to(&format!("/setup?err={}", urlencode(&e))).into_response(),
    };
    if config.ingest.is_none() {
        let secret = match lighthouse_store::random_secret() {
            Ok(s) => s,
            Err(e) => return Redirect::to(&format!("/setup?err={}", urlencode(&e.to_string()))).into_response(),
        };
        config.ingest = Some(IngestConfig {
            bind: "0.0.0.0:8651".to_string(),
            secret,
        });
    }
    match apply(&state, &config) {
        Ok(()) => restarting_page(&state, &user, "Game mod reporting"),
        Err(e) => Redirect::to(&format!("/setup?err={}", urlencode(&format!("Not saved: {e}")))).into_response(),
    }
}

async fn mods_off(State(state): State<AppState>, CurrentUser(user): CurrentUser) -> Response {
    if !user.can(MANAGE) {
        return forbidden(&state, &user);
    }
    let mut config = match current(&state) {
        Ok(c) => c,
        Err(e) => return Redirect::to(&format!("/setup?err={}", urlencode(&e))).into_response(),
    };
    config.ingest = None;
    match apply(&state, &config) {
        Ok(()) => restarting_page(&state, &user, "Turning off game mod reporting"),
        Err(e) => Redirect::to(&format!("/setup?err={}", urlencode(&format!("Not saved: {e}")))).into_response(),
    }
}

// ---------- bots ----------

#[derive(Default)]
struct BotForm {
    token: String,
    game: String,
    title: String,
    channel: String,
}

impl BotForm {
    fn parse(body: &str) -> Self {
        let mut f = BotForm::default();
        for (k, v) in form_pairs(body) {
            let v = v.trim().to_string();
            match k.as_str() {
                "token" => f.token = v,
                "game" => f.game = v,
                "title" => f.title = v,
                "channel" => f.channel = v,
                _ => {}
            }
        }
        f
    }
}

fn game_picker(config: &Config, chosen: &str) -> Markup {
    // Per-game mode: the games that have servers.
    let mut games: Vec<GameKind> = config.servers.iter().map(|s| s.game).collect();
    games.sort_by_key(|g| g.display_name());
    games.dedup();
    html! {
        label { "Which game this bot shows"
            select name="game" required {
                @for g in &games { option value=(g.id()) selected[g.id() == chosen] { (g.display_name()) } }
            }
        }
    }
}

async fn new_bot(State(state): State<AppState>, CurrentUser(user): CurrentUser) -> Response {
    let config = match current(&state) {
        Ok(c) => c,
        Err(e) => return Redirect::to(&format!("/setup?err={}", urlencode(&e))).into_response(),
    };
    layout::page(
        &state,
        &user,
        "setup",
        "Add a bot",
        html! {
            ol.steps {
                li { "Open the " a href=(discord::PORTAL) target="_blank" rel="noopener" { "Discord Developer Portal" } " and click " b { "New Application" } "." }
                li { "On the " b { "Bot" } " page, click " b { "Reset Token" } " and copy the token." }
                li { "Paste it below. Lighthouse finds the bot, gives you its invite link, and lists the channels it can post in." }
            }
            form method="post" action="/setup/bot/check" autocomplete="off" {
                label { "Bot token" input type="password" name="token" required; }
                @if config.mode == BotMode::PerGame { (game_picker(&config, "")) }
                @else { label { "Board title (optional)" input name="title" placeholder="e.g. your community's name"; } }
                div.inline { button.primary type="submit" { "Continue" } a href="/setup" { "Cancel" } }
            }
        },
    )
    .into_response()
}

async fn check_bot(State(state): State<AppState>, CurrentUser(user): CurrentUser, body: String) -> Response {
    if !user.can(MANAGE) {
        return forbidden(&state, &user);
    }
    let f = BotForm::parse(&body);
    let config = match current(&state) {
        Ok(c) => c,
        Err(e) => return Redirect::to(&format!("/setup?err={}", urlencode(&e))).into_response(),
    };
    let who = discord::identify(&f.token).await;
    let channels = match &who {
        Ok(_) => discord::channels(&f.token).await.unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    layout::page(
        &state,
        &user,
        "setup",
        "Add a bot",
        html! {
            @match &who {
                Err(e) => {
                    p.error { (e) }
                    a.back href="/setup/bot/new" { "Try again" }
                }
                Ok(bot) => {
                    p.notice { "✓ This is " b { (bot.name) } "." }
                    h2 { "1. Invite it to your Discord server" }
                    p { a.button href=(discord::invite_url(bot.application_id)) target="_blank" rel="noopener" { "Open the invite link" }
                        " " span.muted.small { "Permissions: " (discord::PERMISSION_NAMES) "." } }
                    h2 { "2. Pick its channel" }
                    form method="post" autocomplete="off" {
                        input type="hidden" name="token" value=(f.token);
                        input type="hidden" name="game" value=(f.game);
                        input type="hidden" name="title" value=(f.title);
                        @if channels.is_empty() {
                            p.muted { "The bot isn't in any Discord server yet. Open the invite link, add it, then refresh." }
                            button type="submit" formaction="/setup/bot/check" { "Refresh" }
                        } @else {
                            label { "Status channel"
                                select name="channel" required {
                                    @for c in &channels { option value=(c.id) { (c.guild) " › #" (c.name) } }
                                }
                            }
                            p.muted.small { "Make sure the bot may post there (View Channel, Send Messages)." }
                            div.inline {
                                button type="submit" formaction="/setup/bot/check" { "Refresh the list" }
                                button.primary type="submit" formaction="/setup/bot/save" { "Save the bot" }
                            }
                        }
                    }
                    @if config.mode == BotMode::Mega && !config.bots.is_empty() {
                        p.muted { "One-bot mode: saving replaces the current bot." }
                    }
                }
            }
        },
    )
    .into_response()
}

async fn save_bot(State(state): State<AppState>, CurrentUser(user): CurrentUser, body: String) -> Response {
    if !user.can(MANAGE) {
        return forbidden(&state, &user);
    }
    let f = BotForm::parse(&body);
    let mut config = match current(&state) {
        Ok(c) => c,
        Err(e) => return Redirect::to(&format!("/setup?err={}", urlencode(&e))).into_response(),
    };
    let Ok(channel) = f.channel.parse::<u64>() else {
        return Redirect::to("/setup?err=Pick%20a%20channel.").into_response();
    };
    let game = GameKind::from_id(&f.game);
    let bot = BotConfig {
        game: if config.mode == BotMode::PerGame { game } else { None },
        token: Some(f.token.clone()),
        token_env: None,
        status_channel_id: channel,
        title: (!f.title.is_empty()).then(|| f.title.clone()),
        alert_channel_id: None,
        alert_role_id: None,
    };
    match config.mode {
        BotMode::Mega => config.bots = vec![bot],
        BotMode::PerGame => {
            config.bots.retain(|b| b.game != bot.game);
            config.bots.push(bot);
        }
    }
    match apply(&state, &config) {
        Ok(()) => restarting_page(&state, &user, "The bot"),
        Err(e) => Redirect::to(&format!("/setup?err={}", urlencode(&format!("Not saved: {e}")))).into_response(),
    }
}

async fn remove_bot(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(index): Path<usize>,
) -> Response {
    if !user.can(MANAGE) {
        return forbidden(&state, &user);
    }
    let mut config = match current(&state) {
        Ok(c) => c,
        Err(e) => return Redirect::to(&format!("/setup?err={}", urlencode(&e))).into_response(),
    };
    if index < config.bots.len() {
        config.bots.remove(index);
    }
    match apply(&state, &config) {
        Ok(()) => restarting_page(&state, &user, "Removal of the bot"),
        Err(e) => Redirect::to(&format!("/setup?err={}", urlencode(&format!("Not removed: {e}")))).into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_form_round_trip_keeps_secrets() {
        let mut s = ServerConfig::new("athens", GameKind::get("7dtd"), "203.0.113.10", 26930);
        s.name = Some("Athens PvE".into());
        s.native = Some(NativeApi::SevenDaysWebApi {
            port: 8080,
            token_name: "lighthouse".into(),
            token_secret: "abc_123".into(),
        });
        s.telnet = Some(Telnet {
            port: 8081,
            password: "tpw".into(),
        });
        let f = ServerForm::from_server(&s);
        assert!(
            f.api_secret.is_empty() && f.telnet_password.is_empty(),
            "secrets never go back to the browser"
        );
        let back = f.to_server(Some(&s), &HashSet::new()).unwrap();
        assert_eq!(back.native, s.native);
        assert_eq!(back.telnet, s.telnet);
        assert_eq!((back.id.as_str(), back.game_port), ("athens", 26930));
    }

    #[test]
    fn new_server_from_form() {
        let f = ServerForm::parse(
            "game=project_zomboid&name=Night+Shade+PZ%21&host=203.0.113.10&game_port=8766&show_players=on&api=rcon&api_port=27015&api_secret=pw&action=save",
        );
        let taken: HashSet<String> = ["night-shade-pz".to_string()].into();
        let s = f.to_server(None, &taken).unwrap();
        assert_eq!(s.id, "night-shade-pz-2");
        assert_eq!(s.game_port, 8766);
        assert!(s.show_players && !s.show_address);
        assert_eq!(
            s.native,
            Some(NativeApi::Rcon {
                port: 27015,
                password: "pw".into()
            })
        );
        assert!(s.control.is_none());
    }

    #[test]
    fn bad_input_is_explained() {
        let bad_host = ServerForm::parse("game=vein&host=1.2.3.4%3A7777");
        assert!(
            bad_host
                .to_server(None, &HashSet::new())
                .unwrap_err()
                .contains("without a port")
        );
        let no_game = ServerForm::parse("host=1.2.3.4");
        assert!(no_game.to_server(None, &HashSet::new()).is_err());
        let rcon_no_pw = ServerForm::parse("game=pz&host=1.2.3.4&api=rcon");
        assert!(
            rcon_no_pw
                .to_server(None, &HashSet::new())
                .unwrap_err()
                .contains("RCON password")
        );
    }
}
