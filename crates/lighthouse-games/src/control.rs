//! Carrying out server controls (stop, restart, start, messages, kicks, game bans).
//!
//! Each action goes the first way that works for the server:
//!
//! 1. **This machine**: a command from the server's `control` config (start/stop/restart only).
//! 2. **The server's mod**: queued and handed over with the reply to its next heartbeat, if the
//!    mod is connected and says it can do it.
//! 3. **The game's console**: the 7 Days to Die web API (`/api/command`) or telnet, or Project
//!    Zomboid RCON.
//!
//! Through the mod or console, "restart" is a clean shutdown: the host (panel auto-restart,
//! txAdmin, a service manager) has to start the server again.

use std::time::Duration;

use lighthouse_core::{ControlAction, GameKind, NativeApi, Relay, ServerConfig};

use crate::rcon::Rcon;
use crate::telnet::{self, quote};

/// A mod counts as connected while its last heartbeat is at most this old.
pub const MOD_MAX_AGE: i64 = 90;
/// Commands on this machine are stopped after this long.
const LOCAL_TIMEOUT: Duration = Duration::from_secs(120);

/// Every action key, in the order the dashboard shows them.
pub const ACTIONS: [&str; 6] = ["start", "restart", "stop", "say", "kick", "ban"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Via {
    Local,
    Mod,
    WebApi,
    Telnet,
    Rcon,
}

impl Via {
    pub fn label(self) -> &'static str {
        match self {
            Via::Local => "command on this machine",
            Via::Mod => "the server's mod",
            Via::WebApi => "the web API",
            Via::Telnet => "telnet",
            Via::Rcon => "RCON",
        }
    }
}

fn console(server: &ServerConfig) -> Option<Via> {
    if server.game.is(GameKind::SEVEN_DAYS_TO_DIE) {
        if crate::native::has_sdtd_api(server) {
            return Some(Via::WebApi);
        }
        if server.telnet.is_some() {
            return Some(Via::Telnet);
        }
    }
    if server.game.is(GameKind::PROJECT_ZOMBOID) && matches!(server.native, Some(NativeApi::Rcon { .. })) {
        return Some(Via::Rcon);
    }
    None
}

/// How `key` would be carried out on this server right now, if at all.
pub fn route(server: &ServerConfig, key: &str, relay: &Relay, now: i64) -> Option<Via> {
    let local = server.control.as_ref().is_some_and(|c| match key {
        "start" => c.command_for(&ControlAction::Start).is_some(),
        "stop" => c.command_for(&ControlAction::Stop).is_some(),
        "restart" => c.command_for(&ControlAction::Restart).is_some(),
        _ => false,
    });
    if local {
        return Some(Via::Local);
    }
    if relay.mod_accepts(&server.id, now, MOD_MAX_AGE).iter().any(|a| a == key) {
        return Some(Via::Mod);
    }
    // The console can do everything except start a stopped server.
    if key != "start" {
        return console(server);
    }
    None
}

/// The console command for an action (7 Days to Die web API or telnet, Project Zomboid RCON).
fn console_command(via: Via, action: &ControlAction) -> Option<String> {
    // The 7 Days to Die web API runs the same console commands as telnet.
    let via = if via == Via::WebApi { Via::Telnet } else { via };
    Some(match (via, action) {
        (Via::Telnet, ControlAction::Stop | ControlAction::Restart) => "shutdown".to_string(),
        (Via::Telnet, ControlAction::Say { message }) => format!("say {}", quote(message)),
        (Via::Telnet, ControlAction::Kick { player, reason }) => format!("kick {} {}", quote(player), quote(reason)),
        (
            Via::Telnet,
            ControlAction::Ban {
                player,
                reason,
                duration_secs,
            },
        ) => {
            // 7 Days to Die bans need a duration; "permanent" is a very long one.
            let (n, unit) = match duration_secs {
                Some(s) => (((*s).max(60) + 59) / 60, "minutes"),
                None => (100, "years"),
            };
            format!("ban add {} {n} {unit} {}", quote(player), quote(reason))
        }
        (Via::Rcon, ControlAction::Stop | ControlAction::Restart) => "quit".to_string(),
        (Via::Rcon, ControlAction::Say { message }) => format!("servermsg {}", quote(message)),
        (Via::Rcon, ControlAction::Kick { player, reason }) => {
            format!("kickuser {} -r {}", quote(player), quote(reason))
        }
        // Project Zomboid bans are permanent; `duration_secs` can't be honoured there.
        (Via::Rcon, ControlAction::Ban { player, reason, .. }) => {
            format!("banuser {} -r {}", quote(player), quote(reason))
        }
        _ => return None,
    })
}

/// Carry out `action`. Returns a short description of what happened.
pub async fn perform(server: &ServerConfig, action: ControlAction, relay: &Relay, now: i64) -> Result<String, String> {
    let via = route(server, action.key(), relay, now)
        .ok_or_else(|| format!("{} is not set up for this server", action.key()))?;
    match via {
        Via::Local => {
            let command = server
                .control
                .as_ref()
                .and_then(|c| c.command_for(&action))
                .ok_or("no command configured")?
                .to_string();
            run_local(&command).await
        }
        Via::Mod => {
            relay.queue(&server.id, action, now);
            Ok("Sent to the server's mod; it acts on it within about 30 seconds.".to_string())
        }
        Via::WebApi | Via::Telnet | Via::Rcon => {
            let command = console_command(via, &action).ok_or("the game console can't do that")?;
            let out = if via == Via::WebApi {
                crate::native::sdtd_command(server, &command).await?
            } else if via == Via::Telnet {
                let t = server.telnet.as_ref().ok_or("telnet is not set up")?;
                telnet::run(&server.host, t.port, &t.password, &command)
                    .await
                    .map_err(|e| e.to_string())?
            } else {
                let Some(NativeApi::Rcon { port, password }) = &server.native else {
                    return Err("RCON is not set up".into());
                };
                let mut rcon = Rcon::connect(&format!("{}:{port}", server.host), password, Duration::from_secs(5))
                    .await
                    .map_err(|e| e.to_string())?;
                rcon.use_terminator = false;
                rcon.quiet = Duration::from_millis(500);
                // `quit` closes the connection before answering; that is success.
                match rcon.exec(&command).await {
                    Ok(out) => out,
                    Err(_) if command == "quit" => String::new(),
                    Err(e) => return Err(e.to_string()),
                }
            };
            let out = out.trim();
            Ok(if out.is_empty() {
                format!("Sent `{command}` over {}.", via.label())
            } else {
                format!("Sent `{command}` over {}: {}", via.label(), last_lines(out, 3))
            })
        }
    }
}

fn last_lines(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.lines().collect();
    lines[lines.len().saturating_sub(n)..].join(" / ")
}

/// Run a configured command through the system shell and report how it went.
async fn run_local(command: &str) -> Result<String, String> {
    let mut cmd = if cfg!(windows) {
        let mut c = tokio::process::Command::new("cmd");
        c.arg("/C").arg(command);
        c
    } else {
        let mut c = tokio::process::Command::new("sh");
        c.arg("-c").arg(command);
        c
    };
    cmd.stdin(std::process::Stdio::null()).kill_on_drop(true);
    let out = tokio::time::timeout(LOCAL_TIMEOUT, cmd.output())
        .await
        .map_err(|_| format!("`{command}` did not finish within {} seconds", LOCAL_TIMEOUT.as_secs()))?
        .map_err(|e| format!("could not run `{command}`: {e}"))?;
    let text = String::from_utf8_lossy(if out.stderr.is_empty() {
        &out.stdout
    } else {
        &out.stderr
    })
    .trim()
    .to_string();
    if out.status.success() {
        Ok(if text.is_empty() {
            format!("Ran `{command}`.")
        } else {
            format!("Ran `{command}`: {}", last_lines(&text, 3))
        })
    } else {
        Err(format!("`{command}` failed ({}): {}", out.status, last_lines(&text, 3)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lighthouse_core::{Heartbeat, LocalControl, ServerState, ServerStatus, Telnet};

    fn server(game: &str) -> ServerConfig {
        ServerConfig::new("s", GameKind::get(game), "127.0.0.1", 1)
    }

    #[test]
    fn routes_in_order() {
        let relay = Relay::default();
        let mut s = server("7dtd");
        assert_eq!(route(&s, "stop", &relay, 0), None);
        s.telnet = Some(Telnet {
            port: 8081,
            password: "p".into(),
        });
        assert_eq!(route(&s, "stop", &relay, 0), Some(Via::Telnet));
        assert_eq!(route(&s, "start", &relay, 0), None);
        relay.heartbeat(
            Heartbeat {
                v: 1,
                server_id: "s".into(),
                game: s.game,
                sent_at_unix: 0,
                status: ServerStatus::new(ServerState::Online, 0),
                accepts: vec!["stop".into()],
                log: vec![],
                events: vec![],
                bans_rev: None,
            },
            0,
        );
        assert_eq!(route(&s, "stop", &relay, 10), Some(Via::Mod));
        assert_eq!(
            route(&s, "stop", &relay, 10 + MOD_MAX_AGE),
            Some(Via::Telnet),
            "mod gone quiet"
        );
        s.native = Some(NativeApi::SevenDaysWebApi {
            port: 8080,
            token_name: "t".into(),
            token_secret: "s".into(),
        });
        assert_eq!(
            route(&s, "stop", &relay, 10 + MOD_MAX_AGE),
            Some(Via::WebApi),
            "web API before telnet"
        );
        s.control = Some(LocalControl {
            stop: Some("systemctl stop x".into()),
            ..Default::default()
        });
        assert_eq!(route(&s, "stop", &relay, 10), Some(Via::Local));

        let mut pz = server("pz");
        pz.native = Some(NativeApi::Rcon {
            port: 1,
            password: "p".into(),
        });
        assert_eq!(route(&pz, "kick", &relay, 0), Some(Via::Rcon));
    }

    #[test]
    fn console_commands() {
        let ban = ControlAction::Ban {
            player: "Bob".into(),
            reason: "grief".into(),
            duration_secs: Some(3600),
        };
        assert_eq!(
            console_command(Via::Telnet, &ban).unwrap(),
            r#"ban add "Bob" 60 minutes "grief""#
        );
        let perm = ControlAction::Ban {
            player: "Bob".into(),
            reason: "grief".into(),
            duration_secs: None,
        };
        assert_eq!(
            console_command(Via::Telnet, &perm).unwrap(),
            r#"ban add "Bob" 100 years "grief""#
        );
        assert_eq!(
            console_command(Via::Rcon, &perm).unwrap(),
            r#"banuser "Bob" -r "grief""#
        );
        assert_eq!(console_command(Via::Rcon, &ControlAction::Restart).unwrap(), "quit");
        let say = ControlAction::Say {
            message: "Restart in 5".into(),
        };
        assert_eq!(console_command(Via::Telnet, &say).unwrap(), r#"say "Restart in 5""#);
        assert_eq!(console_command(Via::Rcon, &ControlAction::Start), None);
    }

    #[tokio::test]
    async fn local_commands_report_success_and_failure() {
        assert!(run_local("echo hello").await.unwrap().contains("hello"));
        assert!(run_local("exit 3").await.is_err());
    }
}
