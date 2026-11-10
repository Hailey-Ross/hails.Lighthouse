//! Small JSON state file: which Discord message shows which server, and since when each server
//! has been online. Kept next to the config so Lighthouse edits the same posts after a restart.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tracing::warn;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageRef {
    pub channel_id: u64,
    pub message_id: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct StateData {
    /// Key: `<bot>/<server id>` or `<bot>/board`.
    #[serde(default)]
    messages: BTreeMap<String, MessageRef>,
    /// Unix time each server was first seen online in its current run.
    #[serde(default)]
    online_since: BTreeMap<String, i64>,
}

pub struct Persist {
    path: PathBuf,
    data: Mutex<StateData>,
}

impl Persist {
    /// Load the state file, starting empty if it is missing or unreadable.
    pub fn load(path: &Path) -> Self {
        let data = match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                warn!("ignoring unreadable state file {}: {e}", path.display());
                StateData::default()
            }),
            Err(_) => StateData::default(),
        };
        Self {
            path: path.to_path_buf(),
            data: Mutex::new(data),
        }
    }

    fn with<T>(&self, f: impl FnOnce(&mut StateData) -> T, save: bool) -> T {
        let mut data = self.data.lock().unwrap_or_else(|p| p.into_inner());
        let out = f(&mut data);
        if save {
            if let Err(e) = write_atomically(&self.path, &data) {
                warn!("could not save {}: {e}", self.path.display());
            }
        }
        out
    }

    pub fn message(&self, key: &str) -> Option<MessageRef> {
        self.with(|d| d.messages.get(key).copied(), false)
    }

    pub fn set_message(&self, key: &str, message: MessageRef) {
        self.with(|d| d.messages.insert(key.to_string(), message), true);
    }

    pub fn remove_message(&self, key: &str) {
        self.with(|d| d.messages.remove(key), true);
    }

    pub fn online_since(&self, server_id: &str) -> Option<i64> {
        self.with(|d| d.online_since.get(server_id).copied(), false)
    }

    pub fn set_online_since(&self, server_id: &str, since: Option<i64>) {
        self.with(
            |d| match since {
                Some(t) => d.online_since.insert(server_id.to_string(), t),
                None => d.online_since.remove(server_id),
            },
            true,
        );
    }
}

fn write_atomically(path: &Path, data: &StateData) -> std::io::Result<()> {
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(data)?)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_through_file() {
        let dir = std::env::temp_dir().join(format!("lighthouse-persist-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.json");
        {
            let p = Persist::load(&path);
            p.set_message(
                "7dtd/athens",
                MessageRef {
                    channel_id: 1,
                    message_id: 2,
                },
            );
            p.set_online_since("athens", Some(100));
        }
        let p = Persist::load(&path);
        assert_eq!(
            p.message("7dtd/athens"),
            Some(MessageRef {
                channel_id: 1,
                message_id: 2
            })
        );
        assert_eq!(p.online_since("athens"), Some(100));
        p.set_online_since("athens", None);
        assert_eq!(p.online_since("athens"), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
