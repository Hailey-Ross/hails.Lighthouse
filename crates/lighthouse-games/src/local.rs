//! Sources that only work when the game server runs on the same machine as Lighthouse:
//!
//! - the start time of the server's process (`process` in the config), for real uptime, e.g. for
//!   VEIN, which has no mods;
//! - a status file written by a game mod (`status_file`), e.g. the Project Zomboid mod, whose Lua
//!   code can't make network requests.

use std::path::Path;
use std::time::{Duration, SystemTime};

use lighthouse_core::ServerStatus;

/// A status file older than this is ignored (the mod writes it every 30 seconds).
pub const STATUS_FILE_MAX_AGE: Duration = Duration::from_secs(120);

/// `VeinServer.exe` and `veinserver` both match `VeinServer`.
fn same_program(found: &str, wanted: &str) -> bool {
    let strip = |s: &str| {
        let s = s.trim();
        let s = s.rsplit(['/', '\\']).next().unwrap_or(s);
        s.strip_suffix(".exe")
            .or_else(|| s.strip_suffix(".EXE"))
            .unwrap_or(s)
            .to_ascii_lowercase()
    };
    !wanted.trim().is_empty() && strip(found) == strip(wanted)
}

/// Pick the start time when exactly one process matched: with several (two servers of the same
/// game on one machine), there's no telling which is which.
fn only_one(starts: Vec<i64>) -> Option<i64> {
    match starts.as_slice() {
        [one] => Some(*one),
        _ => None,
    }
}

/// When the process named `program` started (Unix seconds), if exactly one such process runs.
pub fn process_started_at(program: &str) -> Option<i64> {
    only_one(platform::start_times(program))
}

/// Read a mod's status file. The file holds a `status` object as in `docs/heartbeat.md`.
///
/// A file written recently is used in full. An older one only gives its start time, as a status
/// with state `Unknown`: Project Zomboid stops its game loop (and the mod's writes) while nobody
/// is online, but the start time stays right until the next restart, which rewrites the file.
pub fn read_status_file(path: &Path, now: SystemTime) -> Option<ServerStatus> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let text = std::fs::read_to_string(path).ok()?;
    let status: ServerStatus = match serde_json::from_str(&text) {
        Ok(status) => status,
        Err(e) => {
            tracing::debug!("status file {}: {e}", path.display());
            return None;
        }
    };
    if now.duration_since(modified).unwrap_or_default() <= STATUS_FILE_MAX_AGE {
        return Some(status);
    }
    let mut hint = ServerStatus::new(lighthouse_core::ServerState::Unknown, status.observed_at_unix);
    hint.started_at_unix = status.started_at_unix;
    hint.started_at_unix.is_some().then_some(hint)
}

#[cfg(target_os = "linux")]
mod platform {
    use super::same_program;

    /// Clock ticks per second for `/proc/<pid>/stat` times. 100 on every mainstream Linux.
    const TICKS: i64 = 100;

    fn boot_time() -> Option<i64> {
        let stat = std::fs::read_to_string("/proc/stat").ok()?;
        stat.lines().find_map(|l| l.strip_prefix("btime ")?.trim().parse().ok())
    }

    pub fn start_times(program: &str) -> Vec<i64> {
        let Some(boot) = boot_time() else { return Vec::new() };
        let Ok(dir) = std::fs::read_dir("/proc") else {
            return Vec::new();
        };
        dir.flatten()
            .filter(|e| e.file_name().to_string_lossy().bytes().all(|b| b.is_ascii_digit()))
            .filter_map(|e| {
                let p = e.path();
                // argv[0] holds the full name; `comm` is cut to 15 characters.
                let cmdline = std::fs::read(p.join("cmdline")).ok()?;
                let argv0 = String::from_utf8_lossy(cmdline.split(|&b| b == 0).next()?).into_owned();
                let comm = std::fs::read_to_string(p.join("comm")).unwrap_or_default();
                if !same_program(&argv0, program) && !same_program(&comm, program) {
                    return None;
                }
                let stat = std::fs::read_to_string(p.join("stat")).ok()?;
                // Field 22 (starttime), counted after the `)` that ends the program name.
                let after = &stat[stat.rfind(')')? + 1..];
                let ticks: i64 = after.split_whitespace().nth(19)?.parse().ok()?;
                Some(boot + ticks / TICKS)
            })
            .collect()
    }
}

#[cfg(windows)]
mod platform {
    use super::same_program;
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

    /// FILETIME counts 100 ns steps since 1601.
    fn unix_from_filetime(ft: &FILETIME) -> i64 {
        let t = (u64::from(ft.dwHighDateTime) << 32) | u64::from(ft.dwLowDateTime);
        (t / 10_000_000) as i64 - 11_644_473_600
    }

    fn started(pid: u32) -> Option<i64> {
        // SAFETY: plain Win32 calls with owned, zeroed out-parameters; the handle is closed below.
        unsafe {
            let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if h.is_null() {
                return None;
            }
            let zero = FILETIME {
                dwLowDateTime: 0,
                dwHighDateTime: 0,
            };
            let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
            let ok = GetProcessTimes(h, &mut created, &mut exited, &mut kernel, &mut user);
            CloseHandle(h);
            (ok != 0).then(|| unix_from_filetime(&created))
        }
    }

    pub fn start_times(program: &str) -> Vec<i64> {
        let mut out = Vec::new();
        // SAFETY: the snapshot handle is checked and closed; the entry struct is sized as required.
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap == INVALID_HANDLE_VALUE {
                return out;
            }
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut more = Process32FirstW(snap, &mut entry) != 0;
            while more {
                let len = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
                if same_program(&name, program)
                    && let Some(t) = started(entry.th32ProcessID)
                {
                    out.push(t);
                }
                more = Process32NextW(snap, &mut entry) != 0;
            }
            CloseHandle(snap);
        }
        out
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
mod platform {
    pub fn start_times(_program: &str) -> Vec<i64> {
        Vec::new()
    }
}

/// Unix seconds of a time (tests only).
#[cfg(test)]
fn unix(t: SystemTime) -> i64 {
    t.duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_names() {
        assert!(same_program("C:\\Games\\VeinServer.exe", "VeinServer"));
        assert!(same_program("/opt/vein/veinserver", "VeinServer.exe"));
        assert!(!same_program("VeinServer-Win64-Shipping.exe", "VeinServer"));
        assert!(!same_program("anything", " "));
        assert_eq!(only_one(vec![5]), Some(5));
        assert_eq!(only_one(vec![5, 6]), None);
        assert_eq!(only_one(vec![]), None);
    }

    #[test]
    fn finds_this_test_process() {
        let me = std::env::current_exe().unwrap();
        let name = me.file_name().unwrap().to_string_lossy().into_owned();
        let starts = platform::start_times(&name);
        if cfg!(any(target_os = "linux", windows)) {
            let now = unix(SystemTime::now());
            assert!(starts.iter().any(|&t| t <= now && now - t < 3600), "{starts:?}");
        }
    }

    #[test]
    fn status_file_freshness() {
        let dir = std::env::temp_dir().join(format!("lighthouse-status-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("lighthouse_status.json");
        std::fs::write(
            &path,
            r#"{"state":"online","players_online":3,"started_at_unix":10,"observed_at_unix":20}"#,
        )
        .unwrap();
        let s = read_status_file(&path, SystemTime::now()).unwrap();
        assert_eq!((s.players_online, s.started_at_unix), (Some(3), Some(10)));
        let later = SystemTime::now() + STATUS_FILE_MAX_AGE + Duration::from_secs(5);
        let stale = read_status_file(&path, later).unwrap();
        assert_eq!(
            (stale.state, stale.started_at_unix, stale.players_online),
            (lighthouse_core::ServerState::Unknown, Some(10), None)
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
