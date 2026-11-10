//! Registration, by invitation only.
//!
//! There is no open sign-up. An admin creates an invite on the Users page (with the access the new
//! account will get); the link is single-use and expires. `/register` without a valid code shows
//! an explanation and no form.

use std::net::SocketAddr;

use axum::Form;
use axum::extract::{ConnectInfo, Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use lighthouse_store::Invite;
use maud::{Markup, html};
use serde::Deserialize;

use crate::util::{fmt_duration, now_unix};
use crate::{AppState, auth, layout};

#[derive(Deserialize)]
pub struct RegisterQuery {
    code: Option<String>,
}

#[derive(Deserialize)]
pub struct RegisterForm {
    code: String,
    username: String,
    password: String,
    confirm: String,
}

fn closed(message: &str) -> Response {
    layout::bare(
        "Create your account",
        html! {
            div.login {
                div.brand { img.brand-mark src="/static/mark.svg" alt="" width="40" height="40"; "Lighthouse" }
                p { (message) }
                p { a href="/login" { "Go to sign in" } }
            }
        },
    )
    .into_response()
}

const INVITE_ONLY: &str = "Lighthouse accounts are by invitation only. Ask your server admin for an invite link.";
const INVITE_GONE: &str = "This invite link has expired or was already used. Ask your server admin for a new one.";

fn form(state: &AppState, invite: &Invite, code: &str, error: Option<&str>, username: &str) -> Markup {
    let labels: Vec<&str> = state
        .all_permissions()
        .into_iter()
        .filter(|p| invite.permissions.contains(p.key))
        .map(|p| p.label)
        .collect();
    let servers = match &invite.servers {
        None => "all servers".to_string(),
        Some(s) if s.len() == 1 => "1 server".to_string(),
        Some(s) => format!("{} servers", s.len()),
    };
    layout::bare(
        "Create your account",
        html! {
            form.login method="post" action="/register" {
                div.brand { img.brand-mark src="/static/mark.svg" alt="" width="40" height="40"; "Lighthouse" }
                p.muted { "You've been invited to the dashboard. Pick a username and password to finish." }
                @if let Some(e) = error { p.notice.error role="alert" { (e) } }
                input type="hidden" name="code" value=(code);
                label { "Username" input name="username" value=(username) autocomplete="username" required minlength="2" maxlength="32" autofocus; }
                label { "Password (at least 8 characters)" input type="password" name="password" autocomplete="new-password" required minlength="8"; }
                label { "Repeat password" input type="password" name="confirm" autocomplete="new-password" required minlength="8"; }
                button.primary type="submit" { "Create account" }
                p.muted.small {
                    "Your access: "
                    @if labels.is_empty() { "none yet" } @else { (labels.join("; ")) }
                    ", on " (servers) ". This link expires in " (fmt_duration(invite.expires_at - now_unix())) "."
                }
            }
        },
    )
}

/// A wrong code: count it against the address, and show the block page if this one triggered it.
fn wrong_code(state: &AppState, ip: Option<std::net::IpAddr>) -> Response {
    if let Some(ip) = ip {
        match state.ctx.store.record_bad_code(&ip.to_string(), now_unix()) {
            Ok(Some(block)) => {
                tracing::warn!(%ip, strikes = block.strikes, "blocked for guessing invite codes");
                return auth::blocked_page(&block);
            }
            Ok(None) => {}
            Err(e) => tracing::error!("could not record a wrong invite code: {e}"),
        }
    }
    closed(INVITE_GONE)
}

pub async fn register_page(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Query(q): Query<RegisterQuery>,
) -> Response {
    let Some(code) = q.code.filter(|c| !c.is_empty()) else {
        return closed(INVITE_ONLY);
    };
    match state.ctx.store.invite(&code, now_unix()) {
        Ok(Some(invite)) => form(&state, &invite, &code, None, "").into_response(),
        _ => wrong_code(&state, auth::client_ip(&state, &headers, Some(addr))),
    }
}

pub async fn register(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(f): Form<RegisterForm>,
) -> Response {
    let client = auth::client_ip(&state, &headers, Some(addr));
    let ip = client.unwrap_or(addr.ip());
    let Ok(Some(invite)) = state.ctx.store.invite(&f.code, now_unix()) else {
        return wrong_code(&state, client);
    };
    if f.password != f.confirm {
        return form(
            &state,
            &invite,
            &f.code,
            Some("The two passwords don't match."),
            &f.username,
        )
        .into_response();
    }
    match state
        .ctx
        .store
        .redeem_invite(&f.code, &f.username, &f.password, now_unix())
    {
        Ok(user) => {
            tracing::info!(%ip, "{} registered with an invite", user.username);
            auth::start_session(&state, &user).unwrap_or_else(|e| {
                tracing::error!("could not create session: {e}");
                axum::response::Redirect::to("/login").into_response()
            })
        }
        Err(e) => form(&state, &invite, &f.code, Some(&e.to_string()), &f.username).into_response(),
    }
}
