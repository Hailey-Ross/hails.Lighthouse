//! Moderation: who is online, kicks and bans, and the history of everything that happened on the
//! servers (bans, warns and kicks from txAdmin or the game, scheduled restarts, announcements, and
//! actions taken from this dashboard).
//!
//! Bans work two ways:
//!
//! - **FiveM and RedM**: Lighthouse keeps the ban list. The Lighthouse resource on the server gets
//!   it with each heartbeat, keeps a copy, refuses banned players when they connect and drops them
//!   if they're online. A ban can cover one server or all of them.
//! - **7 Days to Die and Project Zomboid**: the game's own ban command runs (through the mod,
//!   telnet or RCON) and the game keeps the ban.

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::response::Response;
use axum::routing::{get, post};
use lighthouse_core::{ControlAction, Protocol, ServerConfig, ServerEvent, ServerStatus};
use lighthouse_games::control;
use lighthouse_store::{BanRecord, NewBan, StoredEvent};
use maud::{Markup, html};
use serde::Deserialize;

use super::controls::{Flash, find_server, flash, log_action, redirect_with};
use super::server_name;
use crate::auth::{CurrentUser, forbidden};
use crate::module::{Module, PermissionDef};
use crate::util::{fmt_duration, fmt_utc, now_unix};
use crate::{AppState, layout};

pub(crate) const VIEW: &str = "moderation.view";
pub(crate) const KICK: &str = "moderation.kick";
pub(crate) const BAN: &str = "moderation.ban";

/// Ban lengths offered in the form (seconds; 0 = permanent).
const DURATIONS: [(i64, &str); 7] = [
    (3600, "1 hour"),
    (86_400, "1 day"),
    (3 * 86_400, "3 days"),
    (7 * 86_400, "1 week"),
    (30 * 86_400, "30 days"),
    (90 * 86_400, "90 days"),
    (0, "Permanent"),
];

pub struct Moderation;

impl Module for Moderation {
    fn id(&self) -> &'static str {
        "moderation"
    }
    fn title(&self) -> &'static str {
        "Moderation"
    }
    fn icon(&self) -> &'static str {
        // gavel
        "M14 4l6 6 M11 7l6 6 M12.5 5.5l-5 5 M15.5 8.5l-5 5 M9 12L3 18l3 3 6-6 M4 21h9"
    }
    fn view_permission(&self) -> &'static str {
        VIEW
    }
    fn permissions(&self) -> Vec<PermissionDef> {
        vec![
            PermissionDef {
                key: VIEW,
                label: "See players, bans and server activity",
            },
            PermissionDef {
                key: KICK,
                label: "Kick players",
            },
            PermissionDef {
                key: BAN,
                label: "Ban players and lift bans",
            },
        ]
    }
    fn routes(&self) -> Router<AppState> {
        Router::new()
            .route("/", get(page))
            .route("/{id}/kick", post(kick))
            .route("/{id}/ban", post(ban))
            .route("/bans/{ban}/lift", post(lift))
    }
}

/// FiveM and RedM servers use the Lighthouse ban list.
fn uses_ban_list(server: &ServerConfig) -> bool {
    server.game.protocol() == Protocol::FiveM
}

fn kind_label(kind: &str) -> &str {
    match kind {
        "ban" => "Ban",
        "warn" => "Warning",
        "kick" => "Kick",
        "revoke" => "Lifted",
        "announcement" => "Announcement",
        "restart_scheduled" => "Restart scheduled",
        "restart_skipped" => "Restart skipped",
        "shutdown" => "Shutdown",
        "start" => "Started",
        "control" => "Dashboard",
        other => other,
    }
}

fn kind_class(kind: &str) -> &'static str {
    match kind {
        "ban" => "bad",
        "warn" | "kick" | "shutdown" => "warn",
        "revoke" | "start" => "good",
        _ => "",
    }
}

#[derive(Deserialize)]
struct PageQuery {
    server: Option<String>,
    show: Option<String>,
    #[serde(flatten)]
    flash: Flash,
}

async fn page(State(state): State<AppState>, CurrentUser(user): CurrentUser, Query(q): Query<PageQuery>) -> Markup {
    let now = now_unix();
    let relay = state.ctx.monitor.relay();
    let mut servers = state.visible_servers(&user);
    if let Some(only) = q.server.as_deref().filter(|s| !s.is_empty()) {
        servers.retain(|s| s.id == only);
    }
    let ids: Vec<&str> = servers.iter().map(|s| s.id.as_str()).collect();
    let moderation_only = q.show.as_deref() != Some("all");
    let kinds: Option<&[&str]> = moderation_only.then_some(&["ban", "warn", "kick", "revoke", "control"][..]);
    let events: Vec<StoredEvent> = state.ctx.store.events(&ids, kinds, 200).unwrap_or_default();
    let revoked = state.ctx.store.revoked_actions(&ids).unwrap_or_default();
    let bans: Vec<BanRecord> = state.ctx.store.bans(&ids, 200).unwrap_or_default();
    let names = |id: &str| {
        state
            .ctx
            .config
            .servers
            .iter()
            .find(|s| s.id == id)
            .map(|s| server_name(s, state.ctx.monitor.status(&s.id).as_ref()))
            .unwrap_or_else(|| id.to_string())
    };
    let any_ban_list = servers.iter().any(|s| uses_ban_list(s));
    // The offline-ban form posts to the first FiveM/RedM server; its scope picker decides the rest.
    let offline_target = servers
        .iter()
        .find(|s| uses_ban_list(s))
        .map(|s| s.id.clone())
        .unwrap_or_default();

    layout::page(
        &state,
        &user,
        "moderation",
        "Moderation",
        html! {
            (flash(&q.flash))
            form.inline method="get" action="/moderation" {
                select name="server" onchange="this.form.submit()" {
                    option value="" { "All servers" }
                    @for s in state.visible_servers(&user) {
                        option value=(s.id) selected[q.server.as_deref() == Some(s.id.as_str())] { (names(&s.id)) }
                    }
                }
                select name="show" onchange="this.form.submit()" {
                    option value="moderation" selected[moderation_only] { "Bans, kicks, warnings and dashboard actions" }
                    option value="all" selected[!moderation_only] { "Everything (restarts, announcements too)" }
                }
                noscript { button type="submit" { "Show" } }
            }

            h2 { "Online now" }
            @for s in &servers {
                @let status = state.ctx.monitor.status(&s.id);
                @let players = status.as_ref().map(|st| st.players.clone()).unwrap_or_default();
                @let can_kick = user.can(KICK) && control::route(s, "kick", relay, now).is_some();
                @let can_ban = user.can(BAN) && (uses_ban_list(s) || control::route(s, "ban", relay, now).is_some());
                section.card {
                    h3 { (names(&s.id)) " " small.muted { (online_line(status.as_ref())) } }
                    @if players.is_empty() {
                        p.muted { @if status.as_ref().and_then(|st| st.players_online).unwrap_or(0) > 0 { "This server doesn't share player names." } @else { "Nobody online." } }
                    } @else {
                        div.table-wrap { table {
                            thead { tr { th { "Player" } th.num { "Ping" } @if can_kick || can_ban { th { "Action" } } } }
                            tbody { @for p in &players { tr {
                                td { (p.name) @if let Some(id) = &p.id { " " small.muted { "#" (id) } } }
                                td.num { @if let Some(ms) = p.ping_ms { (ms) " ms" } }
                                @if can_kick || can_ban { td {
                                    details.inline-action {
                                        summary { "Kick or ban" }
                                        form method="post" action={ "/moderation/" (s.id) "/kick" } {
                                            input type="hidden" name="player" value=(p.id.clone().unwrap_or_else(|| p.name.clone()));
                                            input type="hidden" name="name" value=(p.name);
                                            input name="reason" maxlength="200" placeholder="Reason (shown to the player)";
                                            @if can_kick { button type="submit" { "Kick" } }
                                            @if can_ban {
                                                select name="duration" { @for (secs, label) in DURATIONS { option value=(secs) selected[secs == 86_400] { (label) } } }
                                                @if uses_ban_list(s) { (scope_select(s)) }
                                                @for id in &p.ids { input type="hidden" name="ids" value=(id); }
                                                button.danger type="submit" formaction={ "/moderation/" (s.id) "/ban" } { "Ban" }
                                            }
                                        }
                                    }
                                } }
                            } } }
                        } }
                    }
                }
            }

            @if any_ban_list && user.can(BAN) {
                h2 { "Ban someone who is offline" }
                p.muted { "FiveM and RedM: ban by identifier (" code { "license:…" } ", " code { "discord:…" } ", " code { "steam:…" } ", " code { "fivem:…" } "), one per line. txAdmin's player page lists them." }
                form.stack method="post" action={ "/moderation/" (offline_target) "/ban" } {
                    input name="name" maxlength="64" placeholder="Player name (for the record)";
                    textarea name="id_lines" rows="3" placeholder="license:0123abcd…" required {}
                    input name="reason" maxlength="200" placeholder="Reason (shown to the player)" required;
                    div.inline {
                        select name="duration" { @for (secs, label) in DURATIONS { option value=(secs) selected[secs == 0] { (label) } } }
                        select name="scope" {
                            option value="all" { "Every FiveM/RedM server" }
                            @for s in servers.iter().filter(|s| uses_ban_list(s)) { option value=(s.id) { "Only " (names(&s.id)) } }
                        }
                        button.danger type="submit" { "Ban" }
                    }
                }
            }

            @if any_ban_list || !bans.is_empty() {
                h2 { "Lighthouse bans" }
                @if bans.is_empty() { p.muted { "None yet." } } @else {
                    div.table-wrap { table {
                        thead { tr { th { "Player" } th { "Reason" } th { "Where" } th { "By" } th { "Until" } th {} } }
                        tbody { @for b in &bans { tr class=[(!b.is_active(now)).then_some("faded")] {
                            td { (b.ban.player.as_deref().unwrap_or("(unnamed)")) br; small.muted { (b.ban.ids.join(", ")) } }
                            td { (b.ban.reason) }
                            td { @match &b.server_id { Some(id) => (names(id)), None => "All FiveM/RedM servers" } }
                            td { (b.ban.author) br; small.muted { (fmt_utc(b.ban.created_unix)) } }
                            td {
                                @if let Some(t) = b.revoked_unix { "Lifted " (fmt_utc(t)) @if let Some(by) = &b.revoked_by { " by " (by) } }
                                @else if let Some(t) = b.ban.expires_unix { @if t > now { "in " (fmt_duration(t - now)) } @else { "Ran out " (fmt_utc(t)) } }
                                @else { "Permanent" }
                            }
                            td { @if b.is_active(now) && user.can(BAN) {
                                form method="post" action={ "/moderation/bans/" (b.ban.id) "/lift" } onsubmit="return confirm('Lift this ban?')" {
                                    button type="submit" { "Lift" }
                                }
                            } }
                        } } }
                    } }
                }
            }

            h2 { "Activity" }
            @if events.is_empty() { p.muted { "Nothing recorded yet. Events arrive from server mods (and txAdmin through the FiveM/RedM resource) and from actions taken here." } } @else {
                div.table-wrap { table {
                    thead { tr { th { "When" } th { "Server" } th { "What" } th { "Player" } th { "By" } th { "Details" } } }
                    tbody { @for e in &events {
                        @let ev = &e.event;
                        @let lifted = ev.action_id.as_ref().is_some_and(|a| ev.kind != "revoke" && revoked.contains(a));
                        tr class=[lifted.then_some("faded")] {
                            td { (fmt_utc(ev.at_unix.unwrap_or_default())) }
                            td { (names(&e.server_id)) }
                            td { span class={ "pill " (kind_class(&ev.kind)) } { (kind_label(&ev.kind)) } @if lifted { " " small.muted { "(lifted)" } } }
                            td { (ev.player.as_deref().unwrap_or("")) }
                            td { (ev.author.as_deref().unwrap_or("")) }
                            td {
                                (ev.reason.as_deref().unwrap_or(""))
                                @if ev.kind == "ban" { @match ev.expires_unix { Some(t) => { " · until " (fmt_utc(t)) }, None => " · permanent" } }
                                @if let Some(a) = &ev.action_id { " " small.muted { (a) } }
                            }
                        }
                    } }
                } }
            }
        },
    )
}

fn online_line(status: Option<&ServerStatus>) -> String {
    match status.and_then(|s| s.players_online) {
        Some(n) => format!("{n} online"),
        None => String::new(),
    }
}

fn scope_select(s: &ServerConfig) -> Markup {
    html! {
        select name="scope" {
            option value=(s.id) { "This server" }
            option value="all" { "Every FiveM/RedM server" }
        }
    }
}

#[derive(Default)]
struct ModForm {
    /// Player id (session id) or name, for the game.
    player: Option<String>,
    /// Display name.
    name: Option<String>,
    reason: Option<String>,
    duration: Option<i64>,
    scope: Option<String>,
    /// Identifiers from hidden fields (online players), one per field.
    ids: Vec<String>,
    /// Identifiers typed in, one per line.
    id_lines: Option<String>,
}

/// Form posts with repeated `ids` fields: collect them by hand (serde_urlencoded can't).
fn parse_form(body: &str) -> ModForm {
    let mut f = ModForm::default();
    for (k, v) in form_pairs(body) {
        match k.as_str() {
            "player" => f.player = Some(v),
            "name" => f.name = Some(v),
            "reason" => f.reason = Some(v),
            "duration" => f.duration = v.parse().ok(),
            "scope" => f.scope = Some(v),
            "ids" => f.ids.push(v),
            "id_lines" => f.id_lines = Some(v),
            _ => {}
        }
    }
    f
}

pub(crate) fn form_pairs(body: &str) -> Vec<(String, String)> {
    fn decode(s: &str) -> String {
        let bytes = s.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'+' => out.push(b' '),
                b'%' if i + 2 < bytes.len() => {
                    match u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("zz"), 16) {
                        Ok(b) => {
                            out.push(b);
                            i += 2;
                        }
                        Err(_) => out.push(b'%'),
                    }
                }
                b => out.push(b),
            }
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }
    body.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| {
            let (k, v) = p.split_once('=').unwrap_or((p, ""));
            (decode(k), decode(v))
        })
        .collect()
}

fn clean(s: Option<String>) -> Option<String> {
    s.map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

async fn kick(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
    body: String,
) -> Response {
    let Some(server) = find_server(&state, &user, &id) else {
        return forbidden(&state, &user);
    };
    if !user.can(KICK) {
        return forbidden(&state, &user);
    }
    let f = parse_form(&body);
    let name = clean(f.name.clone()).or_else(|| clean(f.player.clone()));
    let Some(action) = ControlAction::from_key("kick", f.reason.clone(), f.player.clone()) else {
        return redirect_with("/moderation", &Err("Pick a player first.".to_string()));
    };
    let reason = match &action {
        ControlAction::Kick { reason, .. } => reason.clone(),
        _ => String::new(),
    };
    let result = control::perform(server, action, state.ctx.monitor.relay(), now_unix()).await;
    tracing::info!(server = %server.id, user = %user.username, player = ?name, ?result, "kick");
    if result.is_ok() {
        log_action(
            &state,
            server,
            &user.username,
            ServerEvent {
                kind: "kick".into(),
                player: name.clone(),
                reason: Some(reason),
                ..Default::default()
            },
        );
    }
    redirect_with(
        "/moderation",
        &result.map(|_| format!("Kicked {}.", name.unwrap_or_default())),
    )
}

async fn ban(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path(id): Path<String>,
    body: String,
) -> Response {
    let Some(server) = find_server(&state, &user, &id) else {
        return forbidden(&state, &user);
    };
    if !user.can(BAN) {
        return forbidden(&state, &user);
    }
    let now = now_unix();
    let f = parse_form(&body);
    let name = clean(f.name.clone()).or_else(|| clean(f.player.clone()));
    let reason = clean(f.reason.clone()).unwrap_or_else(|| "Banned by an admin".to_string());
    let duration = f.duration.filter(|d| *d > 0);
    let expires = duration.map(|d| now + d);

    let result: Result<String, String> = if uses_ban_list(server) {
        let mut ids: Vec<String> = f
            .ids
            .iter()
            .map(|i| i.trim().to_string())
            .filter(|i| !i.is_empty())
            .collect();
        if let Some(lines) = &f.id_lines {
            ids.extend(
                lines
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .map(str::to_string),
            );
        }
        ids.sort();
        ids.dedup();
        // A ban list scope: one server, or every FiveM/RedM server.
        let scope = match f.scope.as_deref() {
            Some("all") | None => None,
            Some(other) => match find_server(&state, &user, other) {
                Some(s) if uses_ban_list(s) => Some(s.id.clone()),
                _ => return forbidden(&state, &user),
            },
        };
        if scope.is_none()
            && state
                .ctx
                .config
                .servers
                .iter()
                .any(|s| uses_ban_list(s) && !user.sees_server(&s.id))
        {
            Err("You can only ban on the servers you can see; pick one server.".to_string())
        } else if ids.is_empty() {
            Err(
                "No identifiers for that player yet. The Lighthouse resource sends them; or ban by identifier below."
                    .to_string(),
            )
        } else {
            let new = NewBan {
                player: name.clone(),
                ids,
                reason: reason.clone(),
                author: user.username.clone(),
                created_unix: now,
                expires_unix: expires,
            };
            state
                .ctx
                .store
                .add_ban(scope.as_deref(), &new)
                .map(|id| {
                    format!(
                        "Banned {} (ban #{id}). The server's resource applies it within about 30 seconds.",
                        name.clone().unwrap_or_default()
                    )
                })
                .map_err(|e| e.to_string())
        }
    } else {
        match clean(f.player.clone()) {
            None => Err("Pick a player first.".to_string()),
            Some(player) => {
                let action = ControlAction::Ban {
                    player,
                    reason: reason.clone(),
                    duration_secs: duration,
                };
                control::perform(server, action, state.ctx.monitor.relay(), now).await
            }
        }
    };
    tracing::info!(server = %server.id, user = %user.username, player = ?name, ?result, "ban");
    if result.is_ok() {
        log_action(
            &state,
            server,
            &user.username,
            ServerEvent {
                kind: "ban".into(),
                player: name,
                reason: Some(reason),
                expires_unix: expires,
                ..Default::default()
            },
        );
    }
    redirect_with("/moderation", &result)
}

async fn lift(State(state): State<AppState>, CurrentUser(user): CurrentUser, Path(ban_id): Path<i64>) -> Response {
    if !user.can(BAN) {
        return forbidden(&state, &user);
    }
    let visible: Vec<&str> = state.visible_servers(&user).iter().map(|s| s.id.as_str()).collect();
    let Some(record) = state
        .ctx
        .store
        .bans(&visible, usize::MAX)
        .ok()
        .and_then(|b| b.into_iter().find(|b| b.ban.id == ban_id))
    else {
        return forbidden(&state, &user);
    };
    // Network-wide bans can only be lifted by someone who sees every server they cover.
    if record.server_id.is_none()
        && state
            .ctx
            .config
            .servers
            .iter()
            .any(|s| uses_ban_list(s) && !user.sees_server(&s.id))
    {
        return forbidden(&state, &user);
    }
    let result = state
        .ctx
        .store
        .revoke_ban(ban_id, &user.username, now_unix())
        .map_err(|e| e.to_string())
        .and_then(|lifted| {
            if lifted {
                Ok(format!("Lifted ban #{ban_id}."))
            } else {
                Err("That ban was already lifted.".to_string())
            }
        });
    if result.is_ok() {
        let server = record.server_id.clone().or_else(|| {
            visible
                .iter()
                .find(|id| {
                    state
                        .ctx
                        .config
                        .servers
                        .iter()
                        .any(|s| s.id == **id && uses_ban_list(s))
                })
                .map(|s| s.to_string())
        });
        if let Some(server) = server.and_then(|id| state.ctx.config.servers.iter().find(|s| s.id == id)) {
            log_action(
                &state,
                server,
                &user.username,
                ServerEvent {
                    kind: "revoke".into(),
                    player: record.ban.player.clone(),
                    reason: Some(format!("Lifted Lighthouse ban #{ban_id}")),
                    ..Default::default()
                },
            );
        }
    }
    tracing::info!(user = %user.username, ban_id, ?result, "lift ban");
    redirect_with("/moderation", &result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_form_fields() {
        let f =
            parse_form("player=7&name=Bob+B&ids=license%3Aabc&ids=discord%3A1&duration=3600&scope=all&id_lines=x%0Ay");
        assert_eq!(f.player.as_deref(), Some("7"));
        assert_eq!(f.name.as_deref(), Some("Bob B"));
        assert_eq!(f.ids, ["license:abc", "discord:1"]);
        assert_eq!(f.duration, Some(3600));
        assert_eq!(f.id_lines.as_deref(), Some("x\ny"));
        assert_eq!(form_pairs("a=%zz&b=100%")[0].1, "%zz");
    }
}
