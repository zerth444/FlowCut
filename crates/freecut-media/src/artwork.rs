//! Миниатюры и огибающие громкости с локальным дисковым кэшем.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;

use freecut_core::{Asset, new_id, project};

use crate::{
    Error, Result,
    probe::{find_tool, hide_console},
};

pub fn thumbnail(asset: &Asset) -> Result<Vec<u8>> {
    cached(&asset.path, "thumb", || {
        crate::snapshot::snapshot_png(&asset.path, 0.0)
    })
}

pub fn waveform(asset: &Asset) -> Result<Vec<u8>> {
    cached(&asset.path, "wave", || {
        let ffmpeg = find_tool("ffmpeg").ok_or(Error::ToolchainMissing)?;
        let mut command = Command::new(ffmpeg);
        command
            .args([
                "-nostdin",
                "-v",
                "error",
                "-protocol_whitelist",
                "file",
                "-i",
            ])
            .arg(&asset.path)
            .args([
                "-filter_complex",
                "[0:a:0]aresample=400,showwavespic=s=640x64:colors=0x65DCC8:scale=sqrt[wave]",
                "-map",
                "[wave]",
                "-frames:v",
                "1",
                "-f",
                "image2pipe",
                "-c:v",
                "png",
                "pipe:1",
            ]);
        hide_console(&mut command);
        let out = command.output()?;
        if !out.status.success() || out.stdout.is_empty() {
            return Err(Error::Message(
                String::from_utf8_lossy(&out.stderr).trim().to_owned(),
            ));
        }
        Ok(out.stdout)
    })
}

fn cached(path: &Path, tag: &str, generate: impl FnOnce() -> Result<Vec<u8>>) -> Result<Vec<u8>> {
    let key = cache_key(path, tag);
    if let Some(key) = &key
        && let Ok(bytes) = std::fs::read(key)
        && bytes.starts_with(b"\x89PNG\r\n\x1a\n")
    {
        return Ok(bytes);
    }
    let bytes = generate()?;
    if let Some(key) = key
        && let Some(dir) = key.parent()
        && std::fs::create_dir_all(dir).is_ok()
    {
        let temp = dir.join(format!("{}.tmp", new_id()));
        if std::fs::write(&temp, &bytes).is_ok() {
            let _ = std::fs::rename(&temp, key);
        }
        let _ = std::fs::remove_file(temp);
    }
    Ok(bytes)
}

fn cache_key(path: &Path, tag: &str) -> Option<PathBuf> {
    let meta = std::fs::metadata(path).ok()?;
    let mut hash = DefaultHasher::new();
    path.hash(&mut hash);
    meta.len().hash(&mut hash);
    meta.modified().ok()?.hash(&mut hash);
    Some(
        project::data_dir()
            .join("cache")
            .join(format!("{tag}-{:016x}.png", hash.finish())),
    )
}
