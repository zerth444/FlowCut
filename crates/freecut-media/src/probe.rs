//! Сбор метаданных файла через ffprobe.

use std::path::Path;
use std::process::Command;

use crate::{Error, Result};

/// Метаданные источника, заполняемые ffprobe.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MediaInfo {
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub has_video: bool,
    pub has_audio: bool,
}

/// Скрыть консольное окно подпроцесса на Windows (`CREATE_NO_WINDOW`):
/// ffmpeg/ffprobe — консольные программы, и без этого флага каждое их
/// появление открывает терминал поверх окна редактора.
pub fn hide_console(command: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        let _ = command;
    }
}

/// Определить тип файла по расширению (быстрая эвристика до реального probe).
#[must_use]
pub fn guess_kind(path: &Path) -> freecut_core::AssetKind {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("mp3" | "wav" | "ogg" | "m4a" | "aac" | "flac" | "opus" | "wma") => {
            freecut_core::AssetKind::Audio
        }
        Some("png" | "jpg" | "jpeg" | "bmp" | "gif" | "webp" | "tiff") => {
            freecut_core::AssetKind::Image
        }
        _ => freecut_core::AssetKind::Video,
    }
}

/// Зондирование файла через ffprobe (subprocess, без FFI).
///
/// Для изображений ffprobe успешно отдаёт размеры; длительность считается
/// «ручной» на стороне импорта (см. [`MediaInfo::duration`]).
///
/// # Errors
/// Возвращает [`Error::ToolchainMissing`], если ffprobe не найден,
/// [`Error::Unsupported`] для несуществующих файлов и [`Error::Parse`],
/// если ffprobe не смог прочитать файл.
pub fn probe(path: &Path) -> Result<MediaInfo> {
    let Some(ffprobe) = find_tool("ffprobe") else {
        return Err(Error::ToolchainMissing);
    };
    if !path.is_file() {
        return Err(Error::Unsupported {
            path: path.to_string_lossy().into_owned(),
        });
    }

    let mut ffprobe_cmd = Command::new(&ffprobe);
    ffprobe_cmd
        .args([
            "-protocol_whitelist",
            "file",
            "-v", // тихо, ошибки только в stderr
            "error",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
        ])
        .arg(path);
    hide_console(&mut ffprobe_cmd);
    let output = ffprobe_cmd.output()?;
    if !output.status.success() {
        return Err(Error::Parse(format!(
            "ffprobe: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| Error::Parse(format!("ffprobe вернул не-JSON: {e}")))?;
    parse_probe_json(&json, path)
}

fn parse_probe_json(json: &serde_json::Value, path: &Path) -> Result<MediaInfo> {
    let streams = json
        .get("streams")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| Error::Parse("ffprobe не вернул список потоков".to_owned()))?;

    let mut info = MediaInfo::default();
    for stream in streams {
        match stream.get("codec_type").and_then(serde_json::Value::as_str) {
            Some("video")
                if stream
                    .pointer("/disposition/attached_pic")
                    .and_then(serde_json::Value::as_u64)
                    != Some(1) =>
            {
                info.has_video = true;
                if info.width == 0 {
                    info.width = stream
                        .get("width")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0) as u32;
                    info.height = stream
                        .get("height")
                        .and_then(serde_json::Value::as_u64)
                        .unwrap_or(0) as u32;
                    let rotation = stream
                        .get("side_data_list")
                        .and_then(serde_json::Value::as_array)
                        .and_then(|data| {
                            data.iter()
                                .find_map(|entry| entry.get("rotation")?.as_i64())
                        });
                    if rotation.is_some_and(|angle| angle.rem_euclid(180) == 90) {
                        std::mem::swap(&mut info.width, &mut info.height);
                    }
                }
                if info.fps == 0.0 {
                    info.fps = ["avg_frame_rate", "r_frame_rate"]
                        .into_iter()
                        .find_map(|field| stream.get(field)?.as_str().and_then(parse_rate))
                        .unwrap_or(0.0);
                }
            }
            Some("audio") => info.has_audio = true,
            _ => {}
        }
    }

    if !info.has_video && !info.has_audio {
        return Err(Error::Unsupported {
            path: path.to_string_lossy().into_owned(),
        });
    }

    // Длительность: сначала format, иначе максимальная по потокам.
    info.duration = json
        .pointer("/format/duration")
        .and_then(serde_json::Value::as_str)
        .and_then(|d| d.parse().ok())
        .filter(|d: &f64| d.is_finite() && *d > 0.0)
        .unwrap_or_else(|| {
            streams
                .iter()
                .filter_map(|s| s.get("duration").and_then(serde_json::Value::as_str))
                .filter_map(|d| d.parse::<f64>().ok())
                .filter(|d| d.is_finite())
                .fold(0.0_f64, f64::max)
        });

    Ok(info)
}

/// Разобрать дробь вида «30000/1001» в частоту кадров.
fn parse_rate(text: &str) -> Option<f64> {
    let (num, den) = text.split_once('/')?;
    let num: f64 = num.trim().parse().ok()?;
    let den: f64 = den.trim().parse().ok()?;
    let rate = num / den;
    (den > 0.0 && rate.is_finite() && rate > 0.0).then_some(rate)
}

/// Найти локальный инструмент рядом с редактором, в настроенной папке или PATH.
#[must_use]
pub fn find_tool(name: &str) -> Option<std::path::PathBuf> {
    let ext = if cfg!(windows) { ".exe" } else { "" };

    let mut exe = name.to_owned();
    if !exe.ends_with(ext) {
        exe.push_str(ext);
    }

    let configured = std::env::var_os("FLOWCUT_FFMPEG_DIR").map(std::path::PathBuf::from);
    let adjacent = std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf));
    let paths = std::env::var_os("PATH").unwrap_or_default();
    #[cfg(windows)]
    let shotcut =
        std::env::var_os("ProgramFiles").map(|root| std::path::PathBuf::from(root).join("Shotcut"));
    #[cfg(not(windows))]
    let shotcut = None;
    configured
        .into_iter()
        .chain(adjacent)
        .chain(std::env::split_paths(&paths))
        .chain(shotcut)
        .map(|dir| dir.join(&exe))
        .find(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guesses_audio_by_extension() {
        assert_eq!(
            guess_kind(Path::new("music/track.MP3")),
            freecut_core::AssetKind::Audio
        );
        assert_eq!(
            guess_kind(Path::new("cover.png")),
            freecut_core::AssetKind::Image
        );
        assert_eq!(
            guess_kind(Path::new("clip.mp4")),
            freecut_core::AssetKind::Video
        );
    }
}
