//! Воспроизведение звука: один cpal-поток вывода, микс активных источников.
//!
//! Источник — один клип с аудио: фоновый ffmpeg перекодирует его диапазон
//! в PCM f32 (частота/каналы зажаты под устройство вывода) в кольцевой буфер,
//! callback потока вывода смешивает все активные источники с их громкостью.

use std::collections::VecDeque;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use freecut_core::{Fade, Id, fade_gain};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::{Error, Result};

/// Сколько сэмплов кольцевого буфера считать «готовыми к началу воспроизведения»
/// (~40 мс @48 кГц стерео): убирает щелчок на старте.
const PRIME_FRAMES: usize = 4096;

/// Желаемый источник аудио — один на клип с аудиодорожкой.
#[derive(Debug, Clone)]
pub struct SpawnSource {
    pub clip_id: Id,
    pub path: PathBuf,
    /// Начало внутри источника, сек.
    pub start_s: f64,
    /// Сколько контентных секунд источника декодировать (до конца клипа), сек.
    pub duration_s: f64,
    pub volume: f32,
    /// Скорость 0.25..=4.0 (цепочка atempo).
    pub speed: f64,
    pub local: f64,
    pub clip_duration: f64,
    pub fade_in: Fade,
    pub fade_out: Fade,
}

/// Общее состояние миксера (callback ↔ UI-поток).
struct AudioState {
    sources: Mutex<Vec<Arc<Source>>>,
    channels: usize,
}

/// Один источник: кольцевой буфер, читающий ffmpeg-процесс, громкость.
struct Source {
    clip_id: Id,
    ring: Arc<Mutex<VecDeque<f32>>>,
    ready: Arc<AtomicBool>,
    done: Arc<AtomicBool>,
    envelope: Mutex<SpawnSource>,
    played_frames: AtomicU64,
    start_local: f64,
    sample_rate: u32,
    /// Параметры запуска; скачок позиции/скорости → пересоздание.
    spawn_start: f64,
    spawn_speed: f64,
    /// Момент старта источника (для расчёта «естественной» позиции).
    spawned_at: Instant,
    child: Mutex<Child>,
    stop: Arc<AtomicBool>,
}

impl Drop for Source {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Ok(mut child) = self.child.lock() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Плеер: cpal-поток на устройстве вывода по умолчанию (f32).
pub struct AudioPlayer {
    state: Arc<AudioState>,
    sample_rate: u32,
    channels: usize,
    /// Стрим жив, пока жив этот объект.
    _stream: cpal::Stream,
}

impl AudioPlayer {
    /// Открыть устройство вывода по умолчанию (f32, частота устройства).
    ///
    /// # Errors
    /// [`Error::DeviceUnavailable`], если аудиоустройство недоступно.
    pub fn open() -> Result<Self> {
        let host = cpal::default_host();
        let Some(device) = host.default_output_device() else {
            return Err(Error::DeviceUnavailable("аудиовыход не найден".to_owned()));
        };
        let supported = device.default_output_config().map_err(Error::from_device)?;
        let config: cpal::StreamConfig = supported.config();
        let channels = usize::from(config.channels);
        let sample_rate = config.sample_rate.0;

        let state = Arc::new(AudioState {
            sources: Mutex::new(Vec::new()),
            channels,
        });
        let stream = match supported.sample_format() {
            cpal::SampleFormat::F32 => build_stream::<f32>(&device, &config, &state),
            cpal::SampleFormat::F64 => build_stream::<f64>(&device, &config, &state),
            cpal::SampleFormat::I8 => build_stream::<i8>(&device, &config, &state),
            cpal::SampleFormat::I16 => build_stream::<i16>(&device, &config, &state),
            cpal::SampleFormat::I32 => build_stream::<i32>(&device, &config, &state),
            cpal::SampleFormat::I64 => build_stream::<i64>(&device, &config, &state),
            cpal::SampleFormat::U8 => build_stream::<u8>(&device, &config, &state),
            cpal::SampleFormat::U16 => build_stream::<u16>(&device, &config, &state),
            cpal::SampleFormat::U32 => build_stream::<u32>(&device, &config, &state),
            cpal::SampleFormat::U64 => build_stream::<u64>(&device, &config, &state),
            other => {
                return Err(Error::DeviceUnavailable(format!(
                    "формат вывода {other:?} не поддерживается"
                )));
            }
        }?;
        stream.play().map_err(Error::from_device)?;

        Ok(Self {
            state,
            sample_rate,
            channels,
            _stream: stream,
        })
    }

    /// Частота устройства вывода, Гц.
    #[must_use]
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Привести источники к желаемому набору: добавить новые, убрать лишние,
    /// обновить громкость живьём; пересоздать только при реальном скробе
    /// (прыжке позиции) или смене скорости — обычное воспроизведение не
    /// пересоздаёт источники (иначе они бы пересоздавались каждые ~0.3 с):
    pub fn ensure_sources(&self, desired: &[SpawnSource]) {
        let Ok(current) = self.state.sources.lock().map(|sources| sources.clone()) else {
            return;
        };
        let mut updated = Vec::with_capacity(desired.len());
        for request in desired {
            if let Some(existing) = current.iter().find(|source| {
                let expected = source.spawn_start
                    + source.spawned_at.elapsed().as_secs_f64() * source.spawn_speed;
                source.clip_id == request.clip_id
                    && (source.spawn_speed - request.speed).abs() <= 1e-6
                    && (expected - request.start_s).abs() <= 0.25 * request.speed
            }) {
                if let Ok(mut envelope) = existing.envelope.lock() {
                    envelope.volume = request.volume;
                    envelope.fade_in = request.fade_in;
                    envelope.fade_out = request.fade_out;
                    envelope.clip_duration = request.clip_duration;
                }
                updated.push(Arc::clone(existing));
                continue;
            }
            match spawn_source(request, self.sample_rate, self.channels) {
                Ok(source) => updated.push(source),
                Err(error) => tracing::warn!(%error, "не удалось запустить аудио клипа"),
            }
        }
        if let Ok(mut sources) = self.state.sources.lock() {
            *sources = updated;
        }
    }

    /// Убрать все источники. Подпроцессы завершаются вне блокировки микшера.
    pub fn clear(&self) {
        let old = self
            .state
            .sources
            .lock()
            .ok()
            .map(|mut sources| std::mem::take(&mut *sources));
        drop(old);
    }
}

fn build_stream<T: cpal::SizedSample + cpal::FromSample<f32>>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    state: &Arc<AudioState>,
) -> Result<cpal::Stream> {
    let state = Arc::clone(state);
    let mut buffer = Vec::new();
    device
        .build_output_stream(
            config,
            move |out: &mut [T], _| {
                buffer.resize(out.len(), 0.0_f32);
                mix_chunk(&mut buffer, &state);
                for (sample, mixed) in out.iter_mut().zip(&buffer) {
                    *sample = T::from_sample(*mixed);
                }
            },
            |err| tracing::warn!("ошибка аудиопотока: {err}"),
            None,
        )
        .map_err(Error::from_device)
}
/// Смешать активные источники в буфер вывода (f32, интерливинг).
fn mix_chunk(data: &mut [f32], state: &AudioState) {
    data.fill(0.0);
    let Ok(sources) = state.sources.lock() else {
        return;
    };
    let channels = state.channels.max(1);
    let frames = data.len() / channels;
    for src in sources.iter() {
        if !src.ready.load(Ordering::Relaxed) {
            continue; // буфер ещё подводится
        }
        let Ok(envelope) = src.envelope.lock() else {
            continue;
        };
        let mut offset = src.played_frames.load(Ordering::Relaxed);
        let played;
        {
            let mut ring = match src.ring.lock() {
                Ok(r) => r,
                Err(_) => continue,
            };
            let expected =
                (src.spawned_at.elapsed().as_secs_f64() * f64::from(src.sample_rate)) as u64;
            let lag = expected.saturating_sub(offset);
            if lag > u64::from(src.sample_rate / 25) {
                let skip = (lag as usize).min(ring.len() / channels);
                ring.drain(..skip * channels);
                offset += skip as u64;
            }
            let take = (ring.len() / channels).min(frames);
            for frame in 0..take {
                let local =
                    src.start_local + (offset + frame as u64) as f64 / f64::from(src.sample_rate);
                let volume = envelope.volume
                    * fade_gain(
                        envelope.fade_in,
                        envelope.fade_out,
                        envelope.clip_duration,
                        local,
                    ) as f32;
                for ch in 0..channels {
                    let Some(sample) = ring.pop_front() else {
                        break;
                    };
                    data[frame * channels + ch] += sample * volume;
                }
            }
            played = take;
        }
        src.played_frames
            .store(offset + played as u64, Ordering::Relaxed);
        if played < frames && src.done.load(Ordering::Relaxed) {
            // Всё выдано и закончилось — источник больше не нужен.
            src.stop.store(true, Ordering::Relaxed);
        }
    }
    for sample in data.iter_mut() {
        *sample = sample.clamp(-1.0, 1.0);
    }
}

/// Запустить ffmpeg для клипа и получить живой источник.
fn spawn_source(desired: &SpawnSource, sample_rate: u32, channels: usize) -> Result<Arc<Source>> {
    if desired.duration_s <= 0.01 {
        return Err(Error::Message("нулевая длительность аудио".to_owned()));
    }
    let Some(ffmpeg) = freecut_media::probe::find_tool("ffmpeg") else {
        return Err(Error::Message("ffmpeg не найден в PATH".to_owned()));
    };

    let tempo_arg = tempo_chain(desired.speed).map(|stages| {
        stages
            .iter()
            .map(|s| format!("atempo={s}"))
            .collect::<Vec<_>>()
            .join(",")
    });

    let started = Instant::now();
    let mut command = Command::new(ffmpeg);
    command
        .args([
            "-v", // тихо: только ошибки
            "error",
            "-nostdin",
            "-protocol_whitelist",
            "file",
            "-ss",
            &format!("{:.6}", desired.start_s.max(0.0)),
            "-t",
            &format!("{:.6}", desired.duration_s),
            "-i",
        ])
        .arg(&desired.path)
        .args([
            "-map",
            "0:a:0",
            "-t",
            &format!("{:.6}", desired.duration_s / desired.speed),
        ])
        .args([
            "-f", // сырые сэмплы f32
            "f32le",
            "-acodec",
            "pcm_f32le",
            "-ar",
            &sample_rate.to_string(),
            "-ac",
            &channels.to_string(),
        ]);
    if let Some(filter) = tempo_arg {
        command.args(["-af", filter.as_str()]);
    }
    command
        .arg("pipe:1")
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    freecut_media::probe::hide_console(&mut command);

    let mut child: Child = command.spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Message("ffmpeg не открыл stdout".to_owned()))?;

    let ring = Arc::new(Mutex::new(VecDeque::new()));
    let ready = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));

    let reader_ring = Arc::clone(&ring);
    let reader_ready = Arc::clone(&ready);
    let reader_done = Arc::clone(&done);
    let reader_stop = Arc::clone(&stop);
    let chunk_bytes = channels * 1024 * 4; // 1024 фреймов
    std::thread::Builder::new()
        .name("flowcut_audio_decode".to_owned())
        .spawn(move || {
            let mut stdout = stdout;
            let mut buffer = vec![0_u8; chunk_bytes];
            loop {
                if reader_stop.load(Ordering::Relaxed) {
                    break;
                }
                if reader_ring
                    .lock()
                    .is_ok_and(|ring| ring.len() >= sample_rate as usize * channels / 2)
                {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                }
                let mut len = 0;
                while len < buffer.len() {
                    match stdout.read(&mut buffer[len..]) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => len += read,
                    }
                }
                if let Ok(mut ring) = reader_ring.lock() {
                    let aligned = len - len % (channels * 4);
                    for chunk in buffer[..aligned].as_chunks::<4>().0 {
                        ring.push_back(f32::from_le_bytes([
                            chunk[0], chunk[1], chunk[2], chunk[3],
                        ]));
                    }
                }
                let buffered = reader_ring.lock().map(|r| r.len()).unwrap_or(PRIME_FRAMES);
                if !reader_ready.load(Ordering::Relaxed) && buffered >= PRIME_FRAMES {
                    reader_ready.store(true, Ordering::Relaxed);
                }
                if len < buffer.len() {
                    break;
                }
            }
            reader_ready.store(true, Ordering::Relaxed);
            reader_done.store(true, Ordering::Relaxed);
        })?;

    Ok(Arc::new(Source {
        clip_id: desired.clip_id,
        ring,
        ready,
        done,
        envelope: Mutex::new(desired.clone()),
        played_frames: AtomicU64::new(0),
        start_local: desired.local,
        sample_rate,
        spawn_start: desired.start_s,
        spawn_speed: desired.speed,
        spawned_at: started,
        child: Mutex::new(child),
        stop,
    }))
}

pub use freecut_media::export::tempo_chain;

#[cfg(test)]
#[allow(clippy::expect_used)] // тесты вправе паниковать
mod tests {
    #[test]
    fn tempo_chain_products() {
        use super::tempo_chain;
        assert!(tempo_chain(1.0).is_none());
        assert_eq!(tempo_chain(0.5).as_deref(), Some(&[0.5][..]));
        assert_eq!(tempo_chain(2.0).as_deref(), Some(&[2.0][..]));
        let stages = tempo_chain(0.25).expect("цепочка");
        assert_eq!(stages, vec![0.5, 0.5]);
        let stages = tempo_chain(4.0).expect("цепочка");
        assert_eq!(stages, vec![2.0, 2.0]);
        for speed in [0.25_f64, 0.5, 0.7, 1.0, 1.5, 2.0, 3.0, 4.0] {
            if let Some(stages) = tempo_chain(speed) {
                let product: f64 = stages.iter().product();
                assert!(
                    (product - speed).abs() < 1e-9,
                    "{speed}: произведение тапов {product}"
                );
                for s in stages {
                    assert!((0.5..=2.0).contains(&s), "тап {s} вне диапазона atempo");
                }
            }
        }
    }
}
