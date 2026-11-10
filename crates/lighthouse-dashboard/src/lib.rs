//! Lighthouse web dashboard.
//!
//! A small server-rendered site (axum + maud). Pages are [`Module`]s registered in
//! [`modules::all`]; the router mounts each one under `/<id>` behind its view permission.
//! Users and sessions live in the Lighthouse database; the superadmin is created by the CLI setup.

mod auth;
mod charts;
mod layout;
pub mod module;
pub mod modules;
mod register;
mod util;

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::middleware;
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::get;
use lighthouse_core::{Config, ServerConfig};
use lighthouse_discord::monitor::Monitor;
use lighthouse_store::{Store, User};

pub use module::{Module, PermissionDef};

/// Everything the dashboard needs from the rest of Lighthouse.
pub struct Context {
    pub config: Config,
    pub monitor: Arc<Monitor>,
    pub store: Arc<Store>,
    /// The config file, for the Setup page. `None` disables saving.
    pub config_path: Option<std::path::PathBuf>,
    /// Restarts Lighthouse so a saved config takes effect. `None`: the page asks for a manual restart.
    pub restart: Option<Arc<dyn Fn() + Send + Sync>>,
}

#[derive(Clone)]
pub struct AppState {
    pub ctx: Arc<Context>,
    pub modules: Arc<Vec<Box<dyn Module>>>,
    pub(crate) limiter: Arc<auth::LoginLimiter>,
}

impl AppState {
    /// Servers this user may see, in config order.
    pub fn visible_servers(&self, user: &User) -> Vec<&ServerConfig> {
        self.ctx
            .config
            .servers
            .iter()
            .filter(|s| user.sees_server(&s.id))
            .collect()
    }

    /// Every permission any registered module defines.
    pub fn all_permissions(&self) -> Vec<PermissionDef> {
        self.modules.iter().flat_map(|m| m.permissions()).collect()
    }
}

/// Build the full router. Public for tests.
pub fn router(ctx: Arc<Context>) -> Router {
    let modules = Arc::new(modules::all());
    let state = AppState {
        ctx,
        modules: Arc::clone(&modules),
        limiter: Arc::default(),
    };

    let mut app = Router::new()
        .route("/", get(home))
        .route("/login", get(auth::login_page).post(auth::login))
        .route("/logout", axum::routing::post(auth::logout))
        .route("/register", get(register::register_page).post(register::register))
        .route("/static/app.css", get(layout::stylesheet))
        .route("/static/mark.svg", get(layout::mark))
        .route("/static/hero.svg", get(layout::hero))
        .route("/static/wave.svg", get(layout::wave))
        .route("/static/nunito.ttf", get(layout::font))
        .route("/favicon.ico", get(layout::mark));

    for m in modules.iter() {
        let guarded = m.routes().route_layer(middleware::from_fn_with_state(
            (state.clone(), m.view_permission()),
            auth::require_permission,
        ));
        app = app.nest(&format!("/{}", m.id()), guarded);
        // `/<id>/` is a common typo for `/<id>`.
        let target = format!("/{}", m.id());
        app = app.route(
            &format!("/{}/", m.id()),
            get(move || {
                let target = target.clone();
                async move { Redirect::permanent(&target) }
            }),
        );
    }

    app.layer(middleware::from_fn(auth::same_origin_posts))
        .layer(middleware::from_fn_with_state(state.clone(), auth::reject_blocked))
        .with_state(state)
}

/// `/`: go to the first page this user may open.
async fn home(State(state): State<AppState>, user: auth::CurrentUser) -> Response {
    match state.modules.iter().find(|m| user.0.can(m.view_permission())) {
        Some(m) => Redirect::to(&format!("/{}", m.id())).into_response(),
        None => layout::page(
            &state,
            &user.0,
            "",
            "No access",
            maud::html! {
                p { "Your account has no pages assigned yet. Ask the superadmin to grant you access." }
            },
        )
        .into_response(),
    }
}

/// Serve the dashboard until the process exits.
pub async fn serve(ctx: Arc<Context>, bind: &str) -> std::io::Result<()> {
    let listener = tokio::net::TcpListener::bind(bind).await?;
    tracing::info!("dashboard listening on http://{}", listener.local_addr()?);
    axum::serve(
        listener,
        router(ctx).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await
}
