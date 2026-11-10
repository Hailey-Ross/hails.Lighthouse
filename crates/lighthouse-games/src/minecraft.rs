//! Minecraft: Java Edition server list ping (TCP) and Bedrock Edition unconnected ping (UDP).
//!
//! Java: <https://minecraft.wiki/w/Java_Edition_protocol/Server_List_Ping>.
//! Bedrock: RakNet "unconnected ping"; the reply carries a `;`-separated status string.

use std::time::{Duration, Instant};

use lighthouse_core::{
    Capabilities, GameAdapter, GameKind, Player, QueryError, ServerConfig, ServerState, ServerStatus,
};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpStream, UdpSocket};
use tokio::time::timeout;

const WAIT: Duration = Duration::from_secs(3);
/// Status replies are small; anything bigger is not a Minecraft server.
const MAX_STATUS_LEN: usize = 1 << 20;

fn query_port(server: &ServerConfig) -> u16 {
    server
        .query_port
        .unwrap_or(server.game.query_port_for(server.game_port))
}

fn unreachable(e: impl std::fmt::Display) -> QueryError {
    QueryError::Unreachable(e.to_string())
}

/// Remove `§x` formatting codes.
fn strip_formatting(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '§' {
            chars.next();
        } else {
            out.push(c);
        }
    }
    out.trim().to_string()
}

// ---------- Java Edition ----------

fn write_varint(out: &mut Vec<u8>, value: i32) {
    let mut v = value as u32;
    loop {
        if v & !0x7F == 0 {
            out.push(v as u8);
            return;
        }
        out.push((v as u8 & 0x7F) | 0x80);
        v >>= 7;
    }
}

async fn read_varint<R: AsyncRead + Unpin>(r: &mut R) -> Result<i32, QueryError> {
    let mut value: u32 = 0;
    for i in 0..5 {
        let b = r.read_u8().await.map_err(unreachable)?;
        value |= u32::from(b & 0x7F) << (7 * i);
        if b & 0x80 == 0 {
            return Ok(value as i32);
        }
    }
    Err(QueryError::BadResponse("VarInt too long".into()))
}

fn frame(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 5);
    write_varint(&mut out, payload.len() as i32);
    out.extend_from_slice(payload);
    out
}

fn handshake(host: &str, port: u16) -> Vec<u8> {
    let mut p = vec![0x00]; // packet id: handshake
    write_varint(&mut p, -1); // protocol version: "any" for status
    write_varint(&mut p, host.len() as i32);
    p.extend_from_slice(host.as_bytes());
    p.extend_from_slice(&port.to_be_bytes());
    write_varint(&mut p, 1); // next state: status
    frame(&p)
}

/// Text of a chat component (string, `{"text":..,"extra":[..]}` or an array).
fn chat_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts.iter().map(chat_text).collect(),
        Value::Object(o) => {
            let mut s = o.get("text").and_then(Value::as_str).unwrap_or_default().to_string();
            if let Some(extra) = o.get("extra") {
                s.push_str(&chat_text(extra));
            }
            s
        }
        _ => String::new(),
    }
}

/// Map the Java status JSON to a status.
fn java_status(json: &Value, now_unix: i64) -> ServerStatus {
    let mut s = ServerStatus::new(ServerState::Online, now_unix);
    let motd = strip_formatting(&json.get("description").map(chat_text).unwrap_or_default());
    let mut lines = motd.lines().map(str::trim).filter(|l| !l.is_empty());
    s.name = lines.next().map(str::to_string);
    // The MOTD's second line is usually a tagline.
    s.description = Some(lines.collect::<Vec<_>>().join(" ")).filter(|d| !d.is_empty());
    s.version = json
        .pointer("/version/name")
        .and_then(Value::as_str)
        .map(strip_formatting);
    s.players_online = json
        .pointer("/players/online")
        .and_then(Value::as_u64)
        .map(|n| n as u32);
    s.players_max = json.pointer("/players/max").and_then(Value::as_u64).map(|n| n as u32);
    if let Some(sample) = json.pointer("/players/sample").and_then(Value::as_array) {
        s.players = sample
            .iter()
            .filter_map(|p| p.get("name").and_then(Value::as_str))
            .map(strip_formatting)
            // Servers often hide the list behind fake entries like "and 12 more"; keep only real-looking names.
            .filter(|n| !n.is_empty() && !n.contains(' '))
            .map(|name| Player { name, ping_ms: None })
            .collect();
    }
    s
}

pub struct MinecraftJava(pub GameKind);

#[async_trait::async_trait]
impl GameAdapter for MinecraftJava {
    fn kind(&self) -> GameKind {
        self.0
    }

    fn capabilities(&self, _server: &ServerConfig) -> Capabilities {
        Capabilities {
            player_names: true,
            ..Capabilities::default()
        }
    }

    async fn query(&self, server: &ServerConfig, now_unix: i64) -> Result<ServerStatus, QueryError> {
        let port = query_port(server);
        let started = Instant::now();
        let json = timeout(WAIT * 2, async {
            let mut stream = TcpStream::connect((server.host.as_str(), port))
                .await
                .map_err(unreachable)?;
            stream
                .write_all(&handshake(&server.host, port))
                .await
                .map_err(unreachable)?;
            stream.write_all(&frame(&[0x00])).await.map_err(unreachable)?; // status request
            let _len = read_varint(&mut stream).await?;
            let id = read_varint(&mut stream).await?;
            if id != 0 {
                return Err(QueryError::BadResponse(format!("unexpected packet id {id}")));
            }
            let text_len = read_varint(&mut stream).await?;
            let text_len = usize::try_from(text_len)
                .ok()
                .filter(|&n| n <= MAX_STATUS_LEN)
                .ok_or_else(|| QueryError::BadResponse("bad status length".into()))?;
            let mut buf = vec![0u8; text_len];
            stream.read_exact(&mut buf).await.map_err(unreachable)?;
            serde_json::from_slice::<Value>(&buf).map_err(|e| QueryError::BadResponse(e.to_string()))
        })
        .await
        .map_err(|_| QueryError::Unreachable("timed out".into()))??;
        let mut status = java_status(&json, now_unix);
        status.ping_ms = Some(started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32);
        Ok(status)
    }
}

// ---------- Bedrock Edition ----------

const RAKNET_MAGIC: [u8; 16] = [
    0x00, 0xFF, 0xFF, 0x00, 0xFE, 0xFE, 0xFE, 0xFE, 0xFD, 0xFD, 0xFD, 0xFD, 0x12, 0x34, 0x56, 0x78,
];

fn bedrock_ping(time: i64) -> Vec<u8> {
    let mut p = vec![0x01];
    p.extend_from_slice(&time.to_be_bytes());
    p.extend_from_slice(&RAKNET_MAGIC);
    p.extend_from_slice(&0x4C69_6768_7468_6F75i64.to_be_bytes()); // client guid
    p
}

/// Reply: 0x1C, time (8), server guid (8), magic (16), u16 length, status string.
fn bedrock_status(reply: &[u8], now_unix: i64) -> Result<ServerStatus, QueryError> {
    const HEADER: usize = 1 + 8 + 8 + 16;
    if reply.len() < HEADER + 2 || reply[0] != 0x1C {
        return Err(QueryError::BadResponse("not a RakNet pong".into()));
    }
    let len = u16::from_be_bytes([reply[HEADER], reply[HEADER + 1]]) as usize;
    let text = reply
        .get(HEADER + 2..HEADER + 2 + len)
        .ok_or_else(|| QueryError::BadResponse("truncated".into()))?;
    let text = String::from_utf8_lossy(text);
    // MCPE;motd;protocol;version;online;max;server id;level name;game mode;...
    let f: Vec<&str> = text.split(';').collect();
    let field = |i: usize| f.get(i).map(|v| strip_formatting(v)).filter(|v| !v.is_empty());
    let mut s = ServerStatus::new(ServerState::Online, now_unix);
    s.name = field(1);
    s.version = field(3);
    s.players_online = field(4).and_then(|v| v.parse().ok());
    s.players_max = field(5).and_then(|v| v.parse().ok());
    s.map = field(7);
    s.mode = field(8);
    Ok(s)
}

pub struct MinecraftBedrock(pub GameKind);

#[async_trait::async_trait]
impl GameAdapter for MinecraftBedrock {
    fn kind(&self) -> GameKind {
        self.0
    }

    fn capabilities(&self, _server: &ServerConfig) -> Capabilities {
        Capabilities::default()
    }

    async fn query(&self, server: &ServerConfig, now_unix: i64) -> Result<ServerStatus, QueryError> {
        let port = query_port(server);
        let addr = tokio::net::lookup_host((server.host.as_str(), port))
            .await
            .map_err(unreachable)?
            .next()
            .ok_or_else(|| unreachable("could not resolve host"))?;
        let bind: std::net::SocketAddr = if addr.is_ipv4() {
            ([0, 0, 0, 0], 0).into()
        } else {
            ([0u16; 8], 0).into()
        };
        let socket = UdpSocket::bind(bind).await.map_err(unreachable)?;
        socket.connect(addr).await.map_err(unreachable)?;
        let started = Instant::now();
        socket.send(&bedrock_ping(now_unix)).await.map_err(unreachable)?;
        let mut buf = vec![0u8; 2048];
        let n = timeout(WAIT, socket.recv(&mut buf))
            .await
            .map_err(|_| unreachable("timed out"))?
            .map_err(unreachable)?;
        let mut status = bedrock_status(&buf[..n], now_unix)?;
        status.ping_ms = Some(started.elapsed().as_millis().min(u128::from(u32::MAX)) as u32);
        Ok(status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varint_encoding() {
        let enc = |v| {
            let mut o = Vec::new();
            write_varint(&mut o, v);
            o
        };
        assert_eq!(enc(0), [0x00]);
        assert_eq!(enc(300), [0xAC, 0x02]);
        assert_eq!(enc(-1), [0xFF, 0xFF, 0xFF, 0xFF, 0x0F]);
    }

    #[tokio::test]
    async fn varint_decoding() {
        let mut r: &[u8] = &[0xAC, 0x02];
        assert_eq!(read_varint(&mut r).await.unwrap(), 300);
    }

    #[test]
    fn java_status_json() {
        let json: Value = serde_json::from_str(
            r#"{"version":{"name":"1.21.1","protocol":767},
                "players":{"max":20,"online":2,"sample":[{"name":"Steve","id":"x"},{"name":"§7and 5 more","id":"y"}]},
                "description":{"text":"§aMy ","extra":[{"text":"Server\n§7Survival, no resets"}]}}"#,
        )
        .unwrap();
        let s = java_status(&json, 0);
        assert_eq!(s.name.as_deref(), Some("My Server"));
        assert_eq!(s.description.as_deref(), Some("Survival, no resets"));
        assert_eq!(s.version.as_deref(), Some("1.21.1"));
        assert_eq!((s.players_online, s.players_max), (Some(2), Some(20)));
        assert_eq!(s.players.len(), 1);
    }

    #[test]
    fn bedrock_pong() {
        let text = b"MCPE;Dedicated Server;712;1.21.20;3;10;123;Bedrock level;Survival;1;19132;19133;";
        let mut reply = vec![0x1C];
        reply.extend_from_slice(&[0; 16]);
        reply.extend_from_slice(&RAKNET_MAGIC);
        reply.extend_from_slice(&(text.len() as u16).to_be_bytes());
        reply.extend_from_slice(text);
        let s = bedrock_status(&reply, 0).unwrap();
        assert_eq!(s.name.as_deref(), Some("Dedicated Server"));
        assert_eq!(s.version.as_deref(), Some("1.21.20"));
        assert_eq!((s.players_online, s.players_max), (Some(3), Some(10)));
        assert_eq!(s.map.as_deref(), Some("Bedrock level"));
        assert_eq!(s.mode.as_deref(), Some("Survival"));
    }
}
