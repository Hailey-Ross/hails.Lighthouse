//! Login, sessions and access checks.
//!
//! Sessions are random tokens stored in the database and sent as an `HttpOnly`, `SameSite=Strict`
//! cookie. Form posts must come from the dashboard itself (Origin/Referer check), which together
//! with `SameSite=Strict` blocks cross-site request forgery.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::Form;
use axum::extract::{ConnectInfo, FromRequestParts, Request, State};
use axum::http::request::Parts;
use axum::http::{HeaderMap, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Redirect, Response};
use lighthouse_store::User;
use maud::html;
use serde::Deserialize;

use crate::util::now_unix;
use crate::{AppState, layout};

const COOKIE: &str = "lighthouse_session";
const SESSION_TTL: i64 = 7 * 24 * 3600;
/// Failed logins allowed per address per window.
const MAX_FAILURES: u32 = 5;
const FAILURE_WINDOW: Duration = Duration::from_secs(10 * 60);

/// The signed-in user. Extracting it redirects to the login page when nobody is signed in.
pub struct CurrentUser(pub User);

fn session_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|kv| kv.trim().strip_prefix(&format!("{COOKIE}=")).map(str::to_string))
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let user =
            session_token(&parts.headers).and_then(|t| state.ctx.store.session_user(&t, now_unix()).ok().flatten());
        match user {
            Some(u) => Ok(CurrentUser(u)),
            None => Err(Redirect::to("/login").into_response()),
        }
    }
}

/// Route layer: the signed-in user must hold the module's view permission.
pub async fn require_permission(
    State((state, permission)): State<(AppState, &'static str)>,
    request: Request,
    next: Next,
) -> Response {
    let (mut parts, body) = request.into_parts();
    let user = match CurrentUser::from_request_parts(&mut parts, &state).await {
        Ok(u) => u.0,
        Err(redirect) => return redirect,
    };
    if !user.can(permission) {
        return forbidden(&state, &user);
    }
    next.run(Request::from_parts(parts, body)).await
}

pub fn forbidden(state: &AppState, user: &User) -> Response {
    (
        StatusCode::FORBIDDEN,
        layout::page(
            state,
            user,
            "",
            "No access",
            html! {
                p { "You don't have access to this page. Ask the superadmin if you need it." }
            },
        ),
    )
        .into_response()
}

/// The visitor's address: the connection's peer, or the first `X-Forwarded-For` entry when the
/// dashboard is configured to trust its reverse proxy.
pub(crate) fn client_ip(state: &AppState, headers: &HeaderMap, peer: Option<SocketAddr>) -> Option<IpAddr> {
    if state.ctx.config.dashboard.trust_proxy {
        let forwarded = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.split(',').next())
            .and_then(|v| v.trim().parse().ok());
        if forwarded.is_some() {
            return forwarded;
        }
    }
    peer.map(|p| p.ip())
}

/// Blocked addresses get nothing but an explanation.
pub async fn reject_blocked(State(state): State<AppState>, request: Request, next: Next) -> Response {
    // Static files stay available so the block page itself is styled.
    if request.uri().path().starts_with("/static/") {
        return next.run(request).await;
    }
    let peer = request.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0);
    if let Some(ip) = client_ip(&state, request.headers(), peer)
        && let Ok(Some(block)) = state.ctx.store.active_block(&ip.to_string(), now_unix())
    {
        return blocked_page(&block);
    }
    next.run(request).await
}

pub(crate) fn blocked_page(block: &lighthouse_store::IpBlock) -> Response {
    let until = if block.permanent {
        "until an administrator lifts the block".to_string()
    } else {
        block
            .blocked_until
            .map(|t| format!("for {}", crate::util::fmt_duration(t - now_unix())))
            .unwrap_or_default()
    };
    (
        StatusCode::FORBIDDEN,
        layout::bare(
            "Access blocked",
            html! {
                div.login {
                    div.brand { img.brand-mark src="/static/mark.svg" alt="" width="40" height="40"; "Lighthouse" }
                    p { "Too many wrong invite codes came from your address, so it is blocked " (until) "." }
                    p.muted { "If this is a mistake, ask your server admin to lift the block." }
                }
            },
        ),
    )
        .into_response()
}

/// Reject cross-site form posts.
pub async fn same_origin_posts(request: Request, next: Next) -> Response {
    if request.method() == Method::GET || request.method() == Method::HEAD {
        return next.run(request).await;
    }
    let headers = request.headers();
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default();
    let source = headers
        .get(header::ORIGIN)
        .or_else(|| headers.get(header::REFERER))
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default();
    let source_host = source
        .split("://")
        .nth(1)
        .unwrap_or_default()
        .split('/')
        .next()
        .unwrap_or_default();
    if host.is_empty() || source_host != host {
        return (StatusCode::FORBIDDEN, "Cross-site request blocked").into_response();
    }
    next.run(request).await
}

// ---------- login / logout ----------

#[derive(Default)]
pub struct LoginLimiter {
    failures: Mutex<HashMap<IpAddr, (u32, Instant)>>,
}

impl LoginLimiter {
    pub(crate) fn blocked(&self, ip: IpAddr) -> bool {
        let map = self.failures.lock().unwrap_or_else(|p| p.into_inner());
        map.get(&ip)
            .is_some_and(|(n, since)| *n >= MAX_FAILURES && since.elapsed() < FAILURE_WINDOW)
    }

    pub(crate) fn fail(&self, ip: IpAddr) {
        let mut map = self.failures.lock().unwrap_or_else(|p| p.into_inner());
        let entry = map.entry(ip).or_insert((0, Instant::now()));
        if entry.1.elapsed() >= FAILURE_WINDOW {
            *entry = (0, Instant::now());
        }
        entry.0 += 1;
    }

    pub(crate) fn clear(&self, ip: IpAddr) {
        self.failures.lock().unwrap_or_else(|p| p.into_inner()).remove(&ip);
    }
}

fn login_form(error: Option<&str>) -> Response {
    layout::bare(
        "Sign in",
        html! {
            form.login method="post" action="/login" {
                div.brand { img.brand-mark src="/static/mark.svg" alt="" width="40" height="40"; "Lighthouse" }
                p.muted { "Sign in to see your servers." }
                @if let Some(e) = error { p.notice.error role="alert" { (e) } }
                label { "Username" input name="username" autocomplete="username" required autofocus; }
                label { "Password" input type="password" name="password" autocomplete="current-password" required; }
                button.primary type="submit" { "Sign in" }
            }
        },
    )
    .into_response()
}

pub async fn login_page() -> Response {
    login_form(None)
}

#[derive(Deserialize)]
pub struct LoginForm {
    username: String,
    password: String,
}

pub async fn login(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Form(form): Form<LoginForm>,
) -> Response {
    let ip = addr.ip();
    if state.limiter.blocked(ip) {
        return login_form(Some("Too many failed sign-ins. Try again in a few minutes."));
    }
    let user = match state.ctx.store.verify_login(&form.username, &form.password) {
        Ok(Some(u)) => u,
        Ok(None) => {
            state.limiter.fail(ip);
            tracing::warn!(%ip, "failed dashboard sign-in for {:?}", form.username);
            return login_form(Some("Wrong username or password."));
        }
        Err(e) => {
            tracing::error!("sign-in failed: {e}");
            return login_form(Some("Something went wrong. Check the Lighthouse log."));
        }
    };
    state.limiter.clear(ip);
    let token = match state.ctx.store.create_session(user.id, now_unix(), SESSION_TTL) {
        Ok(t) => t,
        Err(e) => {
            tracing::error!("could not create session: {e}");
            return login_form(Some("Something went wrong. Check the Lighthouse log."));
        }
    };
    tracing::info!(%ip, "{} signed in to the dashboard", user.username);
    (
        [(
            header::SET_COOKIE,
            format!("{COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={SESSION_TTL}"),
        )],
        Redirect::to("/"),
    )
        .into_response()
}

/// Sign `user` in: create a session and send its cookie, then go to `/`.
pub(crate) fn start_session(state: &AppState, user: &User) -> Result<Response, lighthouse_store::StoreError> {
    let token = state.ctx.store.create_session(user.id, now_unix(), SESSION_TTL)?;
    Ok((
        [(
            header::SET_COOKIE,
            format!("{COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={SESSION_TTL}"),
        )],
        Redirect::to("/"),
    )
        .into_response())
}

pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Some(t) = session_token(&headers) {
        let _ = state.ctx.store.delete_session(&t);
    }
    (
        [(
            header::SET_COOKIE,
            format!("{COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0"),
        )],
        Redirect::to("/login"),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn reads_session_cookie() {
        let mut h = HeaderMap::new();
        h.insert(
            header::COOKIE,
            HeaderValue::from_static("a=1; lighthouse_session=abc123; b=2"),
        );
        assert_eq!(session_token(&h).as_deref(), Some("abc123"));
    }

    #[test]
    fn limiter_blocks_after_failures() {
        let l = LoginLimiter::default();
        let ip: IpAddr = [127, 0, 0, 1].into();
        for _ in 0..MAX_FAILURES {
            assert!(!l.blocked(ip));
            l.fail(ip);
        }
        assert!(l.blocked(ip));
        l.clear(ip);
        assert!(!l.blocked(ip));
    }
}
