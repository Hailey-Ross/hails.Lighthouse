//! Steam server query (A2S) client: INFO, PLAYER and RULES.
//!
//! Protocol: <https://developer.valvesoftware.com/wiki/Server_queries>. Handles the challenge
//! handshake (including the A2S_INFO challenge) and split replies. Compressed split replies
//! (old Source engine only) are not supported.

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use tokio::net::UdpSocket;
use tokio::time::timeout;

const SINGLE: [u8; 4] = [0xFF; 4];
const SPLIT: [u8; 4] = [0xFE, 0xFF, 0xFF, 0xFF];
const NO_CHALLENGE: [u8; 4] = [0xFF; 4];

const S2C_CHALLENGE: u8 = b'A';
const S2A_INFO: u8 = b'I';
const S2A_PLAYER: u8 = b'D';
const S2A_RULES: u8 = b'E';

#[derive(Debug, thiserror::Error)]
pub enum A2sError {
    #[error("network error: {0}")]
    Io(#[from] std::io::Error),
    #[error("no reply within {0:?}")]
    Timeout(Duration),
    #[error("could not resolve {0}")]
    Resolve(String),
    #[error("malformed reply: {0}")]
    Malformed(&'static str),
    #[error("compressed split replies are not supported")]
    Compressed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    pub name: String,
    pub map: String,
    pub folder: String,
    pub game: String,
    pub app_id: u16,
    pub players: u8,
    pub max_players: u8,
    pub bots: u8,
    /// `d` dedicated, `l` listen, `p` SourceTV.
    pub server_type: char,
    /// `l` Linux, `w` Windows, `m`/`o` macOS.
    pub environment: char,
    pub password: bool,
    pub vac: bool,
    pub version: String,
    pub port: Option<u16>,
    pub steam_id: Option<u64>,
    pub keywords: Option<String>,
    pub game_id: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct A2sPlayer {
    pub name: String,
    pub score: i32,
    /// Seconds connected.
    pub duration: f32,
}

pub type Rules = BTreeMap<String, String>;

struct Cursor<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }
    fn take(&mut self, n: usize) -> Result<&'a [u8], A2sError> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|&e| e <= self.buf.len())
            .ok_or(A2sError::Malformed("truncated"))?;
        let s = &self.buf[self.pos..end];
        self.pos = end;
        Ok(s)
    }
    fn u8(&mut self) -> Result<u8, A2sError> {
        Ok(self.take(1)?[0])
    }
    fn u16(&mut self) -> Result<u16, A2sError> {
        let b = self.take(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }
    fn i32(&mut self) -> Result<i32, A2sError> {
        let b = self.take(4)?;
        Ok(i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn f32(&mut self) -> Result<f32, A2sError> {
        let b = self.take(4)?;
        Ok(f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }
    fn u64(&mut self) -> Result<u64, A2sError> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes(b.try_into().expect("8 bytes")))
    }
    fn cstr(&mut self) -> Result<String, A2sError> {
        let rest = &self.buf[self.pos..];
        let len = rest
            .iter()
            .position(|&b| b == 0)
            .ok_or(A2sError::Malformed("unterminated string"))?;
        let s = String::from_utf8_lossy(&rest[..len]).into_owned();
        self.pos += len + 1;
        Ok(s)
    }
    fn has_more(&self) -> bool {
        self.pos < self.buf.len()
    }
}

/// Parse an A2S_INFO reply payload (after the `FF FF FF FF` header, starting at `I`).
pub fn parse_info(payload: &[u8]) -> Result<Info, A2sError> {
    let mut c = Cursor::new(payload);
    if c.u8()? != S2A_INFO {
        return Err(A2sError::Malformed("not an INFO reply"));
    }
    let _protocol = c.u8()?;
    let name = c.cstr()?;
    let map = c.cstr()?;
    let folder = c.cstr()?;
    let game = c.cstr()?;
    let app_id = c.u16()?;
    let players = c.u8()?;
    let max_players = c.u8()?;
    let bots = c.u8()?;
    let server_type = c.u8()? as char;
    let environment = c.u8()? as char;
    let password = c.u8()? != 0;
    let vac = c.u8()? != 0;
    let version = c.cstr()?;
    let mut info = Info {
        name,
        map,
        folder,
        game,
        app_id,
        players,
        max_players,
        bots,
        server_type,
        environment,
        password,
        vac,
        version,
        port: None,
        steam_id: None,
        keywords: None,
        game_id: None,
    };
    if c.has_more() {
        let edf = c.u8()?;
        if edf & 0x80 != 0 {
            info.port = Some(c.u16()?);
        }
        if edf & 0x10 != 0 {
            info.steam_id = Some(c.u64()?);
        }
        if edf & 0x40 != 0 {
            let _tv_port = c.u16()?;
            let _tv_name = c.cstr()?;
        }
        if edf & 0x20 != 0 {
            info.keywords = Some(c.cstr()?);
        }
        if edf & 0x01 != 0 {
            info.game_id = Some(c.u64()?);
        }
    }
    Ok(info)
}

/// Parse an A2S_PLAYER reply payload (starting at `D`). Empty names (players still connecting) are kept.
pub fn parse_players(payload: &[u8]) -> Result<Vec<A2sPlayer>, A2sError> {
    let mut c = Cursor::new(payload);
    if c.u8()? != S2A_PLAYER {
        return Err(A2sError::Malformed("not a PLAYER reply"));
    }
    let count = c.u8()?;
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        if !c.has_more() {
            break; // some servers report more players than they list
        }
        let _index = c.u8()?;
        let name = c.cstr()?;
        let score = c.i32()?;
        let duration = c.f32()?;
        out.push(A2sPlayer { name, score, duration });
    }
    Ok(out)
}

/// Parse an A2S_RULES reply payload (starting at `E`).
pub fn parse_rules(payload: &[u8]) -> Result<Rules, A2sError> {
    let mut c = Cursor::new(payload);
    if c.u8()? != S2A_RULES {
        return Err(A2sError::Malformed("not a RULES reply"));
    }
    let count = c.u16()?;
    let mut rules = Rules::new();
    for _ in 0..count {
        if !c.has_more() {
            break;
        }
        let key = c.cstr()?;
        let value = c.cstr()?;
        rules.insert(key, value);
    }
    Ok(rules)
}

/// Header of one part of a split reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SplitHeader {
    id: i32,
    total: u8,
    number: u8,
}

/// Parse a split packet: `FE FF FF FF`, id, total, number, max size, then the data.
fn parse_split(packet: &[u8]) -> Result<(SplitHeader, &[u8]), A2sError> {
    let mut c = Cursor::new(packet);
    c.take(4)?;
    let id = c.i32()?;
    if (id as u32) & 0x8000_0000 != 0 {
        return Err(A2sError::Compressed);
    }
    let total = c.u8()?;
    let number = c.u8()?;
    let _size = c.u16()?;
    if total == 0 || number >= total {
        return Err(A2sError::Malformed("bad split numbering"));
    }
    Ok((SplitHeader { id, total, number }, &packet[c.pos..]))
}

/// Join split parts (indexed by number) and strip the `FF FF FF FF` header of the whole reply.
fn reassemble(parts: Vec<Vec<u8>>) -> Result<Vec<u8>, A2sError> {
    let joined: Vec<u8> = parts.into_iter().flatten().collect();
    match joined.strip_prefix(&SINGLE) {
        Some(rest) => Ok(rest.to_vec()),
        None => Err(A2sError::Malformed("split reply without header")),
    }
}

/// One Steam query session against a server.
pub struct A2sClient {
    socket: UdpSocket,
    wait: Duration,
}

impl A2sClient {
    pub async fn connect(host: &str, port: u16, wait: Duration) -> Result<Self, A2sError> {
        let addr: SocketAddr = tokio::net::lookup_host((host, port))
            .await?
            .next()
            .ok_or_else(|| A2sError::Resolve(format!("{host}:{port}")))?;
        let bind: SocketAddr = if addr.is_ipv4() {
            ([0, 0, 0, 0], 0).into()
        } else {
            ([0u16; 8], 0).into()
        };
        let socket = UdpSocket::bind(bind).await?;
        socket.connect(addr).await?;
        Ok(Self { socket, wait })
    }

    async fn recv_reply(&self) -> Result<Vec<u8>, A2sError> {
        let mut buf = vec![0u8; 65_535];
        let mut parts: Vec<Option<Vec<u8>>> = Vec::new();
        let mut split_id = None;
        loop {
            let n = timeout(self.wait, self.socket.recv(&mut buf))
                .await
                .map_err(|_| A2sError::Timeout(self.wait))??;
            let packet = &buf[..n];
            if let Some(rest) = packet.strip_prefix(&SINGLE) {
                return Ok(rest.to_vec());
            }
            if !packet.starts_with(&SPLIT) {
                return Err(A2sError::Malformed("unknown packet header"));
            }
            let (header, data) = parse_split(packet)?;
            match split_id {
                None => {
                    split_id = Some(header.id);
                    parts = vec![None; header.total as usize];
                }
                Some(id) if id != header.id => continue, // stray part of another reply
                Some(_) => {}
            }
            if let Some(slot) = parts.get_mut(header.number as usize) {
                *slot = Some(data.to_vec());
            }
            if parts.iter().all(Option::is_some) {
                return reassemble(parts.into_iter().map(|p| p.expect("checked")).collect());
            }
        }
    }

    /// Send a request, answering up to three challenges. `with_challenge` builds the packet.
    /// Returns the reply and the round-trip time of the final request.
    async fn ask(&self, with_challenge: impl Fn(Option<[u8; 4]>) -> Vec<u8>) -> Result<(Vec<u8>, Duration), A2sError> {
        let mut challenge = None;
        for _ in 0..3 {
            let sent = Instant::now();
            self.socket.send(&with_challenge(challenge)).await?;
            let reply = self.recv_reply().await?;
            match reply.first() {
                Some(&S2C_CHALLENGE) if reply.len() >= 5 => {
                    challenge = Some([reply[1], reply[2], reply[3], reply[4]]);
                }
                Some(_) => return Ok((reply, sent.elapsed())),
                None => return Err(A2sError::Malformed("empty reply")),
            }
        }
        Err(A2sError::Malformed("server kept sending challenges"))
    }

    /// A2S_INFO and the round-trip time of the final request.
    pub async fn info(&self) -> Result<(Info, Duration), A2sError> {
        let (payload, rtt) = self
            .ask(|challenge| {
                let mut p = SINGLE.to_vec();
                p.push(b'T');
                p.extend_from_slice(b"Source Engine Query\0");
                if let Some(c) = challenge {
                    p.extend_from_slice(&c);
                }
                p
            })
            .await?;
        Ok((parse_info(&payload)?, rtt))
    }

    pub async fn players(&self) -> Result<Vec<A2sPlayer>, A2sError> {
        let (payload, _) = self
            .ask(|c| [&SINGLE[..], &b"U"[..], &c.unwrap_or(NO_CHALLENGE)[..]].concat())
            .await?;
        parse_players(&payload)
    }

    pub async fn rules(&self) -> Result<Rules, A2sError> {
        let (payload, _) = self
            .ask(|c| [&SINGLE[..], &b"V"[..], &c.unwrap_or(NO_CHALLENGE)[..]].concat())
            .await?;
        parse_rules(&payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// INFO reply modelled on a live Project Zomboid B42 server.
    fn pz_info() -> Vec<u8> {
        let mut p = vec![b'I', 17];
        for s in ["Safehouse 3x XP", "Muldraugh, KY", "zomboid", "Project Zomboid"] {
            p.extend_from_slice(s.as_bytes());
            p.push(0);
        }
        p.extend_from_slice(&0u16.to_le_bytes());
        p.extend_from_slice(&[0, 24, 0, b'd', b'w', 0, 1]);
        p.extend_from_slice(b"1.0.0.0\0");
        p.push(0x80 | 0x10 | 0x20 | 0x01);
        p.extend_from_slice(&8766u16.to_le_bytes());
        p.extend_from_slice(&90297121903002624u64.to_le_bytes());
        p.extend_from_slice(b";vanilla;VERSION:42.21\0");
        p.extend_from_slice(&108600u64.to_le_bytes());
        p
    }

    #[test]
    fn parses_info_with_extra_data() {
        let info = parse_info(&pz_info()).unwrap();
        assert_eq!(info.name, "Safehouse 3x XP");
        assert_eq!(info.map, "Muldraugh, KY");
        assert_eq!((info.players, info.max_players), (0, 24));
        assert_eq!(info.server_type, 'd');
        assert!(!info.password);
        assert_eq!(info.port, Some(8766));
        assert_eq!(info.keywords.as_deref(), Some(";vanilla;VERSION:42.21"));
        assert_eq!(info.game_id, Some(108600));
    }

    #[test]
    fn truncated_info_is_an_error() {
        let full = pz_info();
        assert!(parse_info(&full[..20]).is_err());
    }

    #[test]
    fn parses_players_and_rules() {
        let mut p = vec![b'D', 2];
        for (name, secs) in [("Alice", 61.5f32), ("", 2.0)] {
            p.push(0);
            p.extend_from_slice(name.as_bytes());
            p.push(0);
            p.extend_from_slice(&0i32.to_le_bytes());
            p.extend_from_slice(&secs.to_le_bytes());
        }
        let players = parse_players(&p).unwrap();
        assert_eq!(players.len(), 2);
        assert_eq!(players[0].name, "Alice");

        let mut r = vec![b'E'];
        r.extend_from_slice(&2u16.to_le_bytes());
        r.extend_from_slice(b"version\x0042.21\x00pvp\x000\x00");
        let rules = parse_rules(&r).unwrap();
        assert_eq!(rules.get("version").map(String::as_str), Some("42.21"));
        assert_eq!(rules.get("pvp").map(String::as_str), Some("0"));
    }

    #[test]
    fn split_header_and_reassembly() {
        let mut part0 = SPLIT.to_vec();
        part0.extend_from_slice(&7i32.to_le_bytes());
        part0.extend_from_slice(&[2, 0]);
        part0.extend_from_slice(&1248u16.to_le_bytes());
        part0.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF, b'E', 1]);
        let (h, data) = parse_split(&part0).unwrap();
        assert_eq!(
            h,
            SplitHeader {
                id: 7,
                total: 2,
                number: 0
            }
        );
        let joined = reassemble(vec![data.to_vec(), vec![0, b'k', 0, b'v', 0]]).unwrap();
        assert_eq!(parse_rules(&joined).unwrap().get("k").map(String::as_str), Some("v"));
    }

    #[test]
    fn compressed_split_rejected() {
        let mut p = SPLIT.to_vec();
        p.extend_from_slice(&(0x8000_0001u32 as i32).to_le_bytes());
        p.extend_from_slice(&[1, 0, 0, 0]);
        assert!(matches!(parse_split(&p), Err(A2sError::Compressed)));
    }
}
