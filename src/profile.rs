// SPDX-License-Identifier: GPL-3.0-or-later

//! Saving and loading the editor state as JSON.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gtk::glib;
use serde::{Deserialize, Serialize};

use crate::editor::Document;
use crate::effects::LightingFx;
use crate::layout::{self, Region};
use crate::model::{Bitmap, Lighting, Rgb};
use crate::screen_fx::{Animation, ScreenFx};

/// Version 2 added effects, screen modes and animations; version 1 files still load.
const VERSION: u32 = 2;

#[derive(Serialize, Deserialize)]
pub struct Profile {
    pub version: u32,
    pub layout: Region,
    /// LED id as `0x..` → color
    pub keys: BTreeMap<String, Rgb>,
    /// 40 rows of 32 hex digits, see [`Bitmap::to_rows`]
    pub screen: Bitmap,
    #[serde(default)]
    pub effects: LightingFx,
    #[serde(default)]
    pub screen_mode: ScreenFx,
    #[serde(default, skip_serializing_if = "is_empty")]
    pub animation: Animation,
}

fn is_empty(animation: &Animation) -> bool {
    animation.frames.is_empty()
}

impl Profile {
    pub fn capture(doc: &Document) -> Self {
        // Keys of both layouts, so switching layout later keeps every color.
        let mut keys = BTreeMap::new();
        for key in layout::keys(Region::Ansi).into_iter().chain(layout::keys(Region::Iso)) {
            keys.insert(format!("0x{:02x}", key.led), doc.lighting.get(key.led));
        }
        Self {
            version: VERSION,
            layout: doc.region,
            keys,
            screen: doc.screen.clone(),
            effects: doc.lighting_fx.clone(),
            screen_mode: doc.screen_fx.clone(),
            animation: doc.animation.clone(),
        }
    }

    pub fn into_document(self) -> Result<Document, String> {
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
        if self.animation.frames.len() != self.animation.delays_ms.len() {
            return Err("the animation is malformed".into());
        }
        Ok(Document {
            region: self.layout,
            lighting,
            screen: self.screen,
            lighting_fx: self.effects,
            screen_fx: self.screen_mode,
            animation: self.animation,
        })
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

/// App preferences that aren't part of a profile.
#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Settings {
    pub send_to_keyboard: bool,
}

impl Settings {
    fn path() -> PathBuf {
        glib::user_config_dir().join("open-steel-studio").join("settings.json")
    }

    pub fn load() -> Self {
        std::fs::read_to_string(Self::path()).ok().and_then(|text| serde_json::from_str(&text).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let path = Self::path();
        let result = std::fs::create_dir_all(path.parent().unwrap_or(&path))
            .and_then(|()| std::fs::write(&path, serde_json::to_string_pretty(self).unwrap_or_default()));
        if let Err(e) = result {
            eprintln!("Couldn't save settings: {e}");
        }
    }
}

/// Where the editor remembers its state between runs.
pub fn session_path() -> PathBuf {
    glib::user_config_dir().join("open-steel-studio").join("session.json")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::Background;
    use crate::screen_fx::ScreenMode;

    #[test]
    fn round_trip() {
        let mut doc = Document::default();
        doc.lighting.set(0x04, Rgb::new(1, 2, 3));
        doc.lighting.set(0x64, Rgb::new(4, 5, 6)); // ISO-only key survives in an ANSI profile
        doc.screen = Bitmap::from_fn(|x, y| x == y);
        doc.lighting_fx.background = Background::Wave;
        doc.screen_fx.mode = ScreenMode::Animation;
        doc.animation = Animation { frames: vec![Bitmap::default(), doc.screen.clone()], delays_ms: vec![100, 50] };

        let json = serde_json::to_string(&Profile::capture(&doc)).unwrap();
        let loaded = serde_json::from_str::<Profile>(&json).unwrap().into_document().unwrap();
        assert_eq!(loaded.region, Region::Ansi);
        assert_eq!(loaded.lighting.get(0x04), Rgb::new(1, 2, 3));
        assert_eq!(loaded.lighting.get(0x64), Rgb::new(4, 5, 6));
        assert!(loaded.screen == doc.screen);
        assert_eq!(loaded.lighting_fx, doc.lighting_fx);
        assert_eq!(loaded.screen_fx, doc.screen_fx);
        assert_eq!(loaded.animation, doc.animation);
    }

    #[test]
    fn loads_version_1_files() {
        let rows: Vec<String> = vec!["0".repeat(32); 40];
        let json = serde_json::json!({ "version": 1, "layout": "iso", "keys": { "0x04": "#ff0000" }, "screen": rows });
        let doc = serde_json::from_value::<Profile>(json).unwrap().into_document().unwrap();
        assert_eq!(doc.region, Region::Iso);
        assert_eq!(doc.lighting.get(0x04), Rgb::new(255, 0, 0));
        assert_eq!(doc.lighting_fx, LightingFx::default());
    }

    #[test]
    fn rejects_bad_ids() {
        let profile = Profile::capture(&Document::default());
        let mut json = serde_json::to_value(&profile).unwrap();
        json["keys"] = serde_json::json!({ "A": "#000000" });
        assert!(serde_json::from_value::<Profile>(json).unwrap().into_document().is_err());
    }
}
