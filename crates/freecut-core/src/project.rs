//! Локальные файлы проекта, настройки и восстановление.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize, de::DeserializeOwned};

use crate::{Document, SCHEMA_VERSION, new_id};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default)]
pub struct ExportSettings {
    pub height: u32,
    pub fps: u32,
    pub quality: u8,
    pub hevc: bool,
    pub bitrate_kbps: Option<u32>,
}

impl Default for ExportSettings {
    fn default() -> Self {
        Self {
            height: 1080,
            fps: 60,
            quality: 1,
            hevc: false,
            bitrate_kbps: None,
        }
    }
}

#[derive(Default, Deserialize, Serialize)]
#[serde(default)]
pub struct Settings {
    pub recents: Vec<PathBuf>,
    pub export_dir: Option<PathBuf>,
    pub export: ExportSettings,
}

#[derive(Deserialize, Serialize)]
pub struct Recovery {
    pub project_path: Option<PathBuf>,
    pub document: Document,
}

pub fn data_dir() -> PathBuf {
    #[cfg(windows)]
    let root = std::env::var_os("LOCALAPPDATA");
    #[cfg(not(windows))]
    let root = std::env::var_os("XDG_CONFIG_HOME").or_else(|| {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config").into_os_string())
    });
    root.map_or_else(std::env::temp_dir, PathBuf::from)
        .join("FlowCut")
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> io::Result<T> {
    serde_json::from_slice(&fs::read(path)?).map_err(io::Error::other)
}

pub fn write_json(path: &Path, value: &impl Serialize) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(".flowcut-{}.tmp", new_id()));
    let result = (|| {
        let mut file = fs::File::options()
            .create_new(true)
            .write(true)
            .open(&temp)?;
        serde_json::to_writer_pretty(&mut file, value)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

pub fn open(path: &Path) -> io::Result<Document> {
    let mut doc: Document = read_json(path)?;
    if let Some(parent) = path.parent() {
        for asset in &mut doc.assets {
            if asset.path.is_relative() {
                asset.path = parent.join(&asset.path);
            }
        }
    }
    validate(&doc)?;
    Ok(doc)
}

pub fn validate(doc: &Document) -> io::Result<()> {
    if doc.schema_version != SCHEMA_VERSION {
        return Err(io::Error::other(format!(
            "Версия проекта {} не поддерживается",
            doc.schema_version
        )));
    }
    if doc
        .timeline
        .tracks
        .iter()
        .flat_map(|t| &t.clips)
        .any(|clip| {
            let Some(asset) = doc.asset(clip.asset_id) else {
                return true;
            };
            !clip.track_start.is_finite()
                || clip.track_start < 0.0
                || !clip.source_in.is_finite()
                || clip.source_in < 0.0
                || !clip.source_duration.is_finite()
                || clip.source_duration <= 0.0
                || !asset.duration.is_finite()
                || clip.source_in + clip.source_duration > asset.duration + 1e-6
                || !(0.25..=4.0).contains(&clip.speed)
                || !clip.end().is_finite()
        })
    {
        return Err(io::Error::other(
            "Проект содержит клип с неверным диапазоном или отсутствующим ассетом",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Asset, AssetKind};

    #[test]
    fn save_replace_and_open_resolves_relative_paths() -> io::Result<()> {
        let folder = std::env::temp_dir().join(format!("flowcut-test-{}", new_id()));
        let path = folder.join("Тест.flowcut");
        let mut doc = Document::default();
        let mut asset = Asset::new(AssetKind::Video, "video.mp4".into());
        asset.duration = 3.0;
        let id = asset.id;
        doc.assets.push(asset);
        if let Some(clip) = doc.make_clip(id) {
            doc.timeline.tracks[0].clips.push(clip);
        }
        write_json(&path, &doc)?;
        doc.name = "Сохранено повторно".into();
        write_json(&path, &doc)?;
        let read = open(&path)?;
        assert_eq!(read.name, doc.name);
        assert_eq!(read.assets[0].path, folder.join("video.mp4"));
        assert_eq!(read.timeline.end_time(), 3.0);
        fs::remove_file(path)?;
        fs::remove_dir(folder)?;
        Ok(())
    }
}
