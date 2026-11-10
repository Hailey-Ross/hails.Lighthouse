//! 7 Days to Die's telnet console (`TelnetEnabled`, `TelnetPort`, `TelnetPassword` in
//! serverconfig.xml): log in, run one command, collect its output, leave.
//!
//! The console is line based. After connecting the server asks `Please enter password:`, answers
//! `Logon successful.`, prints a banner, and from then on echoes every log line too, so a command's
//! output is "whatever arrives until things go quiet".

use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{Instant, timeout};

const WAIT: Duration = Duration::from_secs(5);
/// Output is complete once the console has been silent this long.
const QUIET: Duration = Duration::from_millis(600);
/// Stop reading after this much output.
const MAX_OUTPUT: usize = 256 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum TelnetError {
    #[error("connection error: {0}")]
    Io(#[from] std::io::Error),
    #[error("the telnet console did not answer in time")]
    Timeout,
    #[error("wrong telnet password")]
    WrongPassword,
}

/// Read until `pred` is true on what has arrived so far, or `wait` passes.
async fn read_until(
    stream: &mut TcpStream,
    buf: &mut Vec<u8>,
    wait: Duration,
    pred: impl Fn(&str) -> bool,
) -> Result<(), TelnetError> {
    let deadline = Instant::now() + wait;
    let mut chunk = [0u8; 4096];
    while !pred(&String::from_utf8_lossy(buf)) {
        let left = deadline.saturating_duration_since(Instant::now());
        let n = timeout(left, stream.read(&mut chunk))
            .await
            .map_err(|_| TelnetError::Timeout)??;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
    Ok(())
}

/// Run `command` on the console and return what it printed.
pub async fn run(host: &str, port: u16, password: &str, command: &str) -> Result<String, TelnetError> {
    let mut stream = timeout(WAIT, TcpStream::connect((host, port)))
        .await
        .map_err(|_| TelnetError::Timeout)??;
    let mut buf = Vec::new();
    read_until(&mut stream, &mut buf, WAIT, |s| {
        s.to_ascii_lowercase().contains("password")
    })
    .await?;
    stream.write_all(format!("{password}\r\n").as_bytes()).await?;
    buf.clear();
    read_until(&mut stream, &mut buf, WAIT, |s| {
        let s = s.to_ascii_lowercase();
        s.contains("logon successful") || s.contains("password incorrect") || s.contains("wrong password")
    })
    .await?;
    if !String::from_utf8_lossy(&buf)
        .to_ascii_lowercase()
        .contains("logon successful")
    {
        return Err(TelnetError::WrongPassword);
    }
    // Let the welcome banner pass before sending the command.
    let _ = drain(&mut stream, QUIET).await;
    stream
        .write_all(format!("{}\r\n", command.replace(['\r', '\n'], " ")).as_bytes())
        .await?;
    let out = drain(&mut stream, QUIET).await.unwrap_or_default();
    let _ = stream.write_all(b"exit\r\n").await;
    Ok(clean(&out, command))
}

/// Read until the console is quiet for `quiet`.
async fn drain(stream: &mut TcpStream, quiet: Duration) -> Result<String, TelnetError> {
    let mut out = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        match timeout(quiet, stream.read(&mut chunk)).await {
            Err(_) => break,
            Ok(Ok(0)) => break,
            Ok(Ok(n)) => {
                out.extend_from_slice(&chunk[..n]);
                if out.len() > MAX_OUTPUT {
                    break;
                }
            }
            Ok(Err(e)) => return Err(e.into()),
        }
    }
    Ok(String::from_utf8_lossy(&out).into_owned())
}

/// Drop the echoed command and blank lines.
fn clean(out: &str, command: &str) -> String {
    out.lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty() && l.trim() != command.trim())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Quote an argument for the 7 Days to Die console: `"…"` with inner quotes removed.
pub fn quote(arg: &str) -> String {
    format!("\"{}\"", arg.replace(['"', '\r', '\n'], " ").trim())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpListener;

    #[test]
    fn quoting_and_cleaning() {
        assert_eq!(quote(r#"Bob "the" Builder"#), r#""Bob  the  Builder""#);
        assert_eq!(
            clean("say \"hi\"\r\n\r\nINF Chat: hi\r\n", "say \"hi\""),
            "INF Chat: hi"
        );
    }

    async fn fake_console(password: &'static str) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut s, _) = listener.accept().await.unwrap();
            s.write_all(b"Please enter password:\r\n").await.unwrap();
            let mut buf = [0u8; 256];
            let n = s.read(&mut buf).await.unwrap();
            if String::from_utf8_lossy(&buf[..n]).trim() != password {
                s.write_all(b"Password incorrect, please enter password:\r\n")
                    .await
                    .unwrap();
                return;
            }
            s.write_all(b"Logon successful.\r\n\r\n*** Connected with 7DTD server.\r\n")
                .await
                .unwrap();
            let n = s.read(&mut buf).await.unwrap();
            let cmd = String::from_utf8_lossy(&buf[..n]).trim().to_string();
            s.write_all(format!("{cmd}\r\nINF Executing command '{cmd}'\r\n").as_bytes())
                .await
                .unwrap();
            let _ = s.read(&mut buf).await;
        });
        port
    }

    #[tokio::test]
    async fn runs_a_command() {
        let port = fake_console("pw").await;
        let out = run("127.0.0.1", port, "pw", "say \"hello\"").await.unwrap();
        assert_eq!(out, "INF Executing command 'say \"hello\"'");
        let port = fake_console("pw").await;
        assert!(matches!(
            run("127.0.0.1", port, "nope", "x").await,
            Err(TelnetError::WrongPassword)
        ));
    }
}
