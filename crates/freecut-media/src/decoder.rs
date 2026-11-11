//! Потоковое декодирование видео (ffmpeg → rawvideo RGBA в фоне).

use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::{
    Error, Result,
    probe::{find_tool, hide_console},
};

/// Декодированный кадр в RGBA (непремультиплированный, как у ffmpeg).
pub struct Frame {
    /// Порядковый номер кадра с момента запуска потока (0, 1, 2...).
    pub index: u64,
    pub rgba: Vec<u8>,
}

/// Живой поток кадров из одного файла.
///
/// Поток paced'ится под `fps`: кадры читаются реальном времени, так что
/// `take_frame()` в последний готовый кадр ≈ «что должно быть на экране сейчас».
/// Дроп вызывает kill процесса ffmpeg; поток чтения завершается сам по EOF.
pub struct FrameStream {
    child: Child,
    stop: Arc<AtomicBool>,
    slot: Arc<Mutex<Option<Frame>>>,
    frames_decoded: Arc<AtomicU64>,
    /// Размер кадров (немулируемый: 0 запрещён при старте).
    pub width: u32,
    pub height: u32,
}

impl FrameStream {
    /// Начать декодирование с момента `start_t` (сек), кадры масштабируются
    /// до `width`×`height` (RGBA). Пейcинг — реальное время по `fps` и `speed`.
    ///
    /// # Errors
    /// [`Error::ToolchainMissing`], если ffmpeg не найден;
    /// [`Error::Unsupported`] для несуществующих файлов;
    /// [`Error::Parse`] при ошибке запуска/нулевом размере.
    pub fn start(
        path: &Path,
        start_t: f64,
        width: u32,
        height: u32,
        fps: f64,
        speed: f64,
    ) -> Result<Self> {
        Self::start_filtered(
            path,
            start_t,
            [width, height],
            fps,
            speed,
            freecut_core::Filters::default(),
        )
    }

    pub fn start_filtered(
        path: &Path,
        start_t: f64,
        [width, height]: [u32; 2],
        fps: f64,
        speed: f64,
        filters: freecut_core::Filters,
    ) -> Result<Self> {
        let Some(ffmpeg) = find_tool("ffmpeg") else {
            return Err(Error::ToolchainMissing);
        };
        if !path.is_file() {
            return Err(Error::Unsupported {
                path: path.to_string_lossy().into_owned(),
            });
        }
        if width == 0 || height == 0 {
            return Err(Error::Parse("нулевой размер кадра".to_owned()));
        }

        let mut filter = format!("fps={fps},scale={width}:{height}");
        if !filters.is_default() {
            filter = format!("{},{filter}", crate::color_filter(filters));
        }
        let mut child_cmd = Command::new(&ffmpeg);
        child_cmd
            .args([
                "-v",
                "error",
                "-nostdin",
                "-protocol_whitelist",
                "file",
                "-ss",
                &format!("{start_t:.6}"),
                "-i",
            ])
            .arg(path)
            .args([
                "-map", "0:V:0",
                "-f", // сырые кадры: fastest path, без контейнера
                "rawvideo", "-pix_fmt", "rgba", "-vf", &filter, "pipe:1",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        hide_console(&mut child_cmd);
        let mut child = child_cmd.spawn()?;

        let mut stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Parse("ffmpeg не открыл stdout".to_owned()))?;

        let stop = Arc::new(AtomicBool::new(false));
        let slot: Arc<Mutex<Option<Frame>>> = Arc::new(Mutex::new(None));
        let frames_decoded = Arc::new(AtomicU64::new(0));

        let reader_stop = Arc::clone(&stop);
        let reader_slot = Arc::clone(&slot);
        let reader_count = Arc::clone(&frames_decoded);
        let frame_bytes = usize::try_from(width)
            .ok()
            .and_then(|w| w.checked_mul(usize::try_from(height).unwrap_or(0)))
            .and_then(|area| area.checked_mul(4))
            .ok_or_else(|| Error::Parse("слишком большой кадр".to_owned()))?;
        let frame_dt = Duration::from_secs_f64(1.0 / fps.max(1.0) / speed.max(0.01));

        std::thread::Builder::new()
            .name("flowcut_video_decode".to_owned())
            .spawn(move || {
                let started = Instant::now();
                let mut index: u64 = 0;
                loop {
                    if reader_stop.load(Ordering::Relaxed) {
                        break;
                    }
                    let mut buf = vec![0_u8; frame_bytes];
                    if stdout.read_exact(&mut buf).is_err() {
                        break; // EOF или процесс убит
                    }
                    if let Ok(mut g) = reader_slot.lock() {
                        *g = Some(Frame { index, rgba: buf });
                    }
                    index += 1;
                    reader_count.store(index, Ordering::Relaxed);
                    // Пейcинг: кадр N должен быть прочитан к (N+1) * frame_dt.
                    let deadline = started + frame_dt.mul_f64(index as f64);
                    let now = Instant::now();
                    if deadline > now {
                        std::thread::sleep(deadline - now);
                    }
                }
            })?;

        Ok(Self {
            child,
            stop,
            slot,
            frames_decoded,
            width,
            height,
        })
    }

    /// Забрать самый свежий готовый кадр (если появился новый).
    #[must_use]
    pub fn take_frame(&self) -> Option<Frame> {
        self.slot.lock().ok()?.take()
    }

    /// Сколько кадров декодировано с момента старта.
    #[must_use]
    pub fn frames_decoded(&self) -> u64 {
        self.frames_decoded.load(Ordering::Relaxed)
    }
}

impl Drop for FrameStream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
