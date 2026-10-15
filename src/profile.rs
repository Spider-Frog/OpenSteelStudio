//! Saving and loading the editor state as JSON.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gtk::glib;
use serde::{Deserialize, Serialize};

use crate::editor::Editor;
use crate::layout::{self, Region};
use crate::model::{Bitmap, Lighting, Rgb};

const VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
pub struct Profile {
    pub version: u32,
    pub layout: Region,
    /// LED id as `0x..` → color
    pub keys: BTreeMap<String, Rgb>,
    /// 40 rows of 32 hex digits, see [`Bitmap::to_rows`]
    pub screen: Vec<String>,
}

impl Profile {
    pub fn capture(editor: &Editor) -> Self {
        let lighting = editor.lighting();
        // Keys of both layouts, so switching layout later keeps every color.
        let mut keys = BTreeMap::new();
        for key in layout::keys(Region::Ansi).into_iter().chain(layout::keys(Region::Iso)) {
            keys.insert(format!("0x{:02x}", key.led), lighting.get(key.led));
        }
        Self { version: VERSION, layout: editor.region(), keys, screen: editor.screen().to_rows() }
    }

    pub fn into_parts(self) -> Result<(Region, Lighting, Bitmap), String> {
        if self.version > VERSION {
            return Err(format!("made by a newer version of the editor (format {})", self.version));
        }
        let mut lighting = Lighting::default();
        for (id, color) in self.keys {
            let led = id
                .strip_prefix("0x")
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
                .ok_or_else(|| format!("invalid key id {id:?}"))?;
            lighting.set(led, color);
        }
        let screen = Bitmap::from_rows(&self.screen).ok_or("the screen image is malformed")?;
        Ok((self.layout, lighting, screen))
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        serde_json::from_str(&text).map_err(|e| e.to_string())
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| e.to_string())
    }
}

/// Where the editor remembers its state between runs.
pub fn session_path() -> PathBuf {
    glib::user_config_dir().join("open-steel-studio").join("session.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut lighting = Lighting::default();
        lighting.set(0x04, Rgb::new(1, 2, 3));
        lighting.set(0x64, Rgb::new(4, 5, 6)); // ISO-only key survives in an ANSI profile
        let screen = Bitmap::from_fn(|x, y| x == y);
        let editor = Editor::new(Region::Ansi, lighting, screen.clone());

        let json = serde_json::to_string(&Profile::capture(&editor)).unwrap();
        let (region, lighting, loaded) = serde_json::from_str::<Profile>(&json).unwrap().into_parts().unwrap();
        assert_eq!(region, Region::Ansi);
        assert_eq!(lighting.get(0x04), Rgb::new(1, 2, 3));
        assert_eq!(lighting.get(0x64), Rgb::new(4, 5, 6));
        assert!(loaded == screen);
    }

    #[test]
    fn rejects_bad_ids() {
        let profile = Profile { version: 1, layout: Region::Iso, keys: [("A".into(), Rgb::BLACK)].into(), screen: Bitmap::default().to_rows() };
        assert!(profile.into_parts().is_err());
    }
}
