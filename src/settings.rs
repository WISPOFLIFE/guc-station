//! Preferences on disk, plus the folder where raw portal pages are saved.

use crate::theme::AppTheme;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub theme: AppTheme,
    /// Where CMS files and mail attachments are saved. None until the user
    /// has picked one, which they're asked to do after first signing in.
    pub download_dir: Option<PathBuf>,
}

/// The folder suggested on first start: Downloads/GUC Station.
pub fn default_download_dir() -> PathBuf {
    dirs::download_dir().or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from(".")).join(FOLDER_NAME)
}

pub const FOLDER_NAME: &str = "GUC Station";

/// The GUC Station folder inside a location the user picked. Picking a
/// folder that's already called GUC Station uses it as it is.
pub fn station_folder(picked: &std::path::Path) -> PathBuf {
    let named = picked.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.eq_ignore_ascii_case(FOLDER_NAME));
    if named { picked.to_path_buf() } else { picked.join(FOLDER_NAME) }
}

fn data_dir() -> PathBuf {
    dirs::data_local_dir().unwrap_or_else(|| PathBuf::from(".")).join("GUC Station")
}

impl Settings {
    pub fn load() -> Self {
        std::fs::read_to_string(data_dir().join("settings.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn downloads(&self) -> PathBuf {
        self.download_dir.clone().unwrap_or_else(default_download_dir)
    }

    pub fn save(&self) {
        let dir = data_dir();
        let _ = std::fs::create_dir_all(&dir);
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(dir.join("settings.json"), json);
        }
    }
}

pub fn save_page(name: &str, html: &str) -> std::io::Result<PathBuf> {
    let dir = data_dir().join("pages");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{}.html", name.to_lowercase()));
    std::fs::write(&path, html)?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn picked_location_gets_its_own_folder() {
        assert_eq!(station_folder(Path::new("D:/Uni")), Path::new("D:/Uni").join("GUC Station"));
        assert_eq!(station_folder(Path::new("D:/Uni/guc station")), Path::new("D:/Uni/guc station"));
        let old: Settings = serde_json::from_str(r#"{"theme":"Dark"}"#).unwrap();
        assert!(old.download_dir.is_none(), "settings from before this option still load");
    }
}
