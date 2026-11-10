//! Minimal Source RCON client.
//!
//! Protocol: <https://developer.valvesoftware.com/wiki/Source_RCON_Protocol>. Replies can span several
//! packets. Two ways to find the end are supported:
//!
//! - **terminator** (default): after the command, send an empty `SERVERDATA_RESPONSE_VALUE`. Servers
//!   mirror it back after the full reply, so its id marks the end.
//! - **quiet timeout**: for servers that ignore the terminator, stop reading once the socket has been
//!   silent for `quiet` after the first reply packet.
//!
//! A timed-out read can leave a partial packet on the socket, so open a fresh connection per poll
//! rather than reusing one after an error.

use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

const SERVERDATA_AUTH: i32 = 3;
const SERVERDATA_AUTH_RESPONSE: i32 = 2;
const SERVERDATA_EXECCOMMAND: i32 = 2;
const SERVERDATA_RESPONSE_VALUE: i32 = 0;

/// Packet size field excludes itself. Minimum: id + type + two NUL bytes.
const MIN_PACKET: i32 = 10;
/// The spec caps replies at 4096 bytes of body; some servers send more, so allow headroom.
const MAX_PACKET: i32 = 64 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum RconError {
    #[error("connection error: {0}")]
    Io(#[from] std::io::Error),
    #[error("wrong RCON password")]
    AuthFailed,
    #[error("RCON server did not answer in time")]
    Timeout,
    #[error("malformed RCON packet")]
    Malformed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Packet {
    id: i32,
    kind: i32,
    body: String,
}

fn encode(id: i32, kind: i32, body: &str) -> Vec<u8> {
    let size = (body.len() + 10) as i32;
    let mut out = Vec::with_capacity(body.len() + 14);
    out.extend_from_slice(&size.to_le_bytes());
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(body.as_bytes());
    out.extend_from_slice(&[0, 0]);
    out
}

async fn read_packet<S: AsyncRead + Unpin>(stream: &mut S) -> Result<Packet, RconError> {
    let size = stream.read_i32_le().await?;
    if !(MIN_PACKET..=MAX_PACKET).contains(&size) {
        return Err(RconError::Malformed);
    }
    let mut buf = vec![0u8; size as usize];
    stream.read_exact(&mut buf).await?;
    let id = i32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
    let kind = i32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
    let body_bytes = &buf[8..buf.len() - 2];
    // Strip any trailing NULs some servers add inside the body.
    let end = body_bytes.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
    let body = String::from_utf8_lossy(&body_bytes[..end]).into_owned();
    Ok(Packet { id, kind, body })
}

pub struct Rcon<S> {
    stream: S,
    next_id: i32,
    /// How long to wait for the first reply packet.
    pub reply_timeout: Duration,
    /// Send a terminator packet to detect the end of multi-packet replies.
    pub use_terminator: bool,
    /// Without a terminator: silence after which the reply counts as complete.
    pub quiet: Duration,
}

impl Rcon<TcpStream> {
    /// Connect and log in.
    pub async fn connect(addr: &str, password: &str, wait: Duration) -> Result<Self, RconError> {
        let stream = timeout(wait, TcpStream::connect(addr))
            .await
            .map_err(|_| RconError::Timeout)??;
        let mut rcon = Rcon::new(stream, wait);
        rcon.auth(password).await?;
        Ok(rcon)
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin> Rcon<S> {
    pub fn new(stream: S, reply_timeout: Duration) -> Self {
        Self {
            stream,
            next_id: 1,
            reply_timeout,
            use_terminator: true,
            quiet: Duration::from_millis(300),
        }
    }

    async fn send(&mut self, kind: i32, body: &str) -> Result<i32, RconError> {
        let id = self.next_id;
        self.next_id = self.next_id.checked_add(1).unwrap_or(1);
        self.stream.write_all(&encode(id, kind, body)).await?;
        self.stream.flush().await?;
        Ok(id)
    }

    async fn read(&mut self, wait: Duration) -> Result<Packet, RconError> {
        timeout(wait, read_packet(&mut self.stream))
            .await
            .map_err(|_| RconError::Timeout)?
    }

    pub async fn auth(&mut self, password: &str) -> Result<(), RconError> {
        let id = self.send(SERVERDATA_AUTH, password).await?;
        loop {
            let p = self.read(self.reply_timeout).await?;
            // Source servers send an empty RESPONSE_VALUE before the AUTH_RESPONSE; skip it.
            if p.kind == SERVERDATA_AUTH_RESPONSE {
                return match p.id {
                    -1 => Err(RconError::AuthFailed),
                    got if got == id => Ok(()),
                    _ => Err(RconError::Malformed),
                };
            }
        }
    }

    /// Run a command and return the full reply text.
    pub async fn exec(&mut self, command: &str) -> Result<String, RconError> {
        let id = self.send(SERVERDATA_EXECCOMMAND, command).await?;
        let end_id = if self.use_terminator {
            Some(self.send(SERVERDATA_RESPONSE_VALUE, "").await?)
        } else {
            None
        };

        let mut out = String::new();
        let mut got_reply = false;
        loop {
            let wait = if !got_reply || end_id.is_some() {
                self.reply_timeout
            } else {
                self.quiet
            };
            let p = match self.read(wait).await {
                Ok(p) => p,
                // Quiet mode, or a server that ignored the terminator: what we have is the reply.
                Err(RconError::Timeout) if got_reply => break,
                Err(e) => return Err(e),
            };
            if Some(p.id) == end_id {
                break;
            }
            if p.id == id {
                got_reply = true;
                out.push_str(&p.body);
            }
            // Anything else is a leftover from an earlier command; ignore it.
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{DuplexStream, duplex};

    async fn reply(server: &mut DuplexStream, id: i32, kind: i32, body: &str) {
        server.write_all(&encode(id, kind, body)).await.unwrap();
    }

    #[test]
    fn encode_layout() {
        let p = encode(7, SERVERDATA_EXECCOMMAND, "hi");
        assert_eq!(&p[0..4], &12i32.to_le_bytes());
        assert_eq!(&p[4..8], &7i32.to_le_bytes());
        assert_eq!(&p[12..14], b"hi");
        assert_eq!(&p[14..], &[0, 0]);
    }

    #[tokio::test]
    async fn auth_then_multi_packet_reply_with_terminator() {
        let (client, mut server) = duplex(4096);
        let srv = tokio::spawn(async move {
            let auth = read_packet(&mut server).await.unwrap();
            assert_eq!((auth.kind, auth.body.as_str()), (SERVERDATA_AUTH, "secret"));
            reply(&mut server, auth.id, SERVERDATA_RESPONSE_VALUE, "").await;
            reply(&mut server, auth.id, SERVERDATA_AUTH_RESPONSE, "").await;

            let cmd = read_packet(&mut server).await.unwrap();
            let term = read_packet(&mut server).await.unwrap();
            assert_eq!(cmd.body, "players");
            reply(
                &mut server,
                cmd.id,
                SERVERDATA_RESPONSE_VALUE,
                "Players connected (2):\n",
            )
            .await;
            reply(&mut server, cmd.id, SERVERDATA_RESPONSE_VALUE, "-Alice\n-Bob\n").await;
            reply(&mut server, term.id, SERVERDATA_RESPONSE_VALUE, "").await;
            server
        });

        let mut rcon = Rcon::new(client, Duration::from_secs(2));
        rcon.auth("secret").await.unwrap();
        let text = rcon.exec("players").await.unwrap();
        assert_eq!(text, "Players connected (2):\n-Alice\n-Bob\n");
        drop(srv.await.unwrap());
    }

    #[tokio::test]
    async fn wrong_password() {
        let (client, mut server) = duplex(1024);
        tokio::spawn(async move {
            let _auth = read_packet(&mut server).await.unwrap();
            reply(&mut server, -1, SERVERDATA_AUTH_RESPONSE, "").await;
            server
        });
        let mut rcon = Rcon::new(client, Duration::from_secs(2));
        assert!(matches!(rcon.auth("nope").await, Err(RconError::AuthFailed)));
    }

    #[tokio::test]
    async fn quiet_timeout_without_terminator() {
        let (client, mut server) = duplex(1024);
        let srv = tokio::spawn(async move {
            let cmd = read_packet(&mut server).await.unwrap();
            reply(&mut server, cmd.id, SERVERDATA_RESPONSE_VALUE, "part one, ").await;
            reply(&mut server, cmd.id, SERVERDATA_RESPONSE_VALUE, "part two").await;
            server // keep the stream open and silent
        });
        let mut rcon = Rcon::new(client, Duration::from_secs(2));
        rcon.use_terminator = false;
        rcon.quiet = Duration::from_millis(100);
        assert_eq!(rcon.exec("x").await.unwrap(), "part one, part two");
        drop(srv.await.unwrap());
    }
}
