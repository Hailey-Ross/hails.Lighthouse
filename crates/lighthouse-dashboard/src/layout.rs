//! Page frame shared by every module: the harbour atmosphere, the sidebar (only the pages the user
//! may open), theme picker, and the static assets (stylesheet, font, brand images).

use axum::http::header;
use axum::response::IntoResponse;
use lighthouse_store::User;
use maud::{DOCTYPE, Markup, PreEscaped, html};

use crate::AppState;

const CSS: &str = include_str!("../assets/app.css");

/// Fingerprint of the stylesheet, so browsers can cache it for good and still fetch the new one
/// after an update (FNV-1a, computed at compile time).
const CSS_VERSION: u64 = {
    let bytes = CSS.as_bytes();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut i = 0;
    while i < bytes.len() {
        h ^= bytes[i] as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
        i += 1;
    }
    h
};
const FONT: &[u8] = include_bytes!("../assets/fonts/Nunito.ttf");
const MARK: &str = include_str!("../../../assets/brand/lighthouse-mark.svg");
const HERO: &str = include_str!("../../../assets/brand/lighthouse-hero.svg");
const WAVE: &str = include_str!("../../../assets/brand/wave.svg");

/// Themes offered in the picker: (id, label, time of day). Colours live in `assets/app.css`.
const THEMES: &[(&str, &str)] = &[
    ("harbor", "Harbor at dawn"),
    ("moss", "Moss garden"),
    ("dusk", "Dusk"),
    ("tide", "Night tide"),
];

/// Applies the saved theme before the page paints, so there is no flash.
/// Also tells the server the viewer's time zone (minutes east of UTC) for local times in reports,
/// and sets each wave layer's phase from the clock so the sea doesn't restart on every page.
const THEME_INIT: &str = "try{var t=localStorage.getItem('lighthouse-theme');if(t)document.documentElement.dataset.theme=t}catch(e){}document.cookie='lighthouse_tz='+(-new Date().getTimezoneOffset())+';path=/;SameSite=Strict;max-age=31536000';\n(function(){var t=Date.now()/1000,r=document.documentElement.style;[['--w1',52],['--w2',44],['--w3',38]].forEach(function(w){r.setProperty(w[0],-(t%w[1]).toFixed(2)+'s')})})();";

/// Wires up the theme swatches.
const THEME_PICKER: &str = r#"
(function(){
  var root=document.documentElement, buttons=document.querySelectorAll('[data-set-theme]');
  function mark(){var cur=root.dataset.theme;buttons.forEach(function(b){b.setAttribute('aria-pressed',String(b.dataset.setTheme===cur))})}
  buttons.forEach(function(b){b.addEventListener('click',function(){
    root.dataset.theme=b.dataset.setTheme;try{localStorage.setItem('lighthouse-theme',b.dataset.setTheme)}catch(e){}mark();
  })});
  mark();
})();
"#;

fn svg(body: &'static str) -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "image/svg+xml"),
            (header::CACHE_CONTROL, "max-age=86400"),
        ],
        body,
    )
}

pub async fn mark() -> impl IntoResponse {
    svg(MARK)
}

pub async fn hero() -> impl IntoResponse {
    svg(HERO)
}

pub async fn wave() -> impl IntoResponse {
    svg(WAVE)
}

pub async fn font() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "font/ttf"),
            (header::CACHE_CONTROL, "max-age=2592000"),
        ],
        FONT,
    )
}

pub async fn stylesheet() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        CSS,
    )
}

fn head(title: &str, extra: Markup) -> Markup {
    html! {
        head {
            meta charset="utf-8";
            meta name="viewport" content="width=device-width, initial-scale=1";
            title { (title) " – Lighthouse" }
            script { (PreEscaped(THEME_INIT)) }
            link rel="preload" href="/static/nunito.ttf" as="font" type="font/ttf" crossorigin;
            link rel="stylesheet" href={ "/static/app.css?v=" (format!("{CSS_VERSION:x}")) };
            link rel="icon" type="image/svg+xml" href="/static/mark.svg";
            (extra)
        }
    }
}

/// The sky, mist and drifting sea behind every page.
fn atmosphere() -> Markup {
    html! {
        div.atmosphere aria-hidden="true" {
            div.mist {}
            div.sea { div.wave.w1 {} div.wave.w2 {} div.wave.w3 {} }
        }
    }
}

fn icon(path: &str) -> Markup {
    html! {
        svg.icon viewBox="0 0 24 24" aria-hidden="true" { path d=(path) {} }
    }
}

/// A page without navigation (sign-in, registration, blocked).
pub fn bare(title: &str, body: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            (head(title, html! {}))
            body.bare { (atmosphere()) main { (body) } }
        }
    }
}

/// A full page. `active` is the id of the current module (for highlighting the menu).
pub fn page(state: &AppState, user: &User, active: &str, title: &str, body: Markup) -> Markup {
    page_with_head(state, user, active, title, html! {}, body)
}

/// Like [`page`], with extra elements in `<head>` (e.g. auto refresh).
pub fn page_with_head(state: &AppState, user: &User, active: &str, title: &str, extra: Markup, body: Markup) -> Markup {
    page_full(state, user, active, title, extra, body, true)
}

/// The full form: `show_title = false` leaves the visible heading to the page body.
pub fn page_full(
    state: &AppState,
    user: &User,
    active: &str,
    title: &str,
    extra: Markup,
    body: Markup,
    show_title: bool,
) -> Markup {
    let initial = user
        .username
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default();
    html! {
        (DOCTYPE)
        html lang="en" {
            (head(title, extra))
            body.app {
                (atmosphere())
                aside.sidebar {
                    a.brand href="/" {
                        img.brand-mark src="/static/mark.svg" alt="" width="44" height="44";
                        span.brand-name { "Lighthouse" small { "keeping watch" } }
                    }
                    nav aria-label="Pages" {
                        @for m in state.modules.iter().filter(|m| user.can(m.view_permission())) {
                            a href={ "/" (m.id()) }
                              class={ "nav-item" @if m.id() == active { " active" } }
                              aria-current=[(m.id() == active).then_some("page")] {
                                (icon(m.icon())) span { (m.title()) }
                            }
                        }
                    }
                    div.sidebar-foot {
                        div.themes role="group" aria-label="Colour theme" {
                            @for (id, label) in THEMES {
                                button type="button" data-set-theme=(id) title=(label) aria-label=(label) {}
                            }
                        }
                        form.account method="post" action="/logout" {
                            span.avatar aria-hidden="true" { (initial) }
                            span.who {
                                (user.username)
                                small { @if user.is_superadmin { "Superadmin" } @else { "Admin" } }
                            }
                            button.quiet.signout type="submit" title="Sign out" aria-label="Sign out" {
                                (icon("M15 4h3a2 2 0 0 1 2 2v12a2 2 0 0 1-2 2h-3 M10 8l-4 4 4 4 M6 12h10"))
                            }
                        }
                    }
                }
                main.content {
                    @if show_title { h1.page-title { (title) } }
                    (body)
                }
                script { (PreEscaped(THEME_PICKER)) }
            }
        }
    }
}
