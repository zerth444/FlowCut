//! Фоновый экспорт таймлайна в MP4 через локальный FFmpeg.

use std::fs;
use std::io::{BufRead, BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use freecut_core::{AssetKind, Document, Fade, TrackKind, new_id, project::ExportSettings};

use crate::{
    Error, Result, color_filter,
    probe::{find_tool, hide_console},
};

pub fn tempo_chain(speed: f64) -> Option<Vec<f64>> {
    let mut rate = speed.clamp(0.25, 4.0);
    if (rate - 1.0).abs() < 1e-6 {
        return None;
    }
    let mut stages = Vec::new();
    while rate < 0.5 {
        stages.push(0.5);
        rate *= 2.0;
    }
    while rate > 2.0 {
        stages.push(2.0);
        rate /= 2.0;
    }
    stages.push(rate);
    Some(stages)
}

pub fn render(
    doc: &Document,
    dir: &Path,
    settings: ExportSettings,
    cancel: &AtomicBool,
    progress: impl Fn(f64) + Sync,
) -> Result<Option<PathBuf>> {
    render_named(doc, dir, &doc.display_name(), settings, cancel, progress)
}

pub fn render_named(
    doc: &Document,
    dir: &Path,
    name: &str,
    settings: ExportSettings,
    cancel: &AtomicBool,
    progress: impl Fn(f64) + Sync,
) -> Result<Option<PathBuf>> {
    freecut_core::project::validate(doc)?;
    let duration = doc.timeline.end_time();
    if duration <= 0.0 {
        return Err(Error::Message(
            "Добавьте клипы на таймлайн перед экспортом".into(),
        ));
    }
    let ffmpeg = find_tool("ffmpeg").ok_or(Error::ToolchainMissing)?;
    let fps = settings.fps;
    let height = settings.height;
    if ![24, 25, 30, 50, 60].contains(&fps) || ![720, 1080, 1440, 2160].contains(&height) {
        return Err(Error::Message("Неверные настройки экспорта".into()));
    }
    let width = height * 16 / 9;
    let id = new_id();
    let script = dir.join(format!(".flowcut-{id}.txt"));
    let output = dir.join(format!(".flowcut-{id}.mp4"));
    let result = (|| {
        let (inputs, graph) = render_graph(doc, width, height, fps)?;
        fs::write(&script, graph)?;
        let hardware: &[&str] = if cfg!(target_os = "macos") {
            if settings.hevc {
                &["hevc_videotoolbox"]
            } else {
                &["h264_videotoolbox"]
            }
        } else if cfg!(windows) {
            if settings.hevc {
                &["hevc_nvenc", "hevc_qsv", "hevc_amf"]
            } else {
                &["h264_nvenc", "h264_qsv", "h264_amf"]
            }
        } else {
            &[]
        };
        let software = if settings.hevc { "libx265" } else { "libx264" };
        for codec in hardware.iter().copied().chain([software]) {
            if cancel.load(Ordering::Relaxed) {
                return Ok(None);
            }
            progress(0.0);
            let mut command = Command::new(&ffmpeg);
            command.args(["-y", "-nostdin", "-hide_banner", "-loglevel", "error"]);
            for (path, offset, length) in &inputs {
                command.args(["-protocol_whitelist", "file", "-ss", &seconds(*offset)]);
                if let Some(length) = length {
                    command.args(["-t", &seconds(*length)]);
                }
                command.arg("-i").arg(path);
            }
            command.arg("-/filter_complex").arg(&script).args([
                "-map",
                "[video]",
                "-map",
                "[audio]",
                "-t",
                &seconds(duration),
                "-c:v",
                codec,
                "-pix_fmt",
                "yuv420p",
                "-c:a",
                "aac",
                "-b:a",
                "192k",
                "-movflags",
                "+faststart",
                "-progress",
                "pipe:1",
            ]);
            if settings.hevc {
                command.args(["-tag:v", "hvc1"]);
            }
            if let Some(kbps) = settings.bitrate_kbps {
                command.args(["-b:v", &format!("{}k", kbps.clamp(500, 100_000))]);
            } else if codec == software {
                let crf = ["28", "21", "17"][usize::from(settings.quality.min(2))];
                command.args(["-preset", "fast", "-crf", crf]);
            } else {
                let base = [4_u64, 8, 16][usize::from(settings.quality.min(2))];
                let kbps =
                    (base * 1000 * u64::from(width) * u64::from(height) / (1920 * 1080)).max(500);
                command.args(["-b:v", &format!("{kbps}k")]);
            }
            command
                .arg(&output)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            hide_console(&mut command);
            match run(&mut command, duration, cancel, &progress) {
                Ok(true) if cancel.load(Ordering::Relaxed) => return Ok(None),
                Ok(true) => {
                    return publish(&output, dir, name).map(Some).map_err(Error::from);
                }
                Ok(false) => return Ok(None),
                Err(error) if codec != software => {
                    tracing::debug!(codec, %error, "кодировщик недоступен")
                }
                Err(error) => return Err(error),
            }
        }
        Err(Error::Message("Нет доступного кодировщика".into()))
    })();
    let _ = fs::remove_file(script);
    let _ = fs::remove_file(output);
    result
}

type Input = (PathBuf, f64, Option<f64>);

fn render_graph(doc: &Document, width: u32, height: u32, fps: u32) -> Result<(Vec<Input>, String)> {
    use std::fmt::Write;
    let mut inputs = Vec::new();
    let mut graph = format!("color=c=black:s={width}x{height}:r={fps}[base];\n");
    let mut video = "base".to_owned();
    let mut audio = String::new();
    let mut audio_count = 0;
    for track in doc.timeline.tracks.iter().rev() {
        for (index, clip) in track.clips.iter().enumerate() {
            let Some(asset) = doc.asset(clip.asset_id) else {
                continue;
            };
            let has_video = track.kind == TrackKind::Video && asset.kind != AssetKind::Audio;
            let has_audio = asset.has_audio
                && !clip.separate_audio
                && !track.muted
                && !clip.muted
                && clip.volume > 0.0;
            if !has_video && !has_audio {
                continue;
            }
            if !asset.path.is_file() {
                return Err(Error::Message(format!(
                    "Файл источника отсутствует: {}",
                    asset.path.display()
                )));
            }
            let idx = inputs.len();
            let image = asset.kind == AssetKind::Image;
            inputs.push((
                asset.path.clone(),
                if image { 0.0 } else { clip.source_in },
                (!image).then_some(clip.source_duration),
            ));
            let len = clip.duration();
            let start = clip.track_start;
            if has_video {
                let _ = write!(graph, "[{idx}:V:0]setpts=(PTS-STARTPTS)/{},", clip.speed);
                if !clip.filters.is_default() {
                    let _ = write!(graph, "{},", color_filter(clip.filters));
                }
                let _ = write!(
                    graph,
                    "scale={width}:{height}:force_original_aspect_ratio=decrease:force_divisible_by=2,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2,setsar=1,format=yuv420p,"
                );
                // Клонирование последнего кадра сохраняет длительность стоп-кадров и VFR.
                let _ = write!(
                    graph,
                    "tpad=stop_mode=clone:stop_duration={},fps={fps},trim=duration={},",
                    seconds(if image { len } else { 2.0 / f64::from(fps) }),
                    seconds(len)
                );
                push_fades(
                    &mut graph,
                    "fade",
                    clip.video_fade,
                    clip.video_fade_out,
                    len,
                );
                let overlap = track.overlap(index);
                if overlap > 0.0 {
                    let _ = write!(
                        graph,
                        "format=yuva420p,fade=t=in:st=0:d={}:alpha=1,",
                        seconds(overlap)
                    );
                }
                let _ = writeln!(graph, "setpts=PTS-STARTPTS+{}/TB[v{idx}];", seconds(start));
                let _ = writeln!(
                    graph,
                    "[{video}][v{idx}]overlay=eof_action=pass:repeatlast=0:enable='gte(t,{})*lt(t,{})'[o{idx}];",
                    seconds(start),
                    seconds(start + len)
                );
                video = format!("o{idx}");
            }
            if has_audio {
                let _ = write!(graph, "[{idx}:a:0]asetpts=PTS-STARTPTS,");
                if let Some(stages) = tempo_chain(clip.speed) {
                    for rate in stages {
                        let _ = write!(graph, "atempo={rate},");
                    }
                }
                let _ = write!(
                    graph,
                    "aformat=sample_rates=48000:channel_layouts=stereo,apad,atrim=duration={},volume={},",
                    seconds(len),
                    clip.volume
                );
                push_fades(
                    &mut graph,
                    "afade",
                    clip.audio_fade,
                    clip.audio_fade_out,
                    len,
                );
                let delay = (start * 48_000.0).round() as u64;
                let _ = writeln!(graph, "adelay={delay}S:all=1[a{idx}];");
                let _ = write!(audio, "[a{idx}]");
                audio_count += 1;
            }
        }
    }
    let _ = writeln!(graph, "[{video}]null[video];");
    if audio_count == 0 {
        graph.push_str("anullsrc=r=48000:cl=stereo[audio]");
    } else {
        let _ = write!(
            graph,
            "{audio}amix=inputs={audio_count}:normalize=0,apad[audio]"
        );
    }
    Ok((inputs, graph))
}

fn seconds(value: f64) -> String {
    format!("{value:.8}")
}

fn push_fades(graph: &mut String, name: &str, input: Fade, output: Fade, len: f64) {
    use std::fmt::Write;
    for (kind, fade) in [("in", input), ("out", output)] {
        let span = fade.effective_duration(len);
        if span > 0.0 {
            let start = if kind == "in" { 0.0 } else { len - span };
            let _ = write!(
                graph,
                "{name}=t={kind}:st={}:d={},",
                seconds(start),
                seconds(span)
            );
        }
    }
}

fn run(
    command: &mut Command,
    duration: f64,
    cancel: &AtomicBool,
    progress: &(impl Fn(f64) + Sync),
) -> Result<bool> {
    let mut child = command.spawn()?;
    std::thread::scope(|scope| {
        let stdout = child.stdout.take();
        scope.spawn(move || {
            if let Some(stdout) = stdout {
                for line in BufReader::new(stdout)
                    .lines()
                    .map_while(std::result::Result::ok)
                {
                    if let Some(us) = line
                        .strip_prefix("out_time_us=")
                        .and_then(|value| value.parse::<f64>().ok())
                    {
                        progress((us / 1_000_000.0 / duration).clamp(0.0, 1.0));
                    }
                }
            }
        });
        let stderr = child.stderr.take();
        let error = scope.spawn(move || {
            let mut log = Vec::new();
            if let Some(mut stderr) = stderr {
                let _ = stderr.read_to_end(&mut log);
            }
            log
        });
        loop {
            if cancel.load(Ordering::Relaxed) {
                let _ = child.kill();
                let _ = child.wait();
                return Ok(false);
            }
            match child.try_wait() {
                Ok(Some(status)) if status.success() => return Ok(true),
                Ok(Some(status)) => {
                    let log = error.join().unwrap_or_default();
                    return Err(Error::Message(format!(
                        "FFmpeg ({status}): {}",
                        String::from_utf8_lossy(&log).trim()
                    )));
                }
                Ok(None) => std::thread::sleep(Duration::from_millis(40)),
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(error.into());
                }
            }
        }
    })
}

fn publish(source: &Path, dir: &Path, name: &str) -> std::io::Result<PathBuf> {
    let safe: String = name
        .chars()
        .take(100)
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    let safe = safe.trim_matches([' ', '.']);
    let stem = if safe.is_empty() { "FlowCut" } else { safe };
    for n in 0.. {
        let suffix = if n == 0 {
            String::new()
        } else {
            format!("_{n}")
        };
        let path = dir.join(format!("{stem}{suffix}.mp4"));
        match fs::hard_link(source, &path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => {
                let mut target = match fs::File::options().write(true).create_new(true).open(&path)
                {
                    Ok(file) => file,
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => return Err(error),
                };
                let result = fs::File::open(source)
                    .and_then(|mut from| std::io::copy(&mut from, &mut target));
                drop(target);
                if let Err(error) = result {
                    let _ = fs::remove_file(path);
                    return Err(error);
                }
                return Ok(path);
            }
        }
    }
    unreachable!()
}
