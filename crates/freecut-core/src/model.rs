//! Модель проекта FreeCut: ассеты, дорожки, клипы, эффекты.
//!
//! Формат `.freecut` — это JSON-сериализация [`Document`] c полем
//! `schema_version`, позволяющим проводить миграции при изменении модели.
//! Все новые поля добавляются с `#[serde(default)]`, чтобы старые проекты
//! открывались в новых версиях приложения.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

/// Версия схемы проекта. Увеличивается при несовместимых изменениях модели.
pub const SCHEMA_VERSION: u32 = 1;

/// Идентификатор сущности в проекте.
pub type Id = Uuid;

/// Сгенерировать новый идентификатор.
#[must_use]
pub fn new_id() -> Id {
    Uuid::new_v4()
}

/// Тип импортированного файла-источника.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    /// Видео (может содержать аудиопоток).
    #[default]
    Video,
    /// Чисто аудиофайл (mp3, wav, ...).
    Audio,
    /// Статичное изображение (длительность задаётся вручную).
    Image,
}

/// Файл-источник, импортированный в медиатеку.
///
/// `path` хранится как есть: при относительном пути он трактуется
/// относительно файла проекта.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Asset {
    pub id: Id,
    pub kind: AssetKind,
    pub path: PathBuf,
    pub display_name: String,
    /// Длительность в секундах (0.0 у изображений).
    pub duration: f64,
    /// Исходное разрешение (0 для аудио).
    pub width: u32,
    pub height: u32,
    /// Частота кадров источника (0.0 если неизвестна).
    pub fps: f64,
    pub has_audio: bool,
}

impl Asset {
    #[must_use]
    pub fn new(kind: AssetKind, path: PathBuf) -> Self {
        let display_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned());
        Self {
            id: new_id(),
            kind,
            path,
            display_name,
            duration: 0.0,
            width: 0,
            height: 0,
            fps: 0.0,
            has_audio: false,
        }
    }

    /// Источник помечен как доступный (файл найден при открытии проекта).
    #[must_use]
    pub fn is_online(&self) -> bool {
        if self.path.is_absolute() {
            self.path.exists()
        } else {
            true // относительные пути проверяются относительно каталога проекта
        }
    }
}

/// Вид дорожки.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TrackKind {
    /// Видеодорожка (клипы могут нести аудио своих источников).
    #[default]
    Video,
    /// Аудиодорожка.
    Audio,
}

/// Дорожка таймлайна: последовательность клипов без зазоров (магнитный режим)
/// либо с фиксированными позициями.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub id: Id,
    pub name: String,
    pub kind: TrackKind,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub video_sound: bool,
    #[serde(default)]
    pub clips: Vec<Clip>,
}

impl Track {
    #[must_use]
    pub fn new(name: impl Into<String>, kind: TrackKind) -> Self {
        Self {
            id: new_id(),
            name: name.into(),
            kind,
            muted: false,
            locked: false,
            video_sound: false,
            clips: Vec::new(),
        }
    }

    /// Конец последнего клипа на дорожке (0.0, если пусто).
    #[must_use]
    pub fn end_time(&self) -> f64 {
        self.clips.iter().map(Clip::end).fold(0.0_f64, f64::max)
    }

    pub fn overlap(&self, incoming: usize) -> f64 {
        let Some(previous) = incoming.checked_sub(1).and_then(|i| self.clips.get(i)) else {
            return 0.0;
        };
        let Some(next) = self.clips.get(incoming) else {
            return 0.0;
        };
        transition_duration(previous, next).min((previous.end() - next.track_start).max(0.0))
    }
}

pub fn transition_duration(left: &Clip, right: &Clip) -> f64 {
    left.transition_out.map_or(0.0, |transition| {
        transition
            .duration
            .clamp(0.0, left.duration().min(right.duration()) * 0.5)
    })
}

/// Затухание/появление (fade). `duration` в секундах, не может превышать
/// длительность клипа — это ограничение проверяется в момент применения.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Fade {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_fade_duration")]
    pub duration: f64,
}

impl Default for Fade {
    fn default() -> Self {
        Self::off()
    }
}

fn default_fade_duration() -> f64 {
    0.5
}

impl Fade {
    #[must_use]
    pub fn off() -> Self {
        Self {
            enabled: false,
            duration: 0.5,
        }
    }

    #[must_use]
    pub fn with(duration: f64) -> Self {
        Self {
            enabled: true,
            duration: duration.clamp(0.0, 60.0),
        }
    }

    /// Эффективная длительность: не больше 0 и не больше указанного лимита.
    #[must_use]
    pub fn effective_duration(&self, clip_duration: f64) -> f64 {
        if !self.enabled {
            return 0.0;
        }
        self.duration.clamp(0.0, clip_duration)
    }

    /// Прогресс 0..=1 для момента `t_local` относительно начала клипа (in).
    #[must_use]
    pub fn progress_at(&self, t_local: f64, clip_duration: f64) -> f64 {
        let d = self.effective_duration(clip_duration);
        if d <= 0.0 || t_local <= 0.0 {
            return 0.0;
        }
        (t_local / d).clamp(0.0, 1.0)
    }
}

/// Базовые цветокоррекционные фильтры. Применяются и в превью, и при экспорте.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Filters {
    /// -1.0 (тёмнее) ..= +1.0 (светлее). 0.0 = без изменений.
    #[serde(default)]
    pub brightness: f64,
    /// 0.0 (чёрный) ..= 2.0. 1.0 = без изменений.
    #[serde(default = "default_speed")]
    pub contrast: f64,
    /// 0.0 (ч/б) ..= 2.0. 1.0 = без изменений.
    #[serde(default = "default_speed")]
    pub saturation: f64,
}

impl Default for Filters {
    fn default() -> Self {
        Self {
            brightness: 0.0,
            contrast: 1.0,
            saturation: 1.0,
        }
    }
}

impl Filters {
    #[must_use]
    pub fn is_default(&self) -> bool {
        self == &Self::default()
    }
}

/// Тип перехода на стыке клипов (расширяемый перечислением вариантов).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TransitionKind {
    /// Кросс-диссолв: уходящий клип растворяется в приходящем.
    #[default]
    CrossDissolve,
}

/// Переход, применяемый на выходе клипа (стык с последующим клипом).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Transition {
    pub kind: TransitionKind,
    /// Длительность перехода в секундах; переезжает внутрь обоих клипов.
    #[serde(default = "default_fade_duration")]
    pub duration: f64,
}

impl Default for Transition {
    fn default() -> Self {
        Self {
            kind: TransitionKind::default(),
            duration: 0.5,
        }
    }
}

/// Клип: использование ассета (или его части) на дорожке.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Clip {
    pub id: Id,
    /// Идентификатор [`Asset`] из медиатеки.
    pub asset_id: Id,
    #[serde(default)]
    pub separate_audio: bool,
    #[serde(default)]
    pub audio_detached: bool,
    #[serde(default)]
    pub audio_parent: Option<Id>,
    /// Позиция на дорожке, секунды.
    pub track_start: f64,
    /// Входная точка внутри источника, секунды.
    pub source_in: f64,
    /// Длительность содержимого источника, секунды (без учёта скорости).
    pub source_duration: f64,
    /// Громкость клипа, 0.0 ..= 2.0 (0..200%).
    #[serde(default = "default_speed")]
    pub volume: f64,
    #[serde(default)]
    pub muted: bool,
    /// Скорость: 1.0 = норм, 0.5 = вдвое медленнее.
    #[serde(default = "default_speed")]
    pub speed: f64,
    #[serde(default)]
    pub video_fade: Fade,
    #[serde(default)]
    pub video_fade_out: Fade,
    #[serde(default)]
    pub audio_fade: Fade,
    #[serde(default)]
    pub audio_fade_out: Fade,
    #[serde(default)]
    pub filters: Filters,
    /// Переход на выходе клипа (к следующему клипу на дорожке).
    #[serde(default)]
    pub transition_out: Option<Transition>,
}

fn default_speed() -> f64 {
    1.0
}

impl Clip {
    #[must_use]
    pub fn new(asset_id: Id) -> Self {
        Self {
            id: new_id(),
            asset_id,
            separate_audio: false,
            audio_detached: false,
            audio_parent: None,
            track_start: 0.0,
            source_in: 0.0,
            source_duration: 0.0,
            volume: 1.0,
            muted: false,
            speed: 1.0,
            video_fade: Fade::off(),
            video_fade_out: Fade::off(),
            audio_fade: Fade::off(),
            audio_fade_out: Fade::off(),
            filters: Filters::default(),
            transition_out: None,
        }
    }

    /// Занимаемая на дорожке длительность с учётом скорости.
    #[must_use]
    pub fn duration(&self) -> f64 {
        self.elementary_duration() / self.speed.max(0.01)
    }

    #[must_use]
    pub fn end(&self) -> f64 {
        self.track_start + self.duration()
    }

    pub fn video_gain(&self, time: f64) -> f64 {
        let local = time - self.track_start;
        fade_gain(self.video_fade, self.video_fade_out, self.duration(), local)
    }

    fn elementary_duration(&self) -> f64 {
        self.source_duration.max(0.0)
    }
}

pub fn fade_gain(input: Fade, output: Fade, duration: f64, local: f64) -> f64 {
    let ramp = |fade: Fade, position: f64| {
        let span = fade.effective_duration(duration);
        if span > 0.0 {
            (position / span).clamp(0.0, 1.0)
        } else {
            1.0
        }
    };
    ramp(input, local) * ramp(output, duration - local)
}

/// Таймлайн проекта: дорожки и режим магнита.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Timeline {
    #[serde(default)]
    pub magnetic: bool,
    #[serde(default = "default_tracks")]
    pub tracks: Vec<Track>,
}

fn default_tracks() -> Vec<Track> {
    vec![
        Track::new("Видео", TrackKind::Video),
        Track::new("Аудио", TrackKind::Audio),
    ]
}

impl Default for Timeline {
    fn default() -> Self {
        Self {
            magnetic: false,
            tracks: default_tracks(),
        }
    }
}

impl Timeline {
    /// Конец таймлайна (конец последнего клипа на любой дорожке).
    #[must_use]
    pub fn end_time(&self) -> f64 {
        self.tracks
            .iter()
            .map(Track::end_time)
            .fold(0.0_f64, f64::max)
    }

    pub fn video_tracks(&self) -> impl Iterator<Item = &Track> {
        self.tracks.iter().filter(|t| t.kind == TrackKind::Video)
    }

    pub fn audio_tracks(&self) -> impl Iterator<Item = &Track> {
        self.tracks.iter().filter(|t| t.kind == TrackKind::Audio)
    }

    #[must_use]
    pub fn find_clip(&self, clip_id: Id) -> Option<&Clip> {
        self.tracks
            .iter()
            .flat_map(|t| t.clips.iter())
            .find(|c| c.id == clip_id)
    }
}

/// Документ проекта: то, что сохраняется в `.freecut`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub schema_version: u32,
    /// Имя проекта (по умолчанию «Безымянный проект»).
    pub name: String,
    #[serde(default)]
    pub assets: Vec<Asset>,
    #[serde(default)]
    pub timeline: Timeline,
}

impl Default for Document {
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            name: String::new(),
            assets: Vec::new(),
            timeline: Timeline::default(),
        }
    }
}

impl Document {
    /// Новый пустой документ.
    #[must_use]
    pub fn new_empty() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn display_name(&self) -> String {
        if self.name.is_empty() {
            "Безымянный проект".to_owned()
        } else {
            self.name.clone()
        }
    }

    #[must_use]
    pub fn asset(&self, id: Id) -> Option<&Asset> {
        self.assets.iter().find(|a| a.id == id)
    }

    /// Создать новый клип из ассета, обрезанный до его длительности.
    #[must_use]
    pub fn make_clip(&self, asset_id: Id) -> Option<Clip> {
        let asset = self.asset(asset_id)?;
        let mut clip = Clip::new(asset_id);
        clip.source_duration = asset.duration.max(0.0);
        Some(clip)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)] // тесты вправе паниковать

    use super::*;

    #[test]
    fn clip_end_accounts_for_speed() {
        let mut c = Clip::new(new_id());
        c.track_start = 10.0;
        c.source_duration = 4.0;
        assert!((c.end() - 14.0).abs() < 1e-9);

        c.speed = 2.0;
        assert!((c.end() - 12.0).abs() < 1e-9, "при x2 длительность = 2с");
    }

    #[test]
    fn fade_progress_saturates() {
        let f = Fade::with(1.0);
        assert!((f.progress_at(0.5, 10.0) - 0.5).abs() < 1e-9);
        assert_eq!(f.progress_at(5.0, 10.0), 1.0); // в середине клипа уже 100%
        assert_eq!(Fade::off().progress_at(1.0, 10.0), 0.0);
    }

    #[test]
    fn fade_cannot_exceed_clip() {
        let f = Fade::with(5.0);
        assert!((f.effective_duration(2.0) - 2.0).abs() < 1e-9);
    }

    #[test]
    fn timeline_end_is_max_of_tracks() {
        let mut doc = Document::new_empty();
        let asset = Asset::new(AssetKind::Video, "demo.mp4".into());
        doc.assets.push(asset.clone());
        let mut clip = doc.make_clip(asset.id).expect("клип");
        clip.track_start = 7.0;
        clip.source_duration = 3.0;
        doc.timeline.tracks[0].clips.push(clip);
        assert!((doc.timeline.end_time() - 10.0).abs() < 1e-9);
    }

    #[test]
    fn document_serializes_roundtrip() {
        let doc = Document::new_empty();
        let json = serde_json::to_string(&doc).expect("сериализация");
        let back: Document = serde_json::from_str(&json).expect("десериализация");
        assert_eq!(back.schema_version, SCHEMA_VERSION);
        assert!(!back.timeline.magnetic);
        assert_eq!(back.timeline.tracks.len(), 2);
    }
}
