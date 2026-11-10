//! End-to-end checks of the dashboard router: sign-in, page access by permission, server scoping
//! and the cross-site form check.

use std::collections::BTreeSet;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Body;
use axum::extract::ConnectInfo;
use axum::http::{Request, StatusCode, header};
use lighthouse_core::Config;
use lighthouse_dashboard::{Context, router};
use lighthouse_discord::monitor::Monitor;
use lighthouse_store::Store;
use tower::ServiceExt;

const CONFIG: &str = r#"
[[servers]]
id = "athens"
game = "7dtd"
name = "Athens PvE"
host = "127.0.0.1"
game_port = 26930

[[servers]]
id = "pz"
game = "pz"
name = "Safehouse"
host = "127.0.0.1"
game_port = 16261
"#;

fn app() -> axum::Router {
    let config = Config::parse(CONFIG).unwrap();
    let store = Store::open_in_memory().unwrap();
    store.set_superadmin("admin", "password1", 0).unwrap();
    let view: BTreeSet<String> = ["servers.view".to_string()].into();
    let athens: BTreeSet<String> = ["athens".to_string()].into();
    store
        .create_user("viewer", "password2", false, &view, Some(&athens), 0)
        .unwrap();
    let monitor = Monitor::new(&config.servers, None);
    router(Arc::new(Context {
        config,
        monitor,
        store: Arc::new(store),
    }))
}

fn req(method: &str, uri: &str, cookie: Option<&str>, body: Option<&str>) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::HOST, "localhost:8650")
        .header(header::ORIGIN, "http://localhost:8650");
    if let Some(c) = cookie {
        b = b.header(header::COOKIE, c);
    }
    if body.is_some() {
        b = b.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    }
    let mut r = b.body(Body::from(body.unwrap_or_default().to_string())).unwrap();
    r.extensions_mut()
        .insert(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 5555))));
    r
}

async fn body_text(resp: axum::response::Response) -> String {
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    String::from_utf8_lossy(&bytes).into_owned()
}

async fn login(app: &axum::Router, user: &str, pass: &str) -> String {
    let resp = app
        .clone()
        .oneshot(req(
            "POST",
            "/login",
            None,
            Some(&format!("username={user}&password={pass}")),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER, "login should redirect");
    let cookie = resp
        .headers()
        .get(header::SET_COOKIE)
        .expect("session cookie")
        .to_str()
        .unwrap();
    assert!(cookie.contains("HttpOnly") && cookie.contains("SameSite=Strict"));
    cookie.split(';').next().unwrap().to_string()
}

#[tokio::test]
async fn signed_out_users_are_sent_to_login() {
    let resp = app().oneshot(req("GET", "/overview", None, None)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(resp.headers()[header::LOCATION], "/login");
}

#[tokio::test]
async fn wrong_password_is_rejected() {
    let resp = app()
        .oneshot(req("POST", "/login", None, Some("username=admin&password=nope")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(body_text(resp).await.contains("Wrong username or password"));
}

#[tokio::test]
async fn superadmin_sees_every_page_and_server() {
    let app = app();
    let cookie = login(&app, "admin", "password1").await;
    for page in ["/overview", "/uptime", "/uptime/pz", "/logs", "/users"] {
        let resp = app
            .clone()
            .oneshot(req("GET", page, Some(&cookie), None))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "{page}");
    }
    let html = body_text(
        app.clone()
            .oneshot(req("GET", "/overview", Some(&cookie), None))
            .await
            .unwrap(),
    )
    .await;
    assert!(html.contains("Athens PvE") && html.contains("Safehouse"));
    assert!(html.contains(">Users<"), "superadmin gets the Users menu");
}

#[tokio::test]
async fn sub_admin_only_gets_what_was_granted() {
    let app = app();
    let cookie = login(&app, "viewer", "password2").await;
    let html = body_text(
        app.clone()
            .oneshot(req("GET", "/overview", Some(&cookie), None))
            .await
            .unwrap(),
    )
    .await;
    assert!(html.contains("Athens PvE"));
    assert!(!html.contains("Safehouse"), "server outside the user's scope is hidden");
    assert!(
        !html.contains(">Users<") && !html.contains(">Logs<"),
        "menu only shows granted pages"
    );
    for page in ["/uptime", "/logs", "/users"] {
        let resp = app
            .clone()
            .oneshot(req("GET", page, Some(&cookie), None))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN, "{page}");
    }
}

#[tokio::test]
async fn superadmin_creates_a_scoped_user() {
    let app = app();
    let cookie = login(&app, "admin", "password1").await;
    let form = "username=mod&password=password3&perm=logs.view&perm=servers.view&scope=some&server=pz";
    let resp = app
        .clone()
        .oneshot(req("POST", "/users", Some(&cookie), Some(form)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(resp.headers()[header::LOCATION], "/users?msg=created");

    let mod_cookie = login(&app, "mod", "password3").await;
    let resp = app
        .clone()
        .oneshot(req("GET", "/logs", Some(&mod_cookie), None))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let html = body_text(
        app.clone()
            .oneshot(req("GET", "/overview", Some(&mod_cookie), None))
            .await
            .unwrap(),
    )
    .await;
    assert!(html.contains("Safehouse") && !html.contains("Athens PvE"));
}

#[tokio::test]
async fn cross_site_posts_are_blocked() {
    let app = app();
    let cookie = login(&app, "admin", "password1").await;
    let mut r = req(
        "POST",
        "/users",
        Some(&cookie),
        Some("username=evil&password=password9"),
    );
    r.headers_mut()
        .insert(header::ORIGIN, "https://evil.example".parse().unwrap());
    let resp = app.clone().oneshot(r).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn registration_needs_an_invite() {
    let app = app();
    let html = body_text(app.clone().oneshot(req("GET", "/register", None, None)).await.unwrap()).await;
    assert!(html.contains("by invitation only") && !html.contains("name=\"password\""));

    // A guessed code (right length, random) is refused, and no form is shown.
    let guess = "0".repeat(64);
    let html = body_text(
        app.clone()
            .oneshot(req("GET", &format!("/register?code={guess}"), None, None))
            .await
            .unwrap(),
    )
    .await;
    assert!(html.contains("expired or was already used") && !html.contains("name=\"password\""));
    let resp = app
        .clone()
        // A different address, so the random block limit (2-5 wrong codes) can't kick in here;
        // blocking has its own test below.
        .oneshot(from(
            req(
                "POST",
                "/register",
                None,
                Some(&format!(
                    "code={guess}&username=sneaky&password=password9&confirm=password9"
                )),
            ),
            [10, 9, 9, 9],
        ))
        .await
        .unwrap();
    assert!(body_text(resp).await.contains("expired or was already used"));
}

#[tokio::test]
async fn invite_flow_creates_a_scoped_account_once() {
    let app = app();
    let admin = login(&app, "admin", "password1").await;
    let resp = app
        .clone()
        .oneshot(req(
            "POST",
            "/users/invite",
            Some(&admin),
            Some("perm=servers.view&scope=some&server=pz&valid=86400"),
        ))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    let location = resp.headers()[header::LOCATION].to_str().unwrap().to_string();
    let code = location
        .split("invite=")
        .nth(1)
        .unwrap()
        .split('#')
        .next()
        .unwrap()
        .to_string();
    assert_eq!(code.len(), 64, "256-bit invite code");

    // The Users page shows the link once, right after creation.
    let html = body_text(
        app.clone()
            .oneshot(req("GET", &location, Some(&admin), None))
            .await
            .unwrap(),
    )
    .await;
    assert!(html.contains(&format!("/register?code={code}")));

    let form = format!("code={code}&username=newbie&password=password7&confirm=password7");
    let resp = app
        .clone()
        .oneshot(req("POST", "/register", None, Some(&form)))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER, "registered and signed in");
    let cookie = resp.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let html = body_text(
        app.clone()
            .oneshot(req("GET", "/overview", Some(&cookie), None))
            .await
            .unwrap(),
    )
    .await;
    assert!(
        html.contains("Safehouse") && !html.contains("Athens PvE"),
        "invite's server scope applied"
    );

    let again = format!("code={code}&username=second&password=password7&confirm=password7");
    let resp = app
        .clone()
        .oneshot(req("POST", "/register", None, Some(&again)))
        .await
        .unwrap();
    assert!(
        body_text(resp).await.contains("expired or was already used"),
        "single use"
    );
}

fn from(mut r: Request<Body>, ip: [u8; 4]) -> Request<Body> {
    r.extensions_mut().insert(ConnectInfo(SocketAddr::from((ip, 4444))));
    r
}

#[tokio::test]
async fn guessing_codes_blocks_the_address_until_lifted() {
    let app = app();
    let attacker = [10, 0, 0, 5];
    let mut blocked_after = None;
    for n in 1..=5 {
        let guess = format!("{:064x}", n);
        let resp = app
            .clone()
            .oneshot(from(
                req("GET", &format!("/register?code={guess}"), None, None),
                attacker,
            ))
            .await
            .unwrap();
        if resp.status() == StatusCode::FORBIDDEN {
            blocked_after = Some(n);
            break;
        }
    }
    let n = blocked_after.expect("blocked within 5 wrong codes");
    assert!((2..=5).contains(&n), "random limit between 2 and 5, got {n}");

    // The whole dashboard is closed to that address, but not to others.
    let resp = app
        .clone()
        .oneshot(from(req("GET", "/login", None, None), attacker))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert!(body_text(resp).await.contains("blocked for 30m"));
    let resp = app.clone().oneshot(req("GET", "/login", None, None)).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // An admin lifts it on the Security page.
    let admin = login(&app, "admin", "password1").await;
    let html = body_text(
        app.clone()
            .oneshot(req("GET", "/security", Some(&admin), None))
            .await
            .unwrap(),
    )
    .await;
    assert!(html.contains("10.0.0.5"));
    let resp = app
        .clone()
        .oneshot(req("POST", "/security/unblock/10.0.0.5", Some(&admin), Some("")))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    let resp = app
        .clone()
        .oneshot(from(req("GET", "/login", None, None), attacker))
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn current_page_is_highlighted_in_the_menu() {
    let app = app();
    let cookie = login(&app, "admin", "password1").await;
    let html = body_text(
        app.clone()
            .oneshot(req("GET", "/reports", Some(&cookie), None))
            .await
            .unwrap(),
    )
    .await;
    assert!(
        html.contains(r#"class="nav-item active" aria-current="page""#),
        "active page marked once, in one class attribute"
    );
    assert_eq!(html.matches("nav-item active").count(), 1);
}
