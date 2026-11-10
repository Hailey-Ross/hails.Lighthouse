//! Server controls: start, stop, restart, and messages to everyone on a server.
//!
//! What each server offers depends on how Lighthouse can reach it (see
//! `lighthouse_games::control`): commands on this machine, the server's mod, or the game console.
//! Every action is written to the server's activity history with who did it.

use axum::Router;
use axum::extract::{Form, Path, Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use lighthouse_core::{ControlAction, ServerConfig, ServerEvent};
use lighthouse_games::control::{self, Via};
use maud::{Markup, html};
use serde::Deserialize;

use super::{server_name, state_badge};
use crate::auth::{CurrentUser, forbidden};
use crate::module::{Module, PermissionDef};
use crate::util::{fmt_duration, now_unix};
use crate::{AppState, layout};

pub(crate) const VIEW: &str = "controls.view";
pub(crate) const POWER: &str = "controls.power";
pub(crate) const SAY: &str = "controls.say";

pub struct Controls;

impl Module for Controls {
    fn id(&self) -> &'static str {
        "controls"
    }
    fn title(&self) -> &'static str {
        "Controls"
    }
    fn icon(&self) -> &'static str {
        // power symbol
        "M12 3v8 M7.5 6.2a7 7 0 1 0 9 0"
    }
    fn view_permission(&self) -> &'static str {
        VIEW
    }
    fn permissions(&self) -> Vec<PermissionDef> {
        vec![
            PermissionDef {
                key: VIEW,
                label: "See server controls",
            },
            PermissionDef {
                key: POWER,
                label: "Start, stop and restart servers",
            },
            PermissionDef {
                key: SAY,
                label: "Send messages to everyone on a server",
            },
        ]
    }
    fn routes(&self) -> Router<AppState> {
        Router::new().route("/", get(page)).route("/{id}/{action}", post(act))
    }
}

/// Which permission an action needs.
pub(crate) fn permission_for(key: &str) -> &'static str {
    match key {
        "say" => SAY,
        "kick" => super::moderation::KICK,
        "ban" => super::moderation::BAN,
        _ => POWER,
    }
}

#[derive(Deserialize)]
pub(crate) struct Flash {
    pub ok: Option<String>,
    pub err: Option<String>,
}

pub(crate) fn flash(f: &Flash) -> Markup {
    html! {
        @if let Some(m) = &f.ok { p.notice { (m) } }
        @if let Some(m) = &f.err { p.error { (m) } }
    }
}

pub(crate) fn redirect_with(path: &str, result: &Result<String, String>) -> Response {
    let (k, v) = match result {
        Ok(m) => ("ok", m.as_str()),
        Err(m) => ("err", m.as_str()),
    };
    Redirect::to(&format!("{path}?{k}={}", urlencode(v))).into_response()
}

pub(crate) fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn confirm(question: &str) -> String {
    format!(
        "return confirm({})",
        serde_json::to_string(question).unwrap_or_default()
    )
}

fn via_note(via: Via) -> &'static str {
    match via {
        Via::Local => "Runs the command set in the config on this machine.",
        Via::Mod => "Handed to the server's mod with its next heartbeat (within about 30 seconds).",
        Via::WebApi => "Sent through the 7 Days to Die web API.",
        Via::Telnet => "Sent over the 7 Days to Die telnet console.",
        Via::Rcon => "Sent over RCON.",
    }
}

async fn page(State(state): State<AppState>, CurrentUser(user): CurrentUser, Query(f): Query<Flash>) -> Markup {
    let now = now_unix();
    let relay = state.ctx.monitor.relay();
    let servers = state.visible_servers(&user);
    layout::page(
        &state,
        &user,
        "controls",
        "Controls",
        html! {
            (flash(&f))
            @if servers.is_empty() { p.muted { "No servers to show." } }
            div.cards {
                @for s in &servers {
                    @let status = state.ctx.monitor.status(&s.id);
                    @let name = server_name(s, status.as_ref());
                    @let routes: Vec<(&str, Via)> = control::ACTIONS.iter()
                        .filter(|k| !matches!(**k, "kick" | "ban"))
                        .filter_map(|k| control::route(s, k, relay, now).map(|v| (*k, v)))
                        .filter(|(k, _)| user.can(permission_for(k)))
                        .collect();
                    section.card {
                        h3 { (name) }
                        p {
                            @if let Some(st) = &status { (state_badge(st.state)) }
                            @if let Some(up) = status.as_ref().and_then(|st| st.uptime_secs(now)) { " · up " (fmt_duration(up as i64)) }
                        }
                        @if routes.is_empty() {
                            p.muted { "No controls set up for this server. See “Server controls” in the README." }
                        } @else {
                            div.actions {
                                @for (key, via) in routes.iter().filter(|(k, _)| *k != "say") {
                                    form.inline method="post" action={ "/controls/" (s.id) "/" (key) }
                                        onsubmit=(confirm(&format!("{} {name}?", title_case(key)))) {
                                        button type="submit" title=(via_note(*via)) class={ "btn " @if *key == "stop" { "danger" } } { (title_case(key)) }
                                    }
                                }
                            }
                            @if let Some((_, via)) = routes.iter().find(|(k, _)| *k == "say") {
                                form.inline method="post" action={ "/controls/" (s.id) "/say" } title=(via_note(*via)) {
                                    input name="message" maxlength="200" placeholder="Message to everyone on the server" required;
                                    button type="submit" { "Send" }
                                }
                            }
                            p.muted.small { @for (i, (key, via)) in routes.iter().enumerate() { @if i > 0 { " · " } (title_case(key)) ": " (via.label()) } }
                        }
                    }
                }
            }
            p.muted {
                "Restarting through a mod or the game console shuts the server down cleanly; your host's panel, "
                "txAdmin or a service manager has to start it again. Every action is listed under Moderation › Activity."
            }
        },
    )
}

fn title_case(key: &str) -> String {
    let mut c = key.chars();
    c.next()
        .map(|f| f.to_uppercase().chain(c).collect())
        .unwrap_or_default()
}

#[derive(Deserialize)]
struct ActForm {
    message: Option<String>,
}

pub(crate) fn find_server<'a>(
    state: &'a AppState,
    user: &lighthouse_store::User,
    id: &str,
) -> Option<&'a ServerConfig> {
    state
        .ctx
        .config
        .servers
        .iter()
        .find(|s| s.id == id && user.sees_server(&s.id))
}

/// Record an action taken from the dashboard in the server's activity history.
pub(crate) fn log_action(state: &AppState, server: &ServerConfig, user: &str, event: ServerEvent) {
    let now = now_unix();
    let event = ServerEvent {
        at_unix: Some(now),
        author: Some(user.to_string()),
        ..event
    };
    if let Err(e) = state.ctx.store.add_events(&server.id, &[event], now) {
        tracing::warn!("could not save the action to the history: {e}");
    }
}

async fn act(
    State(state): State<AppState>,
    CurrentUser(user): CurrentUser,
    Path((id, key)): Path<(String, String)>,
    Form(form): Form<ActForm>,
) -> Response {
    let Some(server) = find_server(&state, &user, &id) else {
        return forbidden(&state, &user);
    };
    if matches!(key.as_str(), "kick" | "ban") || !user.can(permission_for(&key)) {
        return forbidden(&state, &user);
    }
    let Some(action) = ControlAction::from_key(&key, form.message, None) else {
        return redirect_with("/controls", &Err("Write a message first.".to_string()));
    };
    let detail = match &action {
        ControlAction::Say { message } => Some(message.clone()),
        _ => None,
    };
    let result = control::perform(server, action.clone(), state.ctx.monitor.relay(), now_unix()).await;
    tracing::info!(server = %server.id, user = %user.username, action = %key, ?result, "server control");
    log_action(
        &state,
        server,
        &user.username,
        ServerEvent {
            kind: "control".into(),
            reason: Some(match (&result, detail) {
                (Ok(_), Some(d)) => format!("{}: {d}", action.done_label()),
                (Ok(_), None) => action.done_label().to_string(),
                (Err(e), _) => format!("{} failed: {e}", title_case(&key)),
            }),
            ..Default::default()
        },
    );
    let name = server_name(server, state.ctx.monitor.status(&server.id).as_ref());
    redirect_with(
        "/controls",
        &result
            .map(|m| format!("{name}: {m}"))
            .map_err(|e| format!("{name}: {e}")),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(urlencode("a b&c/é"), "a%20b%26c%2F%C3%A9");
        assert_eq!(title_case("restart"), "Restart");
        assert_eq!(confirm("Stop \"X\"?"), r#"return confirm("Stop \"X\"?")"#);
        assert_eq!(permission_for("start"), POWER);
        assert_eq!(permission_for("say"), SAY);
    }
}
