//! The extension point of the dashboard.
//!
//! Every page is a [`Module`]: it names itself, says which permission is needed to open it, lists
//! any extra permissions it understands, and brings its own routes. The router mounts each module
//! under `/<id>`, checks the view permission before any of the module's handlers run, and builds
//! the navigation from the modules the signed-in user may open.
//!
//! To add a page: create `modules/<name>.rs` implementing `Module`, then add it to
//! `modules::all()`. Nothing else needs to change.

use axum::Router;

use crate::AppState;

/// A permission a module understands. Shown as a checkbox on the Users page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PermissionDef {
    /// Stable key stored with each user, e.g. `logs.view`. Never rename a released key.
    pub key: &'static str,
    /// Short human description.
    pub label: &'static str,
}

pub trait Module: Send + Sync + 'static {
    /// URL prefix and navigation key, e.g. `logs` serves `/logs/...`.
    fn id(&self) -> &'static str;

    /// Navigation label.
    fn title(&self) -> &'static str;

    /// Navigation icon: SVG path data on a 24 × 24 grid, drawn as a 1.8 px stroke.
    fn icon(&self) -> &'static str {
        "M12 4a8 8 0 1 0 0 16 8 8 0 0 0 0-16z"
    }

    /// Permission needed to open any page of this module.
    fn view_permission(&self) -> &'static str;

    /// Every permission this module defines, including the view permission.
    fn permissions(&self) -> Vec<PermissionDef>;

    /// The module's routes, relative to `/<id>`. `/` is the module's main page.
    fn routes(&self) -> Router<AppState>;
}
