//! Интеграционный smoke-тест: probe + snapshot на реальном сэмпле,
//! сгенерированном самим ffmpeg. Пропускается, если ffmpeg нет в PATH.

#![allow(clippy::unwrap_used, clippy::expect_used)] // тесты вправе паниковать

use std::path::PathBuf;
use std::process::Command;

use freecut_media::decoder::FrameStream;
use freecut_media::probe::{find_tool, probe};
use freecut_media::snapshot::snapshot_png;

struct Sample(PathBuf);

impl Drop for Sample {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.0.join("sample.mp4"));
        let _ = std::fs::remove_dir(&self.0);
    }
}

fn make_sample() -> Option<Sample> {
    let ffmpeg = find_tool("ffmpeg")?;
    find_tool("ffprobe")?;
    let dir = std::env::temp_dir().join(format!("flowcut-{}", freecut_core::new_id()));
    std::fs::create_dir_all(&dir).expect("временная папка теста");
    let sample = Sample(dir);
    let result = Command::new(ffmpeg)
        .args([
            "-y",
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=640x360:rate=25",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-t",
            "2",
            "-c:v",
            "mpeg4",
            "-pix_fmt",
            "yuv420p",
            "-c:a",
            "aac",
        ])
        .arg(sample.0.join("sample.mp4"))
        .output()
        .expect("запуск FFmpeg");
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    Some(sample)
}
#[test]
fn probe_and_snapshot_on_generated_sample() {
    let Some(sample) = make_sample() else {
        eprintln!("ffmpeg не найден — тест пропущен");
        return;
    };

    let info = match probe(&sample.0.join("sample.mp4")) {
        Ok(info) => info,
        Err(e) => panic!("probe сгенерированного файла: {e}"),
    };
    assert!(
        (info.duration - 2.0).abs() < 0.2,
        "длительность ~2 с, получено {}",
        info.duration
    );
    assert_eq!(info.width, 640);
    assert_eq!(info.height, 360);
    assert!(
        (info.fps - 25.0).abs() < 1.0,
        "fps ~25, получено {}",
        info.fps
    );
    assert!(info.has_video);
    assert!(info.has_audio);

    let png = match snapshot_png(&sample.0.join("sample.mp4"), 1.0) {
        Ok(png) => png,
        Err(e) => panic!("кадр на 1 с: {e}"),
    };
    assert!(png.len() > 1000, "PNG слишком мал: {} байт", png.len());
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n", "stdout не PNG");

    // Несуществующий файл → Err
    let missing = std::env::temp_dir().join("freecut_no_such_file_42.mp4");
    assert!(probe(&missing).is_err());
}

#[test]
fn frame_stream_decodes_scaled_frames() {
    let Some(sample) = make_sample() else {
        eprintln!("ffmpeg не найден — тест пропущен");
        return;
    };
    let stream = match FrameStream::start(&sample.0.join("sample.mp4"), 0.0, 320, 180, 25.0, 1.0) {
        Ok(s) => s,
        Err(e) => panic!("старт потока: {e}"),
    };
    // Ждём первый кадр до ~2.5 с (декодер должен успеть всегда).
    let mut frame = None;
    for _ in 0..50 {
        if let Some(f) = stream.take_frame() {
            frame = Some(f);
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let frame = frame.expect("первый кадр за ~0.5 с");
    assert_eq!(frame.rgba.len(), 320 * 180 * 4, "кадр RGBA 320×180");
    assert!(frame.rgba.iter().any(|&b| b > 16), "кадр не пустой");

    // Спустя ~0.5 с декодер успел ещё кадры (реалтайм-пейcинг).
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert!(stream.frames_decoded() >= 2);

    // Дроп убивает ffmpeg — просто не виснем.
    drop(stream);
}

#[test]
fn frame_stream_rejects_zero_size() {
    let Some(sample) = make_sample() else {
        eprintln!("ffmpeg не найден — тест пропущен");
        return;
    };
    assert!(FrameStream::start(&sample.0.join("sample.mp4"), 0.0, 0, 180, 25.0, 1.0).is_err());
}

#[test]
fn renders_trimmed_clips_and_audio_fades() {
    use freecut_core::{Asset, AssetKind, Document, Fade, project::ExportSettings};
    use std::sync::atomic::AtomicBool;
    let Some(sample) = make_sample() else {
        return;
    };
    let mut doc = Document {
        name: "Монтаж".into(),
        ..Default::default()
    };
    let path = sample.0.join("sample.mp4");
    let mut asset = Asset::new(AssetKind::Video, path);
    asset.duration = 2.0;
    asset.width = 640;
    asset.height = 360;
    asset.has_audio = true;
    let id = asset.id;
    doc.assets.push(asset);
    let mut clip = doc.make_clip(id).unwrap();
    clip.source_in = 0.2;
    clip.source_duration = 1.0;
    clip.audio_fade = Fade::with(0.4);
    clip.audio_fade_out = Fade::with(0.4);
    clip.video_fade = Fade::with(0.4);
    clip.video_fade_out = Fade::with(0.4);
    clip.filters.saturation = 0.0;
    let mut right = clip.clone();
    right.id = freecut_core::new_id();
    right.track_start = 1.0;
    right.speed = 2.0;
    doc.timeline.tracks[0].clips = vec![clip, right];
    let settings = ExportSettings::default();
    let cancel = AtomicBool::new(false);
    let path = freecut_media::export::render_named(
        &doc,
        &sample.0,
        "Моё видео",
        settings,
        &cancel,
        |_| {},
    )
    .expect("экспорт с настоящим FFmpeg")
    .expect("экспорт не отменён");
    assert_eq!(path.file_name().unwrap(), "Моё видео.mp4");
    let info = probe(&path).expect("метаданные MP4");
    assert_eq!((info.width, info.height, info.fps), (1920, 1080, 60.0));
    assert!(info.has_audio);
    assert!((info.duration - 1.5).abs() < 0.05);
    let ffmpeg = find_tool("ffmpeg").unwrap();
    let decode = |time: &str, format: &str, filter: &[&str]| {
        let result = Command::new(&ffmpeg)
            .args(["-v", "error", "-ss", time, "-i"])
            .arg(&path)
            .args(filter)
            .args(["-f", format, "pipe:1"])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        result.stdout
    };
    let light = |time| {
        let result = decode(
            time,
            "rawvideo",
            &[
                "-an",
                "-frames:v",
                "1",
                "-vf",
                "scale=32:18",
                "-pix_fmt",
                "gray",
            ],
        );
        assert_eq!(result.len(), 32 * 18);
        result.into_iter().map(u32::from).sum::<u32>()
    };
    let rms = |time| {
        let bytes = decode(
            time,
            "f32le",
            &["-vn", "-t", "0.04", "-ac", "1", "-c:a", "pcm_f32le"],
        );
        assert!(!bytes.is_empty());
        let sum = bytes
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b).powi(2))
            .sum::<f32>();
        (sum / (bytes.len() / 4) as f32).sqrt()
    };
    assert!(light("0.50") > light("0.00") * 3);
    assert!(light("0.50") > light("0.96") * 3);
    assert!(rms("0.50") > rms("0.00") * 3.0);
    assert!(rms("0.50") > rms("0.94") * 3.0);
    let second = freecut_media::export::render_named(
        &doc,
        &sample.0,
        "Моё видео",
        settings,
        &cancel,
        |_| {},
    )
    .unwrap()
    .unwrap();
    assert_eq!(second.file_name().unwrap(), "Моё видео_1.mp4");
    cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    assert!(
        freecut_media::export::render(&doc, &sample.0, settings, &cancel, |_| {})
            .unwrap()
            .is_none()
    );
    std::fs::remove_file(path).unwrap();
    std::fs::remove_file(second).unwrap();
}

#[test]
fn renders_a_dissolve_between_still_images() {
    use freecut_core::{Asset, AssetKind, Document, Transition, project::ExportSettings};
    use std::sync::atomic::AtomicBool;
    let Some(sample) = make_sample() else {
        return;
    };
    let ffmpeg = find_tool("ffmpeg").unwrap();
    let mut doc = Document::default();
    for (i, color) in ["red", "blue"].into_iter().enumerate() {
        let path = sample.0.join(format!("{color}.png"));
        let result = Command::new(&ffmpeg)
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                &format!("color={color}:s=160x90"),
                "-frames:v",
                "1",
                "-update",
                "1",
            ])
            .arg(&path)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let mut asset = Asset::new(AssetKind::Image, path);
        asset.duration = 1.0;
        let id = asset.id;
        doc.assets.push(asset);
        let mut clip = doc.make_clip(id).unwrap();
        if i == 0 {
            clip.transition_out = Some(Transition::default());
        } else {
            clip.track_start = 0.5;
        }
        doc.timeline.tracks[0].clips.push(clip);
    }
    let settings = ExportSettings {
        height: 720,
        fps: 30,
        ..Default::default()
    };
    let result =
        freecut_media::export::render(&doc, &sample.0, settings, &AtomicBool::new(false), |_| {})
            .unwrap()
            .unwrap();
    let pixel = |time| {
        let output = Command::new(&ffmpeg)
            .args(["-v", "error", "-ss", time, "-i"])
            .arg(&result)
            .args([
                "-an",
                "-frames:v",
                "1",
                "-vf",
                "scale=2:2",
                "-pix_fmt",
                "rgb24",
                "-f",
                "rawvideo",
                "pipe:1",
            ])
            .output()
            .unwrap();
        assert!(output.status.success());
        (output.stdout[0], output.stdout[2])
    };
    let (before_red, before_blue) = pixel("0.30");
    let (middle_red, middle_blue) = pixel("0.75");
    let (after_red, after_blue) = pixel("1.20");
    assert!(before_red > 200 && before_blue < 20);
    assert!(
        middle_red > 60 && middle_blue > 60,
        "{middle_red}, {middle_blue}"
    );
    assert!(after_blue > 200 && after_red < 20);
    std::fs::remove_file(result).unwrap();
    for asset in doc.assets {
        std::fs::remove_file(asset.path).unwrap();
    }
}
