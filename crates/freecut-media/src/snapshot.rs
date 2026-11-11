//! Извлечение одиночных кадров через ffmpeg (subprocess, PNG в stdout).

use std::path::Path;
use std::process::Command;

use crate::probe::{find_tool, hide_console};
use crate::{Error, Result};

/// Ограничение высоты снимаемого кадра: превью и миниатюры ужимаются,
/// чтобы текстура не занимала сотни мегабайт на 4K-источниках.
const MAX_SNAPSHOT_HEIGHT: u32 = 540;

/// Снять один кадр с момента `t` (сек) и вернуть PNG-байты в stdout.
///
/// # Errors
/// [`Error::ToolchainMissing`], если ffmpeg не найден,
/// [`Error::Unsupported`] для несуществующих файлов,
/// [`Error::Parse`], если ffmpeg не смог декодировать кадр.
pub fn snapshot_png(path: &Path, t: f64) -> Result<Vec<u8>> {
    snapshot_filtered(path, t, freecut_core::Filters::default())
}

pub fn snapshot_filtered(path: &Path, t: f64, filters: freecut_core::Filters) -> Result<Vec<u8>> {
    let Some(ffmpeg) = find_tool("ffmpeg") else {
        return Err(Error::ToolchainMissing);
    };
    if !path.is_file() {
        return Err(Error::Unsupported {
            path: path.to_string_lossy().into_owned(),
        });
    }

    let t = t.max(0.0);
    let mut filter = format!(
        "scale=w='min(iw,960)':h='min(ih,{MAX_SNAPSHOT_HEIGHT})':force_original_aspect_ratio=decrease"
    );
    if !filters.is_default() {
        filter = format!("{},{filter}", crate::color_filter(filters));
    }
    let mut command = Command::new(&ffmpeg);
    command
        .args([
            "-v", // без спама в stderr
            "error",
            "-nostdin",
            "-protocol_whitelist",
            "file",
            "-ss", // быстрый поиск ключевого кадра: точность не критична
            &format!("{t:.6}"),
            "-i",
        ])
        .arg(path)
        .args([
            "-map",
            "0:V:0",
            "-frames:v",
            "1",
            "-vf",
            filter.as_str(),
            "-f", // image2pipe: единственный мультиплексор, умеющий stdout
            "image2pipe",
            "-c:v",
            "png",
            "pipe:1",
        ]);
    hide_console(&mut command);
    let output = command.output()?;

    if !output.status.success() || output.stdout.is_empty() {
        return Err(Error::Parse(format!(
            "ffmpeg не вернул кадр: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output.stdout)
}
