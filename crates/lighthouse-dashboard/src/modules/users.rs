//! Dashboard accounts. The superadmin (created in the CLI setup) can do everything. Sub-admins get
//! exactly the permissions and servers ticked here.
//!
//! Anyone with `users.manage` who is not the superadmin can only hand out permissions and servers
//! they have themselves, cannot change their own access, and cannot touch the superadmin.

use std::collections::BTreeSet;

use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Form, Router};
use lighthouse_store::User;
use maud::{Markup, html};
use serde::Deserialize;

use crate::auth::{CurrentUser, forbidden};
use crate::module::{Module, PermissionDef};
use crate::util::{fmt_utc, now_unix};
use crate::{AppState, layout};

const MANAGE: &str = "users.manage";

pub struct Users;

impl Module for Users {
    fn id(&self) -> &'static str {
        "users"
    }
    fn title(&self) -> &'static str {
        "Users"
    }
    fn icon(&self) -> &'static str {
        "M9 11a4 4 0 1 0 0-8 4 4 0 0 0 0 8z M2 21c0-4 3-6 7-6s7 2 7 6 M16 4a3.5 3.5 0 0 1 0 7 M22 21c0-3-2-5-4.5-5.5"
    }
    fn view_permission(&self) -> &'static str {
        MANAGE
    }
    fn permissions(&self) -> Vec<PermissionDef> {
        vec![PermissionDef {
            key: MANAGE,
            label: "Manage dashboard users (only within their own access)",
        }]
    }
    fn routes(&self) -> Router<AppState> {
        Router::new()
            .route("/", get(list).post(create))
            .route("/invite", post(create_invite))
            .route("/invite/{code}/revoke", post(revoke_invite))
            .route("/{id}", get(edit))
            .route("/{id}/access", post(update_access))
            .route("/{id}/password", post(update_password))
            .route("/{id}/delete", post(delete))
    }
}

// ---------- form parsing ----------

/// Form fields arrive as repeated `perm=…` / `server=…` pairs, so parse them as a list.
struct AccessForm {
    username: String,
    password: String,
    permissions: BTreeSet<String>,
    all_servers: bool,
    servers: BTreeSet<String>,
}

fn parse(fields: Vec<(String, String)>) -> AccessForm {
    let mut f = AccessForm {
        username: String::new(),
        password: String::new(),
        permissions: BTreeSet::new(),
        all_servers: false,
        servers: BTreeSet::new(),
    };
    for (k, v) in fields {
        match k.as_str() {
            "username" => f.username = v,
            "password" => f.password = v,
            "perm" => {
                f.permissions.insert(v);
            }
            "scope" => f.all_servers = v == "all",
            "server" => {
                f.servers.insert(v);
            }
            _ => {}
        }
    }
    f
}

/// Limit requested access to what the acting user may grant. Returns (permissions, servers).
fn grantable(state: &AppState, actor: &User, form: &AccessForm) -> (BTreeSet<String>, Option<BTreeSet<String>>) {
    let known: BTreeSet<&str> = state.all_permissions().iter().map(|p| p.key).collect();
    let permissions = form
        .permissions
        .iter()
        .filter(|p| known.contains(p.as_str()) && actor.can(p))
        .cloned()
        .collect();
    let all_ids: BTreeSet<String> = state.ctx.config.servers.iter().map(|s| s.id.clone()).collect();
    let servers = if form.all_servers && actor.servers.is_none() {
        None
    } else {
        Some(
            form.servers
                .iter()
                .filter(|s| all_ids.contains(*s) && actor.sees_server(s))
                .cloned()
                .collect(),
        )
    };
    (permissions, servers)
}

/// May `actor` change `target`'s access or delete them?
fn may_manage(actor: &User, target: &User) -> bool {
    !target.is_superadmin && (actor.is_superadmin || actor.id != target.id)
}

// ---------- pages ----------

/// Human list of a user's permissions, using each module's label.
fn access_summary(labels: &[PermissionDef], user: &User) -> String {
    let mut out: Vec<&str> = labels
        .iter()
        .filter(|p| user.permissions.contains(p.key))
        .map(|p| p.label)
        .collect();
    out.dedup();
    let unknown = user
        .permissions
        .iter()
        .filter(|k| !labels.iter().any(|p| p.key == k.as_str()))
        .count();
    let mut text = out.join("; ");
    if unknown > 0 {
        text.push_str(&format!(" (+{unknown} from removed pages)"));
    }
    text
}

#[derive(Deserialize)]
struct Flash {
    msg: Option<String>,
    err: Option<String>,
    /// Code of an invite that was just created, to show its link.
    invite: Option<String>,
}

fn flash(f: &Flash) -> Markup {
    let msg = match f.msg.as_deref() {
        Some("created") => Some("User created."),
        Some("saved") => Some("Changes saved."),
        Some("password") => Some("Password changed. That user has been signed out everywhere."),
        Some("deleted") => Some("User deleted."),
        Some("revoked") => Some("Invite cancelled. Its link no longer works."),
        _ => None,
    };
    html! {
        @if let Some(m) = msg { p.notice { (m) } }
        @if let Some(e) = &f.err { p.notice.error { (e) } }
    }
}

fn access_fields(state: &AppState, actor: &User, current: Option<&User>) -> Markup {
    let perms = state.all_permissions();
    let all_servers = current.is_none_or(|u| u.servers.is_none());
    html! {
        fieldset {
            legend { "Pages and actions" }
            @for p in perms.iter().filter(|p| actor.can(p.key)) {
                label.check {
                    input type="checkbox" name="perm" value=(p.key)
                        checked[current.is_some_and(|u| u.permissions.contains(p.key))];
                    (p.label) span.muted { " (" (p.key) ")" }
                }
            }
        }
        fieldset {
            legend { "Servers" }
            @if actor.servers.is_none() {
                label.check { input type="radio" name="scope" value="all" checked[all_servers]; "All servers (including ones added later)" }
                label.check { input type="radio" name="scope" value="some" checked[!all_servers]; "Only these:" }
            } @else {
                input type="hidden" name="scope" value="some";
            }
            @for s in state.ctx.config.servers.iter().filter(|s| actor.sees_server(&s.id)) {
                label.check {
                    input type="checkbox" name="server" value=(s.id)
                        checked[current.is_some_and(|u| u.servers.as_ref().is_some_and(|set| set.contains(&s.id)))];
                    (s.name.as_deref().unwrap_or(&s.id)) span.muted { " (" (s.game) ")" }
                }
            }
        }
    }
}

async fn list(
    State(state): State<AppState>,
    CurrentUser(actor): CurrentUser,
    headers: HeaderMap,
    Query(f): Query<Flash>,
) -> Markup {
    let users = state.ctx.store.users().unwrap_or_default();
    let labels = state.all_permissions();
    let now = now_unix();
    let invites = state.ctx.store.open_invites(now).unwrap_or_default();
    let base = base_url(&headers);
    // The code is only in this one redirect URL; it isn't stored, so it can't be shown again later.
    let new_link = f
        .invite
        .as_ref()
        .filter(|c| c.len() == 64 && c.chars().all(|ch| ch.is_ascii_hexdigit()))
        .map(|c| format!("{base}/register?code={c}"));
    layout::page(
        &state,
        &actor,
        "users",
        "Users",
        html! {
            (flash(&f))
            div.table-wrap { table {
                thead { tr { th { "User" } th { "Access" } th { "Servers" } th { "Created" } } }
                tbody { @for u in &users { tr {
                    td {
                        @if may_manage(&actor, u) || u.id == actor.id { a href={ "/users/" (u.id) } { (u.username) } }
                        @else { (u.username) }
                    }
                    td { @if u.is_superadmin { "Everything (superadmin)" }
                     @else if u.permissions.is_empty() { span.muted { "Nothing yet" } }
                     @else { (access_summary(&labels, u)) } }
                    td { @match &u.servers { None => "All", Some(s) => (s.len()) } }
                    td.muted { (fmt_utc(u.created_at)) }
                } } }
            } }
            h2 { "Add a user" }
            form.user-form method="post" action="/users" {
                label { "Username" input name="username" required minlength="2" maxlength="32" pattern="[A-Za-z0-9._\\-]+"; }
                label { "Password (at least 8 characters)" input type="password" name="password" required minlength="8" autocomplete="new-password"; }
                (access_fields(&state, &actor, None))
                button.primary type="submit" { "Create user" }
            }

            h2 id="invite" { "Invite someone" }
            p.muted { "Send a link instead of choosing a password for them. Each link works once and then expires. "
                      "There is no other way to sign up." }
            @if let Some(link) = &new_link {
                div.notice.invite-link {
                    p { "Invite created. Copy the link now and send it to the person you're inviting. For safety it isn't stored, so it can't be shown again." }
                    div.inline {
                        input id="invite-url" type="text" readonly value=(link) aria-label="Invite link";
                        button.primary type="button" onclick="var i=document.getElementById('invite-url');i.select();navigator.clipboard&&navigator.clipboard.writeText(i.value);this.textContent='Copied'" { "Copy link" }
                    }
                }
            }
            form.user-form method="post" action="/users/invite" {
                (access_fields(&state, &actor, None))
                label { "Link works for"
                    select name="valid" {
                        option value="86400" { "1 day" }
                        option value="259200" { "3 days" }
                        option value="604800" selected { "7 days" }
                    }
                }
                button.primary type="submit" { "Create invite link" }
            }
            @if !invites.is_empty() {
                h2 { "Open invites" }
                div.table-wrap { table {
                    thead { tr { th { "Access" } th { "Servers" } th { "Expires" } th {} } }
                    tbody { @for i in &invites { tr {
                        td { @if i.permissions.is_empty() { span.muted { "Nothing yet" } }
                             @else { (access_summary(&labels, &User { id: 0, username: String::new(), is_superadmin: false,
                                       permissions: i.permissions.clone(), servers: None, created_at: 0 })) } }
                        td { @match &i.servers { None => "All", Some(s) => (s.len()) } }
                        td { "in " (crate::util::fmt_duration(i.expires_at - now)) }
                        td { form method="post" action={ "/users/invite/" (i.id) "/revoke" } {
                            button.quiet type="submit" { "Cancel invite" } } }
                    } } }
                } }
            }
        },
    )
}

async fn create(
    State(state): State<AppState>,
    CurrentUser(actor): CurrentUser,
    Form(fields): Form<Vec<(String, String)>>,
) -> Response {
    let form = parse(fields);
    let (perms, servers) = grantable(&state, &actor, &form);
    match state.ctx.store.create_user(
        &form.username,
        &form.password,
        false,
        &perms,
        servers.as_ref(),
        now_unix(),
    ) {
        Ok(u) => {
            tracing::info!("{} created dashboard user {}", actor.username, u.username);
            Redirect::to("/users?msg=created").into_response()
        }
        Err(e) => Redirect::to(&format!("/users?err={}", urlencode(&e.to_string()))).into_response(),
    }
}

async fn edit(
    State(state): State<AppState>,
    CurrentUser(actor): CurrentUser,
    Path(id): Path<i64>,
    Query(f): Query<Flash>,
) -> Response {
    let Some(target) = state.ctx.store.user(id).ok().flatten() else {
        return Redirect::to("/users").into_response();
    };
    let own = target.id == actor.id;
    if !may_manage(&actor, &target) && !own {
        return forbidden(&state, &actor);
    }
    layout::page(&state, &actor, "users", &target.username, html! {
        (flash(&f))
        @if may_manage(&actor, &target) {
            h2 { "Access" }
            form method="post" action={ "/users/" (target.id) "/access" } {
                (access_fields(&state, &actor, Some(&target)))
                button.primary type="submit" { "Save access" }
            }
        } @else if target.is_superadmin {
            p.muted { "The superadmin always has full access." }
        }
        h2 { "Password" }
        form method="post" action={ "/users/" (target.id) "/password" } {
            label { "New password (at least 8 characters)" input type="password" name="password" required minlength="8" autocomplete="new-password"; }
            button type="submit" { "Change password" }
        }
        @if may_manage(&actor, &target) {
            h2 { "Delete" }
            form method="post" action={ "/users/" (target.id) "/delete" } {
                button.danger type="submit" { "Delete " (target.username) }
            }
        }
        a.back href="/users" { "Back to all users" }
    })
    .into_response()
}

async fn update_access(
    State(state): State<AppState>,
    CurrentUser(actor): CurrentUser,
    Path(id): Path<i64>,
    Form(fields): Form<Vec<(String, String)>>,
) -> Response {
    let Some(target) = state.ctx.store.user(id).ok().flatten() else {
        return Redirect::to("/users").into_response();
    };
    if !may_manage(&actor, &target) {
        return forbidden(&state, &actor);
    }
    let form = parse(fields);
    let (mut perms, servers) = grantable(&state, &actor, &form);
    // Keep permissions the actor can't see or grant; they may only change what they hold themselves.
    perms.extend(target.permissions.iter().filter(|p| !actor.can(p)).cloned());
    match state.ctx.store.set_access(target.id, &perms, servers.as_ref()) {
        Ok(()) => {
            tracing::info!("{} changed access for {}", actor.username, target.username);
            Redirect::to(&format!("/users/{}?msg=saved", target.id)).into_response()
        }
        Err(e) => Redirect::to(&format!("/users/{}?err={}", target.id, urlencode(&e.to_string()))).into_response(),
    }
}

#[derive(Deserialize)]
struct PasswordForm {
    password: String,
}

async fn update_password(
    State(state): State<AppState>,
    CurrentUser(actor): CurrentUser,
    Path(id): Path<i64>,
    Form(form): Form<PasswordForm>,
) -> Response {
    let Some(target) = state.ctx.store.user(id).ok().flatten() else {
        return Redirect::to("/users").into_response();
    };
    if !(target.id == actor.id || may_manage(&actor, &target)) {
        return forbidden(&state, &actor);
    }
    match state.ctx.store.set_password(target.id, &form.password) {
        Ok(()) if target.id == actor.id => Redirect::to("/login").into_response(),
        Ok(()) => Redirect::to(&format!("/users/{}?msg=password", target.id)).into_response(),
        Err(e) => Redirect::to(&format!("/users/{}?err={}", target.id, urlencode(&e.to_string()))).into_response(),
    }
}

async fn delete(State(state): State<AppState>, CurrentUser(actor): CurrentUser, Path(id): Path<i64>) -> Response {
    let Some(target) = state.ctx.store.user(id).ok().flatten() else {
        return Redirect::to("/users").into_response();
    };
    if !may_manage(&actor, &target) || target.id == actor.id {
        return forbidden(&state, &actor);
    }
    match state.ctx.store.delete_user(target.id) {
        Ok(()) => {
            tracing::info!("{} deleted dashboard user {}", actor.username, target.username);
            Redirect::to("/users?msg=deleted").into_response()
        }
        Err(e) => Redirect::to(&format!("/users?err={}", urlencode(&e.to_string()))).into_response(),
    }
}

/// `http(s)://host` of this request, honouring a reverse proxy's X-Forwarded-Proto.
fn base_url(headers: &HeaderMap) -> String {
    let host = headers
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("localhost");
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|h| h.to_str().ok())
        .filter(|p| *p == "https")
        .unwrap_or("http");
    format!("{scheme}://{host}")
}

async fn create_invite(
    State(state): State<AppState>,
    CurrentUser(actor): CurrentUser,
    Form(fields): Form<Vec<(String, String)>>,
) -> Response {
    let valid = fields
        .iter()
        .find(|(k, _)| k == "valid")
        .and_then(|(_, v)| v.parse::<i64>().ok())
        .unwrap_or(604_800)
        .clamp(3600, 30 * 86_400);
    let form = parse(fields);
    let (perms, servers) = grantable(&state, &actor, &form);
    match state
        .ctx
        .store
        .create_invite(actor.id, &perms, servers.as_ref(), now_unix(), valid)
    {
        Ok((_, code)) => {
            tracing::info!("{} created an invite", actor.username);
            Redirect::to(&format!("/users?invite={code}#invite")).into_response()
        }
        Err(e) => Redirect::to(&format!("/users?err={}", urlencode(&e.to_string()))).into_response(),
    }
}

async fn revoke_invite(
    State(state): State<AppState>,
    CurrentUser(actor): CurrentUser,
    Path(code): Path<String>,
) -> Response {
    match state.ctx.store.revoke_invite(&code) {
        Ok(()) => {
            tracing::info!("{} cancelled an invite", actor.username);
            Redirect::to("/users?msg=revoked#invite").into_response()
        }
        Err(e) => Redirect::to(&format!("/users?err={}", urlencode(&e.to_string()))).into_response(),
    }
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            b' ' => "+".to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn user(id: i64, superadmin: bool, perms: &[&str], servers: Option<&[&str]>) -> User {
        User {
            id,
            username: format!("u{id}"),
            is_superadmin: superadmin,
            permissions: perms.iter().map(|p| p.to_string()).collect(),
            servers: servers.map(|s| s.iter().map(|x| x.to_string()).collect()),
            created_at: 0,
        }
    }

    #[test]
    fn who_may_manage_whom() {
        let root = user(1, true, &[], None);
        let mgr = user(2, false, &["users.manage"], None);
        let other = user(3, false, &[], None);
        assert!(may_manage(&root, &mgr));
        assert!(may_manage(&mgr, &other));
        assert!(!may_manage(&mgr, &root), "nobody manages the superadmin");
        assert!(!may_manage(&mgr, &mgr), "no self-escalation");
    }

    #[test]
    fn parses_repeated_fields() {
        let f = parse(vec![
            ("username".into(), "bob".into()),
            ("perm".into(), "logs.view".into()),
            ("perm".into(), "servers.view".into()),
            ("scope".into(), "some".into()),
            ("server".into(), "athens".into()),
        ]);
        assert_eq!(f.username, "bob");
        assert_eq!(f.permissions.len(), 2);
        assert!(!f.all_servers);
        assert!(f.servers.contains("athens"));
    }

    #[test]
    fn url_encoding() {
        assert_eq!(urlencode("a b&c"), "a+b%26c");
    }
}
