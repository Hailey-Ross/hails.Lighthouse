//! Dashboard pages. **Register new modules in [`all`]**; the order here is the menu order.

mod controls;
mod logs;
mod moderation;
mod overview;
mod reports;
mod security;
mod setup;
mod uptime;
mod users;

use lighthouse_core::{ServerConfig, ServerState, ServerStatus};
use maud::{Markup, html};

use crate::module::Module;

/// Every dashboard module, in menu order.
pub fn all() -> Vec<Box<dyn Module>> {
    vec![
        Box::new(overview::Overview),
        Box::new(uptime::Uptime),
        Box::new(reports::Reports),
        Box::new(controls::Controls),
        Box::new(moderation::Moderation),
        Box::new(logs::Logs),
        Box::new(users::Users),
        Box::new(security::Security),
        Box::new(setup::Setup),
    ]
}

// ---------- helpers shared by modules ----------

/// Name to show for a server: configured name, then the name it reports, then its id.
pub(crate) fn server_name(server: &ServerConfig, status: Option<&ServerStatus>) -> String {
    server
        .name
        .clone()
        .or_else(|| status.and_then(|s| s.name.clone()))
        .unwrap_or_else(|| server.id.clone())
}

pub(crate) fn state_class(state: ServerState) -> &'static str {
    match state {
        ServerState::Online => "online",
        ServerState::Starting => "starting",
        ServerState::Offline => "offline",
        ServerState::Unknown => "unknown",
    }
}

pub(crate) fn state_label(state: ServerState) -> &'static str {
    match state {
        ServerState::Online => "Online",
        ServerState::Starting => "Starting",
        ServerState::Offline => "Offline",
        ServerState::Unknown => "Checking",
    }
}

/// A small glowing lamp followed by the state, e.g. ● Online.
pub(crate) fn state_badge(state: ServerState) -> Markup {
    html! { span.status { span class={ "lamp " (state_class(state)) } aria-hidden="true" {} (state_label(state)) } }
}

/// Just the lamp, with the state as its accessible name.
pub(crate) fn lamp(state: ServerState) -> Markup {
    html! { span class={ "lamp " (state_class(state)) } role="img" aria-label=(state_label(state)) title=(state_label(state)) {} }
}
