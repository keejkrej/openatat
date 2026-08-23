//! Persist Orb rest position per output. Hide is not stored.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::paths::orb_position_path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct OrbPos {
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OrbPositions {
    #[serde(default)]
    pub by_output: BTreeMap<String, OrbPos>,
}

impl OrbPositions {
    pub fn load() -> Self {
        let path = orb_position_path();
        match std::fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn get(&self, output: &str) -> Option<OrbPos> {
        self.by_output.get(output).copied()
    }

    pub fn set(&mut self, output: String, pos: OrbPos) {
        self.by_output.insert(output, pos);
    }

    pub fn save(&self) {
        let path = orb_position_path();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(raw) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(path, raw);
        }
    }
}

/// Default rest: lower-right of the output, inset so the circle stays on-screen.
pub fn default_pos(output_w: i32, output_h: i32, orb: u32) -> OrbPos {
    let inset = 24;
    let orb = orb as i32;
    OrbPos {
        x: (output_w - orb - inset).max(inset),
        y: (output_h - orb - 80).max(inset),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_stays_on_output() {
        let p = default_pos(1920, 1080, 56);
        assert!(p.x + 56 <= 1920);
        assert!(p.y + 56 <= 1080);
        assert!(p.x >= 0 && p.y >= 0);
    }

    #[test]
    fn roundtrip_per_output() {
        let _g = crate::paths::xdg_test_lock();
        let dir = std::env::temp_dir().join(format!("openatat-orbpos-{}", uuid::Uuid::new_v4()));
        std::env::set_var("XDG_DATA_HOME", &dir);
        let mut store = OrbPositions::default();
        store.set("DP-1".into(), OrbPos { x: 100, y: 200 });
        store.save();
        let loaded = OrbPositions::load();
        assert_eq!(loaded.get("DP-1"), Some(OrbPos { x: 100, y: 200 }));
        assert_eq!(loaded.get("eDP-1"), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
