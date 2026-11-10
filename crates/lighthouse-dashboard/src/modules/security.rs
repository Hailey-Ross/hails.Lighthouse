//! Blocked addresses. Guessing invite codes gets an address blocked for 30 minutes, then 3 hours,
//! 7 days, 30 days, and finally until someone with this page's permission removes the block.

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use maud::{Markup, html};
use serde::Deserialize;

use crate::auth::CurrentUser;
use crate::module::{Module, PermissionDef};
use crate::util::{fmt_duration, fmt_utc, now_unix};
use crate::{AppState, layout};

const MANAGE: &str = "security.manage";

pub struct Security;

impl Module for Security {
    fn id(&self) -> &'static str {
        "security"
    }
    fn title(&self) -> &'static str {
        "Security"
    }
    fn icon(&self) -> &'static str {
        // shield
        "M12 3l7 3v5c0 4.5-3 8.3-7 10-4-1.7-7-5.5-7-10V6l7-3z M9.5 12l2 2 3.5-4"
    }
    fn view_permission(&self) -> &'static str {
        MANAGE
    }
    fn permissions(&self) -> Vec<PermissionDef> {
        vec![PermissionDef {
            key: MANAGE,
            label: "See and lift blocked addresses",
        }]
    }
    fn routes(&self) -> Router<AppState> {
        Router::new()
            .route("/", get(page))
            .route("/unblock/{ip}", post(unblock))
    }
}

#[derive(Deserialize)]
struct Flash {
    msg: Option<String>,
}

async fn page(State(state): State<AppState>, CurrentUser(user): CurrentUser, Query(f): Query<Flash>) -> Markup {
    let now = now_unix();
    let blocks = state.ctx.store.blocks().unwrap_or_default();
    let active = blocks.iter().filter(|b| b.is_blocked(now)).count();
    layout::page(
        &state,
        &user,
        "security",
        "Security",
        html! {
            @if f.msg.as_deref() == Some("unblocked") { p.notice { "Block lifted. That address can use the dashboard again." } }
            p.lede-small {
                @match active { 0 => "No addresses are blocked right now.", 1 => "1 address is blocked right now.", n => { (n) " addresses are blocked right now." } }
            }
            p.muted {
                "Anyone who guesses invite codes is blocked after a few wrong tries: for 30 minutes, then 3 hours, "
                "7 days and 30 days. After that the block stays until it's lifted here."
            }
            @if !blocks.is_empty() {
                div.table-wrap { table {
                    thead { tr { th { "Address" } th { "Status" } th.num { "Times blocked" } th { "Last wrong code" } th {} } }
                    tbody { @for b in &blocks { tr {
                        td { code { (b.ip) } }
                        td {
                            @if b.permanent { span.pill.bad { "Blocked until lifted" } }
                            @else if let Some(t) = b.blocked_until.filter(|t| *t > now) { span.pill.warn { "Blocked for " (fmt_duration(t - now)) } }
                            @else { span.pill { "Not blocked" } }
                        }
                        td.num { (b.strikes) }
                        td { (fmt_utc(b.last_failure)) }
                        td { form method="post" action={ "/security/unblock/" (b.ip) } {
                            button type="submit" { @if b.is_blocked(now) { "Lift block" } @else { "Forget" } }
                        } }
                    } } }
                } }
            }
        },
    )
}

async fn unblock(State(state): State<AppState>, CurrentUser(user): CurrentUser, Path(ip): Path<String>) -> Response {
    match state.ctx.store.unblock(&ip) {
        Ok(_) => {
            tracing::info!("{} lifted the block on {ip}", user.username);
            Redirect::to("/security?msg=unblocked").into_response()
        }
        Err(e) => {
            tracing::error!("could not lift block on {ip}: {e}");
            Redirect::to("/security").into_response()
        }
    }
}
