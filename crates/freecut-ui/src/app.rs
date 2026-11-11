//! Корневое состояние приложения и разбиение окна на панели.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Instant;

use egui::{Context, TextureHandle, Ui};

use freecut_core::{Asset, AssetKind, Clip, Document, Id, Timeline, Track, TrackKind};

use crate::{
    project_ui::{ProjectAction, ProjectUi},
    theme, widgets,
};

/// Payload drag&drop ассета из медиатеки на дорожку.
#[derive(Debug, Clone, Copy)]
pub struct AssetDragPayload {
    pub asset_id: Id,
    pub is_audio: bool,
}

/// Длительность статичного изображения после импорта, сек.
const IMAGE_DEFAULT_DURATION: f64 = 5.0;
/// Максимальная высота кадров декодирования/превью (экономия памяти).
const PREVIEW_MAX_HEIGHT: u32 = 540;

#[derive(Clone, Copy, PartialEq)]
struct FrameKey {
    asset: Id,
    time: f64,
    filters: freecut_core::Filters,
}

type ImportResult = Result<(Asset, Option<egui::ColorImage>), String>;

enum ImportEvent {
    Asset(ImportResult),
    Waveform(Id, egui::ColorImage),
}

enum Art {
    Thumbnail(Id, egui::ColorImage),
    Waveform(Id, egui::ColorImage),
}

/// Состояние плеера превью: живой поток кадров или статичный seek.
#[derive(Default)]
struct VideoPlayer {
    /// Активный поток декодирования (жив при воспроизведении).
    stream: Option<freecut_media::decoder::FrameStream>,
    /// Сессия потока: (asset, стартовый source_t, fps, speed).
    session: Option<(Id, f64, f64, f64, freecut_core::Filters)>,
    /// Момент последнего перезапуска потока (троттлинг спавна ffmpeg).
    last_restart: Option<Instant>,
    /// Фоновой seek занят (не больше одного ffmpeg-процесса за раз).
    seek: Option<(FrameKey, Receiver<freecut_media::Result<Vec<u8>>>)>,
    /// Какой момент времени отражает текущий статичный кадр.
    static_frame: Option<FrameKey>,
    texture: Option<TextureHandle>,
    error: Option<String>,
    clip: Option<Id>,
}

#[derive(Default)]
struct Player {
    video: VideoPlayer,
    incoming: VideoPlayer,
    audio: Option<freecut_audio::AudioPlayer>,
    audio_failed: bool,
}

struct VideoFrame {
    clip_id: Id,
    key: FrameKey,
    path: PathBuf,
    size: [u32; 2],
    fps: f64,
    speed: f64,
    still: bool,
}

impl VideoFrame {
    fn at(clip: &Clip, asset: &Asset, time: f64) -> Option<Self> {
        if asset.kind == AssetKind::Audio
            || asset.width == 0
            || asset.height == 0
            || !asset.path.is_file()
        {
            return None;
        }
        let still = asset.kind == AssetKind::Image;
        let fps = if asset.fps > 0.0 {
            asset.fps.clamp(1.0, 120.0)
        } else {
            30.0
        };
        let frame = (clip.source_in + (time - clip.track_start) * clip.speed) * fps;
        let source_end = (clip.source_in + clip.source_duration).min(asset.duration);
        let last_frame = (source_end * fps - 1e-7).ceil() - 1.0;
        let time = if still {
            0.0
        } else {
            (frame + 1e-7).floor().min(last_frame).max(0.0) / fps
        };
        let (w, h) = display_size(asset.width, asset.height);
        Some(Self {
            clip_id: clip.id,
            key: FrameKey {
                asset: asset.id,
                time,
                filters: clip.filters,
            },
            path: asset.path.clone(),
            size: [w, h],
            fps,
            speed: clip.speed,
            still,
        })
    }
}
/// Размер кадров декодирования: укладываемся в PREVIEW_MAX_HEIGHT, чётные стороны.
#[must_use]
fn display_size(width: u32, height: u32) -> (u32, u32) {
    let even = |v: u32| (v - (v % 2)).max(2);
    if height == 0 || width == 0 {
        return (16, 16);
    }
    let scale = (f64::from(PREVIEW_MAX_HEIGHT) / f64::from(height))
        .min(960.0 / f64::from(width))
        .min(1.0);
    (
        even((f64::from(width) * scale).round() as u32),
        even((f64::from(height) * scale).round() as u32),
    )
}

pub(crate) fn plan_clips(clips: &[Clip], dur: f64, t: f64, magnetic: bool) -> (f64, usize) {
    if !magnetic {
        let mut start = if t.is_finite() {
            (t - dur * 0.5).max(0.0)
        } else {
            clips.last().map_or(0.0, Clip::end)
        };
        for (idx, clip) in clips.iter().enumerate() {
            if start + dur <= clip.track_start {
                return (start, idx);
            }
            start = start.max(clip.end());
        }
        return (start, clips.len());
    }
    let idx = clips.partition_point(|c| c.track_start + c.duration() * 0.5 <= t);
    let start = if idx == 0 { 0.0 } else { clips[idx - 1].end() };
    (start, idx)
}

fn snap_center(
    clips: &[Clip],
    t: f64,
    duration: f64,
    playhead: f64,
    pixels_per_second: f64,
) -> f64 {
    if !t.is_finite() {
        return t;
    }
    let half = duration * 0.5;
    clips
        .iter()
        .flat_map(|clip| [clip.track_start, clip.end()])
        .chain([0.0, playhead])
        .flat_map(|edge| [edge - half, edge + half])
        .filter(|center| *center >= half && (*center - t).abs() <= 8.0 / pixels_per_second)
        .min_by(|a, b| (a - t).abs().total_cmp(&(b - t).abs()))
        .unwrap_or(t)
}

fn pack(track: &mut Track) {
    if track.video_sound {
        return;
    }
    let mut time = 0.0;
    for index in 0..track.clips.len() {
        if index > 0 && track.kind == TrackKind::Video {
            time -= freecut_core::transition_duration(&track.clips[index - 1], &track.clips[index]);
        }
        let clip = &mut track.clips[index];
        clip.track_start = time;
        time = clip.end();
    }
}

/// Вкладки левой панели.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryTab {
    Media,
    Audio,
    Effects,
}

/// Какой край клипа тянут при тримминге.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TrimSide {
    Left,
    Right,
}

/// Активный перетаскиваемый край клипа (тримминг на дорожке).
#[derive(Debug, Clone, Copy)]
pub struct TrimState {
    pub clip_id: Id,
    pub side: TrimSide,
    /// Время таймлайна под указателем в момент захвата края.
    pub anchor_t: f64,
    pub orig_track_start: f64,
    pub orig_source_in: f64,
    pub orig_source_duration: f64,
    pub speed: f64,
    pub min_start: f64,
    pub max_end: f64,
}

/// Всё UI-состояние окна (вне модели проекта).
pub struct UiState {
    /// Модель проекта.
    pub doc: Document,
    /// Есть несохранённые изменения.
    pub dirty: bool,
    /// Воспроизведение.
    pub playing: bool,
    /// Позиция плейхеда, секунды.
    pub playhead: f64,
    /// Масштаб таймлайна, пикселей на секунду.
    pub pps: f64,
    /// Прокрутка таймлайна, секунды от начала.
    pub scroll: f64,
    /// Текущая вкладка медиатеки.
    pub library_tab: LibraryTab,
    /// Выбранный клип (id).
    pub selected_clip: Option<Id>,
    extra_selection: HashSet<Id>,
    pub moving: Option<(Id, f64)>,
    /// Активный тримминг края клипа (зажатый краешек).
    pub trim: Option<TrimState>,
    /// Редактирование имени проекта в верхней панели.
    pub renaming: bool,
    /// Файлы, ожидающие импорта.
    pub pending_import: Vec<std::path::PathBuf>,
    /// Открыть системный диалог импорта в начале следующего кадра (кнопка/Ctrl+I).
    pub want_import: bool,
    /// Миниатюры ассетов (кадр, снятый при импорте), по id ассета.
    pub thumbs: HashMap<Id, TextureHandle>,
    pub waves: HashMap<Id, TextureHandle>,
    /// Текущий кадр превью (живой при воспроизведении, статичный на паузе).
    pub preview_tex: Option<TextureHandle>,
    pub secondary_preview: Option<(TextureHandle, f64, f64)>,
    /// Высота строки видеодорожки (перетягивается мышью).
    pub video_row_h: f32,
    /// Высота строки аудиодорожки (перетягивается мышью).
    pub audio_row_h: f32,
    pub notice: Option<String>,
    pub importing: bool,
    pub(crate) project: ProjectUi,
    undo: Vec<Timeline>,
    redo: Vec<Timeline>,
    trim_before: Option<Timeline>,
    clipboard: Vec<(TrackKind, Clip)>,
}

impl UiState {
    pub fn sync_video_sound(&mut self) {
        let assets = &self.doc.assets;
        let tracks = &mut self.doc.timeline.tracks;
        let linked: Vec<_> = tracks
            .iter_mut()
            .filter(|track| track.kind == TrackKind::Video)
            .flat_map(|track| {
                let muted = track.muted;
                track.clips.iter_mut().filter_map(move |clip| {
                    if clip.audio_detached
                        || !assets
                            .iter()
                            .any(|asset| asset.id == clip.asset_id && asset.has_audio)
                    {
                        return None;
                    }
                    clip.separate_audio = true;
                    Some((clip.clone(), muted))
                })
            })
            .collect();
        let row = match tracks.iter().position(|track| track.video_sound) {
            Some(index) => &mut tracks[index],
            None if linked.is_empty() => return,
            None => {
                let mut track = Track::new("Звук", TrackKind::Audio);
                track.video_sound = true;
                let index = tracks
                    .iter()
                    .position(|track| track.kind == TrackKind::Audio)
                    .unwrap_or(tracks.len());
                tracks.insert(index, track);
                &mut tracks[index]
            }
        };
        let mut old_ids = HashMap::new();
        row.clips.retain(|clip| {
            if let Some(parent) = clip.audio_parent {
                old_ids.insert(parent, clip.id);
                false
            } else {
                true
            }
        });
        for (mut clip, muted) in linked {
            let parent = clip.id;
            clip.id = old_ids
                .get(&parent)
                .copied()
                .unwrap_or_else(freecut_core::new_id);
            clip.audio_parent = Some(parent);
            clip.separate_audio = false;
            clip.muted |= muted;
            clip.transition_out = None;
            row.clips.push(clip);
        }
        row.clips
            .sort_by(|a, b| a.track_start.total_cmp(&b.track_start));
    }

    pub fn detach_audio(&mut self, audio_id: Id) {
        let Some(parent) = self
            .doc
            .timeline
            .find_clip(audio_id)
            .and_then(|clip| clip.audio_parent)
        else {
            return;
        };
        self.record_edit();
        for clip in self
            .doc
            .timeline
            .tracks
            .iter_mut()
            .flat_map(|track| &mut track.clips)
        {
            if clip.id == parent {
                clip.audio_detached = true;
            }
            if clip.id == audio_id {
                clip.audio_parent = None;
            }
        }
    }
    pub(crate) fn new() -> Self {
        Self {
            doc: Document::new_empty(),
            dirty: false,
            playing: false,
            playhead: 0.0,
            pps: 90.0,
            scroll: 0.0,
            library_tab: LibraryTab::Media,
            selected_clip: None,
            extra_selection: HashSet::new(),
            moving: None,
            trim: None,
            renaming: false,
            pending_import: Vec::new(),
            want_import: false,
            thumbs: HashMap::new(),
            waves: HashMap::new(),
            preview_tex: None,
            secondary_preview: None,
            video_row_h: 74.0,
            audio_row_h: 54.0,
            notice: None,
            importing: false,
            project: ProjectUi::default(),
            undo: Vec::new(),
            redo: Vec::new(),
            trim_before: None,
            clipboard: Vec::new(),
        }
    }

    /// Конец таймлайна.
    #[must_use]
    pub fn duration(&self) -> f64 {
        self.doc.timeline.end_time()
    }

    /// Шаг в один кадр (проект 60 fps по умолчанию).
    #[must_use]
    pub fn frame_step(&self) -> f64 {
        1.0 / 60.0
    }

    /// Клип, под которым находится плейхед (для превью).
    #[must_use]
    pub fn clip_under_playhead(&self) -> Option<&Clip> {
        let end = self.duration();
        let time = if (self.playhead - end).abs() < 1e-9 {
            (end - 1e-8).max(0.0)
        } else {
            self.playhead
        };
        self.doc
            .timeline
            .video_tracks()
            .flat_map(|t| t.clips.iter())
            .find(|c| c.track_start <= time && time < c.end())
    }

    fn transition_under_playhead(&self) -> Option<(&Clip, f64)> {
        let current = self.clip_under_playhead()?;
        let (track_index, index) = self.clip_position(current.id)?;
        let track = &self.doc.timeline.tracks[track_index];
        let incoming = track.clips.get(index + 1)?;
        let overlap = track.overlap(index + 1);
        if overlap > 0.0 && self.playhead >= incoming.track_start {
            Some((
                incoming,
                ((self.playhead - incoming.track_start) / overlap).clamp(0.0, 1.0),
            ))
        } else {
            None
        }
    }

    pub fn set_transition(&mut self, id: Id, transition: Option<freecut_core::Transition>) {
        let Some((row, index)) = self.clip_position(id) else {
            return;
        };
        if self.doc.timeline.tracks[row].clips.get(index + 1).is_some()
            && self.doc.timeline.magnetic
        {
            self.update_clip(id, |clip| clip.transition_out = transition);
            self.repack_track(id);
        }
    }

    pub fn project_fade(&mut self, at_end: bool) {
        let clips = self
            .doc
            .timeline
            .video_tracks()
            .flat_map(|track| &track.clips);
        let clip = if at_end {
            clips.max_by(|a, b| a.end().total_cmp(&b.end()))
        } else {
            clips.min_by(|a, b| a.track_start.total_cmp(&b.track_start))
        };
        if let Some(clip) = clip {
            self.update_clip(clip.id, |clip| {
                let fade = if at_end {
                    &mut clip.video_fade_out
                } else {
                    &mut clip.video_fade
                };
                *fade = freecut_core::Fade::with(0.5);
            });
        }
    }

    pub fn has_next_clip(&self, id: Id) -> bool {
        self.clip_position(id).is_some_and(|(row, index)| {
            self.doc.timeline.tracks[row].clips.get(index + 1).is_some()
        })
    }

    pub fn toggle_playback(&mut self) {
        if self.playing {
            self.playing = false;
        } else if self.duration() > 0.0 {
            if self.playhead >= self.duration() {
                self.playhead = 0.0;
            }
            self.playing = true;
        }
    }

    pub fn seek(&mut self, time: f64) {
        self.playing = false;
        self.playhead = time.max(0.0);
    }

    pub(crate) fn replace_document(&mut self, doc: Document) {
        self.project.export_name = None;
        self.doc = doc;
        self.dirty = false;
        self.seek(0.0);
        self.scroll = 0.0;
        self.undo.clear();
        self.redo.clear();
        self.clipboard.clear();
        self.trim = None;
        self.trim_before = None;
        self.clear_selection();
        self.moving = None;
        self.thumbs.clear();
        self.preview_tex = None;
        self.secondary_preview = None;
        self.pending_import.clear();
        self.importing = false;
        self.notice = None;
        self.project.completed = None;
    }

    /// Сплан магнитной вставки ассета на дорожку `track_id` в момент `t`.
    #[must_use]
    pub fn plan_insert(&self, track_id: Id, asset_id: Id, t: f64) -> Option<(f64, usize)> {
        let asset = self.doc.asset(asset_id)?;
        let track = self
            .doc
            .timeline
            .tracks
            .iter()
            .find(|tr| tr.id == track_id)?;
        Some(self.plan_position(&track.clips, asset.duration, t))
    }

    fn plan_position(&self, clips: &[Clip], duration: f64, target: f64) -> (f64, usize) {
        let magnetic = self.doc.timeline.magnetic;
        let target = if magnetic {
            target
        } else {
            snap_center(clips, target, duration, self.playhead, self.pps)
        };
        plan_clips(clips, duration, target, magnetic)
    }

    /// Вставить клип ассета на дорожку в момент `t` (магнитно, со сдвигом
    /// последующих клипов). Возвращает id нового клипа.
    pub fn insert_clip(&mut self, asset_id: Id, track_id: Id, t: f64) -> Option<Id> {
        let clip = self.doc.make_clip(asset_id)?;
        self.insert_existing(clip, track_id, t)
    }

    fn insert_existing(&mut self, clip: Clip, track_id: Id, t: f64) -> Option<Id> {
        if !self
            .doc
            .timeline
            .tracks
            .iter()
            .any(|track| track.id == track_id)
        {
            return None;
        }
        self.record_edit();
        let result = self.place_clip(clip, track_id, t);
        if let Some(id) = result {
            self.select_only(id);
        }
        result
    }

    fn place_clip(&mut self, mut clip: Clip, track_id: Id, t: f64) -> Option<Id> {
        let index = self
            .doc
            .timeline
            .tracks
            .iter()
            .position(|tr| tr.id == track_id)?;
        let magnetic = self.doc.timeline.magnetic;
        let (start, idx) =
            self.plan_position(&self.doc.timeline.tracks[index].clips, clip.duration(), t);
        clip.track_start = start;
        let track = &mut self.doc.timeline.tracks[index];
        let clip_id = clip.id;
        track.clips.insert(idx, clip);
        if magnetic {
            pack(track);
        }
        Some(clip_id)
    }

    /// Добавить ассет в конец первой дорожки нужного типа (двойной клик).
    /// Возвращает id нового клипа.
    pub fn add_asset_to_timeline(&mut self, asset_id: Id) -> Option<Id> {
        let kind = self.doc.asset(asset_id)?.kind;
        let track_kind = if kind == AssetKind::Audio {
            TrackKind::Audio
        } else {
            TrackKind::Video
        };
        // t = INFINITY: планировщик разобрал её как «в самый конец дорожки».
        let track_id = self
            .doc
            .timeline
            .tracks
            .iter()
            .find(|t| t.kind == track_kind && !t.video_sound)
            .map(|t| t.id)?;
        self.insert_clip(asset_id, track_id, f64::INFINITY)
    }

    /// Изменить клип по id (и пометить документ грязным).
    pub fn update_clip(&mut self, clip_id: Id, f: impl FnOnce(&mut Clip)) {
        if self.trim.is_none() && self.doc.timeline.find_clip(clip_id).is_some() {
            self.record_edit();
        }
        if let Some(clip) = self
            .doc
            .timeline
            .tracks
            .iter_mut()
            .flat_map(|track| track.clips.iter_mut())
            .find(|clip| clip.id == clip_id)
        {
            f(clip);
            self.dirty = true;
        }
    }

    /// Уплотнить дорожку с клипом `clip_id` (магнитный режим: без зазоров).
    pub fn repack_track(&mut self, clip_id: Id) {
        if !self.doc.timeline.magnetic {
            return;
        }
        let Some(track) = self
            .doc
            .timeline
            .tracks
            .iter_mut()
            .find(|track| track.clips.iter().any(|clip| clip.id == clip_id))
        else {
            return;
        };
        pack(track);
    }

    fn record_edit(&mut self) {
        self.undo.push(self.doc.timeline.clone());
        self.redo.clear();
        self.dirty = true;
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty() && self.trim.is_none()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty() && self.trim.is_none()
    }

    pub fn undo(&mut self) {
        if self.can_undo()
            && let Some(previous) = self.undo.pop()
        {
            self.redo
                .push(std::mem::replace(&mut self.doc.timeline, previous));
            self.after_history_change();
        }
    }

    pub fn redo(&mut self) {
        if self.can_redo()
            && let Some(next) = self.redo.pop()
        {
            self.undo
                .push(std::mem::replace(&mut self.doc.timeline, next));
            self.after_history_change();
        }
    }

    fn after_history_change(&mut self) {
        self.seek(self.playhead);
        self.dirty = true;
        self.selected_clip = self
            .selected_clip
            .filter(|id| self.doc.timeline.find_clip(*id).is_some());
        self.extra_selection
            .retain(|id| self.doc.timeline.find_clip(*id).is_some());
        if self.selected_clip.is_none() {
            self.selected_clip = self.extra_selection.iter().next().copied();
            if let Some(id) = self.selected_clip {
                self.extra_selection.remove(&id);
            }
        }
    }

    fn clip_position(&self, id: Id) -> Option<(usize, usize)> {
        self.doc
            .timeline
            .tracks
            .iter()
            .enumerate()
            .find_map(|(track, row)| {
                row.clips
                    .iter()
                    .position(|clip| clip.id == id)
                    .map(|index| (track, index))
            })
    }

    pub fn begin_trim(&mut self, trim: TrimState) {
        self.trim_before = Some(self.doc.timeline.clone());
        self.trim = Some(trim);
        self.playing = false;
        self.select_only(trim.clip_id);
    }

    pub fn finish_trim(&mut self) {
        if let (Some(trim), Some(before)) = (self.trim.take(), self.trim_before.take())
            && self
                .doc
                .timeline
                .find_clip(trim.clip_id)
                .is_some_and(|clip| {
                    clip.source_in != trim.orig_source_in
                        || clip.source_duration != trim.orig_source_duration
                })
        {
            self.undo.push(before);
            self.redo.clear();
            self.dirty = true;
        }
    }

    pub fn toggle_magnet(&mut self) {
        self.record_edit();
        self.doc.timeline.magnetic = !self.doc.timeline.magnetic;
    }

    pub fn toggle_track_mute(&mut self, id: Id) {
        if let Some(index) = self
            .doc
            .timeline
            .tracks
            .iter()
            .position(|track| track.id == id)
        {
            self.record_edit();
            let track = &mut self.doc.timeline.tracks[index];
            track.muted = !track.muted;
        }
    }

    pub fn split_target(&self) -> Option<&Clip> {
        let can_split = |clip: &&Clip| {
            clip.audio_parent.is_none()
                && self.playhead > clip.track_start + 1e-8
                && self.playhead < clip.end() - 1e-8
        };
        match self.selected_clip {
            Some(id) => self.doc.timeline.find_clip(id).filter(can_split),
            None => self
                .doc
                .timeline
                .tracks
                .iter()
                .flat_map(|track| &track.clips)
                .find(can_split),
        }
    }

    pub fn split_at_playhead(&mut self) {
        let Some(clip) = self.split_target() else {
            return;
        };
        let Some((track, index)) = self.clip_position(clip.id) else {
            return;
        };
        let consumed = (self.playhead - clip.track_start) * clip.speed;
        let mut right = clip.clone();
        right.id = freecut_core::new_id();
        right.source_in += consumed;
        right.source_duration -= consumed;
        right.track_start = self.playhead;
        self.record_edit();
        let clips = &mut self.doc.timeline.tracks[track].clips;
        clips[index].source_duration = consumed;
        clips[index].transition_out = None;
        self.select_only(right.id);
        let clips = &mut self.doc.timeline.tracks[track].clips;
        clips.insert(index + 1, right);
    }

    pub fn clear_selection(&mut self) {
        self.selected_clip = None;
        self.extra_selection.clear();
    }

    pub fn select_only(&mut self, id: Id) {
        self.clear_selection();
        self.selected_clip = Some(id);
    }

    pub fn is_selected(&self, id: Id) -> bool {
        self.selected_clip == Some(id) || self.extra_selection.contains(&id)
    }

    pub fn select_clip(&mut self, id: Id, modifiers: egui::Modifiers) {
        if modifiers.shift
            && let (Some((row, index)), Some((anchor_row, anchor))) = (
                self.clip_position(id),
                self.selected_clip
                    .and_then(|selected| self.clip_position(selected)),
            )
            && row == anchor_row
        {
            self.extra_selection.extend(
                self.doc.timeline.tracks[row].clips[index.min(anchor)..=index.max(anchor)]
                    .iter()
                    .filter(|clip| Some(clip.id) != self.selected_clip)
                    .map(|clip| clip.id),
            );
            return;
        }
        if modifiers.ctrl || modifiers.shift {
            if self.selected_clip == Some(id) {
                self.selected_clip = self.extra_selection.iter().next().copied();
                if let Some(id) = self.selected_clip {
                    self.extra_selection.remove(&id);
                }
            } else if !self.extra_selection.remove(&id)
                && let Some(previous) = self.selected_clip.replace(id)
            {
                self.extra_selection.insert(previous);
            }
        } else {
            self.select_only(id);
        }
    }

    pub fn delete_selected(&mut self, ripple: bool) {
        if self.selected_clip.is_none() {
            return;
        }
        self.record_edit();
        let removed_audio: HashSet<_> = self
            .doc
            .timeline
            .audio_tracks()
            .flat_map(|track| &track.clips)
            .filter(|clip| self.is_selected(clip.id))
            .filter_map(|clip| clip.audio_parent)
            .collect();
        let magnetic = self.doc.timeline.magnetic;
        for row in &mut self.doc.timeline.tracks {
            let ripple = ripple && !row.video_sound;
            let mut removed_duration = 0.0;
            let before = row.clips.len();
            row.clips.retain_mut(|clip| {
                if removed_audio.contains(&clip.id) {
                    clip.audio_detached = true;
                }
                let remove =
                    self.selected_clip == Some(clip.id) || self.extra_selection.contains(&clip.id);
                if remove {
                    removed_duration += clip.duration();
                } else if ripple && !magnetic {
                    clip.track_start = (clip.track_start - removed_duration).max(0.0);
                }
                !remove
            });
            if ripple && magnetic && row.clips.len() != before {
                pack(row);
            }
        }
        self.clear_selection();
        self.seek(self.playhead);
    }

    pub fn move_clip(&mut self, id: Id, target: f64) {
        let Some((track, index)) = self.clip_position(id) else {
            return;
        };
        let row = &self.doc.timeline.tracks[track];
        let mut candidates = row.clips.clone();
        let mut clip = candidates.remove(index);
        if clip.audio_parent.is_some() {
            return;
        }
        let (start, position) = self.plan_move(&candidates, clip.duration(), target);
        if index == position && (start - clip.track_start).abs() < 1e-8 {
            return;
        }
        self.record_edit();
        clip.track_start = start;
        candidates.insert(position, clip);
        let row = &mut self.doc.timeline.tracks[track];
        row.clips = candidates;
        self.seek(self.playhead);
    }

    pub fn plan_move(&self, clips: &[Clip], duration: f64, target: f64) -> (f64, usize) {
        let target = if self.doc.timeline.magnetic {
            snap_center(clips, target, duration, self.playhead, self.pps)
        } else {
            target
        };
        plan_clips(clips, duration, target, false)
    }

    fn selected_clips(&self) -> Vec<(TrackKind, Clip)> {
        self.doc
            .timeline
            .tracks
            .iter()
            .flat_map(|row| {
                row.clips
                    .iter()
                    .filter(|clip| clip.audio_parent.is_none() && self.is_selected(clip.id))
                    .map(|clip| (row.kind, clip.clone()))
            })
            .collect()
    }

    pub fn copy_selected(&mut self) {
        let selected = self.selected_clips();
        if !selected.is_empty() {
            self.clipboard = selected;
        }
    }

    pub fn paste(&mut self, duplicate: bool) {
        let clips = if duplicate {
            self.selected_clips()
        } else {
            self.clipboard.clone()
        };
        if clips.is_empty() {
            return;
        }
        let first = clips
            .iter()
            .map(|(_, clip)| clip.track_start)
            .fold(f64::INFINITY, f64::min);
        let start = if duplicate {
            clips.iter().map(|(_, clip)| clip.end()).fold(0.0, f64::max)
        } else {
            self.playhead
        };
        self.record_edit();
        self.clear_selection();
        for (kind, mut clip) in clips {
            let Some(track) = self
                .doc
                .timeline
                .tracks
                .iter()
                .find(|track| track.kind == kind && !track.video_sound)
            else {
                continue;
            };
            clip.id = freecut_core::new_id();
            let target = start + clip.track_start - first + clip.duration() * 0.5;
            if let Some(id) = self.place_clip(clip, track.id, target)
                && let Some(previous) = self.selected_clip.replace(id)
            {
                self.extra_selection.insert(previous);
            }
        }
    }

    /// Длительность ассета (0.0, если не найден).
    #[must_use]
    pub fn asset_duration(&self, asset_id: Id) -> f64 {
        self.doc
            .asset(asset_id)
            .map(|asset| asset.duration.max(0.0))
            .unwrap_or_default()
    }
}

/// Приложение FlowCut: одно окно, вход сразу в редактор.
pub struct FlowCutApp {
    st: UiState,
    player: Player,
    imports: Option<Receiver<ImportEvent>>,
    thumbnails: Option<Receiver<Art>>,
    last_tick: Instant,
}

impl FlowCutApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::install(&cc.egui_ctx);
        tracing::info!("FlowCut запущен");
        let mut app = Self {
            st: UiState::new(),
            player: Player::default(),
            imports: None,
            thumbnails: None,
            last_tick: Instant::now(),
        };
        ProjectUi::restore(&mut app.st);
        app.reload_thumbnails(&cc.egui_ctx);
        app
    }

    fn update_preview(&mut self, ctx: &Context) {
        if self.st.playing {
            self.update_audio();
        } else {
            self.stop_audio();
        }
        let time = self.st.playhead;
        let frame_at = |clip: &Clip| VideoFrame::at(clip, self.st.doc.asset(clip.asset_id)?, time);
        let primary = self.st.clip_under_playhead().and_then(frame_at);
        let incoming = self
            .st
            .transition_under_playhead()
            .and_then(|(clip, mix)| Some((frame_at(clip)?, mix, clip.video_gain(time))));
        if let Some(primary) = &primary
            && self.player.incoming.clip == Some(primary.clip_id)
            && self.player.video.clip != Some(primary.clip_id)
        {
            std::mem::swap(&mut self.player.video, &mut self.player.incoming);
        }
        self.player.video.update(ctx, primary, self.st.playing);
        let (secondary, mix, gain) = incoming.map_or((None, 0.0, 1.0), |(frame, mix, gain)| {
            (Some(frame), mix, gain)
        });
        self.player.incoming.update(ctx, secondary, self.st.playing);
        self.st.preview_tex = self.player.video.texture.clone();
        self.st.secondary_preview = self
            .player
            .incoming
            .texture
            .clone()
            .map(|texture| (texture, mix, gain));
        if let Some(error) = self
            .player
            .video
            .error
            .take()
            .or_else(|| self.player.incoming.error.take())
        {
            self.st.notice = Some(error);
        }
    }
    /// Синхронизировать аудио с плейхедом: собрать клипы с аудио, которые
    /// звучат сейчас (плейхед внутри клипа), и сделать их источниками плеера.
    fn update_audio(&mut self) {
        let playhead = self.st.playhead;
        let desired: Vec<freecut_audio::SpawnSource> = self
            .st
            .doc
            .timeline
            .tracks
            .iter()
            .filter(|track| !track.muted)
            .flat_map(|track| track.clips.iter())
            .filter(|clip| !clip.muted)
            .filter_map(|clip| {
                let start = clip.track_start;
                let end = start + clip.duration();
                if playhead < start || playhead >= end {
                    return None;
                }
                let asset = self.st.doc.asset(clip.asset_id)?;
                if !asset.has_audio || clip.separate_audio {
                    return None;
                }
                let offset = (playhead - start) * clip.speed.max(0.01);
                Some(freecut_audio::SpawnSource {
                    clip_id: clip.id,
                    path: asset.path.clone(),
                    start_s: clip.source_in + offset,
                    duration_s: (end - playhead) * clip.speed.max(0.01),
                    volume: clip.volume as f32,
                    speed: clip.speed,
                    local: playhead - start,
                    clip_duration: clip.duration(),
                    fade_in: clip.audio_fade,
                    fade_out: clip.audio_fade_out,
                })
            })
            .collect();
        if !desired.is_empty() {
            self.ensure_audio_player();
        }
        if let Some(audio) = &self.player.audio {
            audio.ensure_sources(&desired);
        }
    }

    /// Тишина: убрать все аудио-источники (пауза/остановка).
    fn stop_audio(&mut self) {
        if let Some(audio) = &self.player.audio {
            audio.clear();
        }
    }

    /// Лениво открыть аудиоустройство; без устройства — warn один раз.
    fn ensure_audio_player(&mut self) {
        if self.player.audio.is_some() || self.player.audio_failed {
            return;
        }
        match freecut_audio::AudioPlayer::open() {
            Ok(player) => {
                self.player.audio = Some(player);
                tracing::info!("аудиовыход открыт");
            }
            Err(error) => {
                self.player.audio_failed = true;
                self.st.notice = Some(format!("Аудиовыход недоступен: {error}"));
            }
        }
    }

    fn update_imports(&mut self, ctx: &Context) {
        if let Some(rx) = &self.imports {
            loop {
                match rx.try_recv() {
                    Ok(ImportEvent::Asset(Ok((asset, thumbnail)))) => {
                        if let Some(image) = thumbnail {
                            let name = format!("thumb_{}", asset.id.simple());
                            let tex =
                                ctx.load_texture(name, image, egui::TextureOptions::default());
                            self.st.thumbs.insert(asset.id, tex);
                        }
                        self.st.doc.assets.push(asset);
                        self.st.dirty = true;
                    }
                    Ok(ImportEvent::Asset(Err(error))) => self.st.notice = Some(error),
                    Ok(ImportEvent::Waveform(id, image)) => {
                        self.st
                            .waves
                            .insert(id, artwork_texture(ctx, id, "wave", image));
                    }
                    Err(TryRecvError::Empty) => return,
                    Err(TryRecvError::Disconnected) => break,
                }
            }
            self.imports = None;
            self.st.importing = false;
        }
        if self.st.pending_import.is_empty() {
            return;
        }
        if ["ffprobe", "ffmpeg"]
            .iter()
            .any(|name| freecut_media::probe::find_tool(name).is_none())
        {
            self.st.notice = Some(freecut_media::Error::ToolchainMissing.to_string());
            self.st.pending_import.clear();
            return;
        }
        let paths = std::mem::take(&mut self.st.pending_import);
        let (tx, rx) = std::sync::mpsc::channel();
        let repaint = ctx.clone();
        match std::thread::Builder::new()
            .name("flowcut_import".into())
            .spawn(move || {
                for path in paths {
                    let asset = import_asset(path);
                    let audio = asset
                        .as_ref()
                        .ok()
                        .map(|(asset, _)| asset)
                        .filter(|asset| asset.has_audio)
                        .cloned();
                    if tx.send(ImportEvent::Asset(asset)).is_err() {
                        break;
                    }
                    repaint.request_repaint();
                    if let Some(asset) = audio
                        && let Some(image) = asset_waveform(&asset)
                    {
                        if tx.send(ImportEvent::Waveform(asset.id, image)).is_err() {
                            break;
                        }
                        repaint.request_repaint();
                    }
                }
                drop(tx);
                repaint.request_repaint();
            }) {
            Ok(_) => {
                self.imports = Some(rx);
                self.st.importing = true;
            }
            Err(error) => self.st.notice = Some(error.to_string()),
        }
    }

    fn reload_thumbnails(&mut self, ctx: &Context) {
        let assets = self.st.doc.assets.clone();
        self.st.thumbs.clear();
        self.st.waves.clear();
        if assets.is_empty() {
            self.thumbnails = None;
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let repaint = ctx.clone();
        self.thumbnails = std::thread::Builder::new()
            .name("flowcut_thumbnails".into())
            .spawn(move || {
                for asset in assets.into_iter().filter(|asset| asset.path.is_file()) {
                    if let Some(image) = asset_thumbnail(&asset) {
                        if tx.send(Art::Thumbnail(asset.id, image)).is_err() {
                            break;
                        }
                        repaint.request_repaint();
                    }
                    if asset.has_audio
                        && let Some(image) = asset_waveform(&asset)
                    {
                        if tx.send(Art::Waveform(asset.id, image)).is_err() {
                            break;
                        }
                        repaint.request_repaint();
                    }
                }
            })
            .ok()
            .map(|_| rx);
    }

    fn poll_thumbnails(&mut self, ctx: &Context) {
        if let Some(rx) = &self.thumbnails {
            for art in rx.try_iter() {
                match art {
                    Art::Thumbnail(id, image) => {
                        self.st
                            .thumbs
                            .insert(id, artwork_texture(ctx, id, "thumb", image));
                    }
                    Art::Waveform(id, image) => {
                        self.st
                            .waves
                            .insert(id, artwork_texture(ctx, id, "wave", image));
                    }
                }
            }
        }
    }
    /// Движение плейхеда во время воспроизведения.
    fn advance_playhead(&mut self, dt: f64) {
        if !(self.st.playing && self.st.duration() > 0.0) {
            return;
        }
        self.st.playhead += dt;
        let end = self.st.duration();
        if self.st.playhead >= end {
            self.st.playhead = end;
            self.st.playing = false; // автостоп в конце, без зацикливания
        }
    }

    /// Глобальные хоткеи.
    fn hotkeys(&mut self, ctx: &Context) {
        let wants_kb = ctx.text_edit_focused();
        let duration = self.st.duration();
        let frame_step = self.st.frame_step();

        ctx.input(|i| {
            if i.modifiers.ctrl {
                if i.key_pressed(egui::Key::S) {
                    self.st.project.action = Some(ProjectAction::Save);
                } else if i.key_pressed(egui::Key::E) {
                    self.st.project.export_open = !self.st.project.export_open;
                } else if i.key_pressed(egui::Key::O) {
                    self.st.project.action = Some(ProjectAction::Open);
                } else if i.key_pressed(egui::Key::N) {
                    self.st.project.action = Some(ProjectAction::New);
                } else if i.key_pressed(egui::Key::I) {
                    self.st.want_import = true;
                } else if !wants_kb && self.st.trim.is_none() {
                    if i.key_pressed(egui::Key::Z) && !i.modifiers.shift {
                        self.st.undo();
                    } else if i.key_pressed(egui::Key::Z) || i.key_pressed(egui::Key::Y) {
                        self.st.redo();
                    } else if i.key_pressed(egui::Key::B) {
                        self.st.split_at_playhead();
                    } else if i.key_pressed(egui::Key::C) {
                        self.st.copy_selected();
                    } else if i.key_pressed(egui::Key::V) {
                        self.st.paste(false);
                    } else if i.key_pressed(egui::Key::D) {
                        self.st.paste(true);
                    }
                }
                return;
            }

            if i.key_pressed(egui::Key::Escape) {
                self.st.renaming = false;
                self.st.clear_selection();
            }

            if wants_kb || self.st.trim.is_some() {
                return; // рядом с текстовыми полями глобальные хоткеи не срабатывают
            }
            if i.key_pressed(egui::Key::Space) {
                self.st.toggle_playback();
            } else if i.key_pressed(egui::Key::S) {
                self.st.split_at_playhead();
            } else if i.key_pressed(egui::Key::Delete) {
                self.st.delete_selected(!i.modifiers.shift);
            } else if i.key_pressed(egui::Key::ArrowLeft) {
                self.st.seek(self.st.playhead - frame_step);
            } else if i.key_pressed(egui::Key::ArrowRight) {
                self.st.seek(self.st.playhead + frame_step);
            } else if i.key_pressed(egui::Key::Home) {
                self.st.seek(0.0);
            } else if i.key_pressed(egui::Key::End) {
                self.st.seek(duration);
            } else if i.key_pressed(egui::Key::J) {
                self.st.seek(self.st.playhead - frame_step * 10.0);
            } else if i.key_pressed(egui::Key::K) {
                self.st.playing = false;
            } else if i.key_pressed(egui::Key::L) {
                self.st.seek(self.st.playhead + frame_step * 10.0);
            }
        });
    }
}

impl eframe::App for FlowCutApp {
    fn logic(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        ProjectUi::handle_close(&mut self.st, ctx);
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        self.editor_ui(ui);
    }
}

impl FlowCutApp {
    fn editor_ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();

        // Drag&drop файлов из ОС: собираем пути (импорт сразу же в этом кадре).
        let dropped: Vec<std::path::PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_owned())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        if !dropped.is_empty() {
            tracing::info!(count = dropped.len(), "перетащены файлы");
            self.st.pending_import.extend(dropped);
        }

        let now = Instant::now();
        let dt = now.duration_since(self.last_tick).as_secs_f64();
        self.last_tick = now;
        self.advance_playhead(dt);
        if self.st.project.confirmation.is_none() {
            self.hotkeys(&ctx);
        }
        self.st.sync_video_sound();
        if ProjectUi::tick(&mut self.st, &ctx) {
            self.reset_media(&ctx);
        }

        // Кнопка «Импортировать файлы» / Ctrl+I: системный диалог выбора файлов.
        if self.st.want_import {
            self.st.want_import = false;
            self.st.playing = false;
            self.stop_audio();
            let picked = rfd::FileDialog::new()
                .add_filter(
                    "Медиафайлы",
                    &[
                        "mp4", "mkv", "webm", "mov", "avi", "m4v", "mp3", "wav", "ogg", "m4a",
                        "aac", "flac", "opus", "png", "jpg", "jpeg", "bmp", "webp", "gif",
                    ],
                )
                .pick_files();
            if let Some(paths) = picked {
                self.st.pending_import.extend(paths);
            }
        }

        self.update_imports(&ctx);
        self.poll_thumbnails(&ctx);

        // Payload перетаскивания из медиатеки живёт ровно пока тянем мышь:
        // без этого «призрак» залип бы после отмененного дропа.
        let dnd_stale =
            ctx.input(|i| !i.pointer.is_decidedly_dragging() && !i.pointer.any_released());
        if dnd_stale {
            egui::DragAndDrop::clear_payload(&ctx);
        }

        if self.st.playing {
            ctx.request_repaint();
        }

        // Кадр превью: живой поток при воспроизведении, seek на паузе.
        self.update_preview(&ctx);

        // Порядок важен: нижняя панель должна получить полную ширину под
        // верхней, поэтому таймлайн добавляем до боковых панелей.
        widgets::top_bar::show(ui, &mut self.st);
        widgets::top_bar::status(ui, &mut self.st);
        widgets::timeline::show(ui, &mut self.st);
        widgets::library::show(ui, &mut self.st);
        if self.st.project.export_open {
            ProjectUi::show_export(ui, &mut self.st);
        } else {
            widgets::inspector::show(ui, &mut self.st);
        }
        widgets::preview::show(ui, &mut self.st);
        self.st.sync_video_sound();
        if ProjectUi::confirmation(ui, &mut self.st) {
            self.reset_media(&ctx);
        }
    }
}

impl VideoPlayer {
    fn update(&mut self, ctx: &Context, frame: Option<VideoFrame>, playing: bool) {
        let Some(frame) = frame else {
            self.stream = None;
            self.session = None;
            self.static_frame = None;
            self.clip = None;
            self.texture = None;
            return;
        };
        self.clip = Some(frame.clip_id);
        if playing && !frame.still {
            self.stream_playback(ctx, &frame);
        } else {
            self.stream = None;
            self.session = None;
            self.seek_static(ctx, &frame.path, frame.key);
        }
    }
    /// Воспроизведение: один ffmpeg-поток на клип, кадры в текстуру в реальном
    /// времени. При скрабе/переключении клипа поток перезапускается (троттлинг).
    fn stream_playback(&mut self, ctx: &Context, frame: &VideoFrame) {
        let VideoFrame {
            clip_id: asset_id,
            key,
            path,
            size,
            fps,
            speed,
            ..
        } = frame;
        let (asset_id, fps, speed, source_t, filters) =
            (*asset_id, *fps, *speed, key.time, key.filters);
        const DRIFT_TOLERANCE: f64 = 0.45;
        const RESTART_GAP: std::time::Duration = std::time::Duration::from_millis(300);
        let speed = speed.clamp(0.25, 4.0);

        let alive = match (&self.stream, self.session) {
            (Some(stream), Some((sid, start, sfps, sspeed, previous_filters)))
                if sid == asset_id
                    && (sspeed - speed).abs() < 1e-6
                    && filters == previous_filters =>
            {
                // Поток paced'ится в реальном времени: кадров fps·speed в
                // секунду, источник съедается на speed за ту же секунду.
                // Времени источника после n кадров = n / fps (а не / fps·speed).
                let expected = start + stream.frames_decoded() as f64 / sfps;
                (expected - source_t).abs() <= DRIFT_TOLERANCE
            }
            _ => false,
        };

        if !alive {
            let throttled = self.last_restart.is_some_and(|t| t.elapsed() < RESTART_GAP);
            let changed_clip = self.session.is_none_or(|(id, _, _, _, _)| id != asset_id);
            if changed_clip || !throttled {
                self.last_restart = Some(Instant::now());
                if changed_clip {
                    self.stream = None;
                    self.texture = None;
                }
                match freecut_media::decoder::FrameStream::start_filtered(
                    path, source_t, *size, fps, speed, filters,
                ) {
                    Ok(stream) => {
                        self.stream = Some(stream); // Drop старого убьёт его ffmpeg
                        self.session = Some((asset_id, source_t, fps, speed, filters));
                        self.last_restart = Some(Instant::now());
                        tracing::debug!("видеопоток перезапущен на {source_t:.2} с");
                    }
                    Err(error) => {
                        tracing::warn!(%error, "не удалось запустить видеопоток");
                        self.stream = None;
                        self.session = None;
                    }
                }
            }
        }

        if let Some(stream) = &self.stream
            && let Some(frame) = stream.take_frame()
        {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [stream.width as usize, stream.height as usize],
                &frame.rgba,
            );
            self.set_preview_image(ctx, image);
            self.static_frame = None;
        }
    }

    /// Пауза: показать точный кадр момента `t` через фоновый ffmpeg seek
    /// (один процесс одновременно, результат подхватывается в следующих кадрах).
    fn seek_static(&mut self, ctx: &Context, path: &std::path::Path, key: FrameKey) {
        if self
            .static_frame
            .is_some_and(|shown| shown.asset != key.asset)
        {
            self.texture = None;
        }
        if let Some((requested, rx)) = &self.seek {
            let requested = *requested;
            match rx.try_recv() {
                Err(TryRecvError::Empty) => return,
                result => {
                    self.seek = None;
                    if requested.asset == key.asset {
                        if let Ok(Ok(png)) = result {
                            if let Some(image) = decode_png(&png) {
                                self.set_preview_image(ctx, image);
                            }
                        } else if requested == key {
                            self.error = Some("Не удалось декодировать кадр".into());
                        }
                        self.static_frame = Some(requested);
                    }
                }
            }
        }
        if self.static_frame == Some(key) {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let path = path.to_owned();
        let repaint = ctx.clone();
        match std::thread::Builder::new()
            .name("flowcut_seek".into())
            .spawn(move || {
                let _ = tx.send(freecut_media::snapshot::snapshot_filtered(
                    &path,
                    key.time,
                    key.filters,
                ));
                repaint.request_repaint();
            }) {
            Ok(_) => self.seek = Some((key, rx)),
            Err(error) => self.error = Some(error.to_string()),
        }
    }
    /// Обновить/создать текстуру превью.
    fn set_preview_image(&mut self, ctx: &Context, image: egui::ColorImage) {
        let size = image.size;
        if let Some(tex) = &mut self.texture
            && tex.size() == size
        {
            tex.set(image, egui::TextureOptions::default());
            return;
        }
        self.texture =
            Some(ctx.load_texture("preview_frame", image, egui::TextureOptions::default()));
    }
}

impl FlowCutApp {
    fn reset_media(&mut self, ctx: &Context) {
        self.st.playing = false;
        self.st.importing = false;
        self.st.preview_tex = None;
        self.player = Player::default();
        self.imports = None;
        self.reload_thumbnails(ctx);
        ctx.request_repaint();
    }
}

pub(crate) fn import_asset(path: PathBuf) -> ImportResult {
    let error = |message: &str| format!("{}: {message}", path.display());
    let info = freecut_media::probe::probe(&path).map_err(|err| error(&err.to_string()))?;
    let kind = if info.has_video {
        match freecut_media::probe::guess_kind(&path) {
            AssetKind::Image => AssetKind::Image,
            _ => AssetKind::Video,
        }
    } else {
        AssetKind::Audio
    };
    let duration = if kind == AssetKind::Image {
        IMAGE_DEFAULT_DURATION
    } else if info.duration > 0.0 {
        info.duration
    } else {
        return Err(error("не удалось определить длительность"));
    };
    let mut asset = Asset::new(kind, path);
    asset.width = info.width;
    asset.height = info.height;
    asset.fps = info.fps;
    asset.has_audio = info.has_audio;
    asset.duration = duration;
    let thumbnail = if info.has_video {
        let png = freecut_media::artwork::thumbnail(&asset)
            .map_err(|err| format!("{}: {err}", asset.path.display()))?;
        Some(
            decode_png(&png)
                .ok_or_else(|| format!("{}: кадр не декодируется как PNG", asset.path.display()))?,
        )
    } else {
        None
    };
    Ok((asset, thumbnail))
}

fn asset_thumbnail(asset: &Asset) -> Option<egui::ColorImage> {
    if asset.kind == AssetKind::Audio {
        return None;
    }
    let png = freecut_media::artwork::thumbnail(asset).ok()?;
    decode_png(&png)
}
fn asset_waveform(asset: &Asset) -> Option<egui::ColorImage> {
    decode_png(&freecut_media::artwork::waveform(asset).ok()?)
}

fn artwork_texture(ctx: &Context, id: Id, kind: &str, image: egui::ColorImage) -> TextureHandle {
    ctx.load_texture(
        format!("{kind}_{}", id.simple()),
        image,
        egui::TextureOptions::default(),
    )
}
/// Декодировать PNG-байты (ffprobe кадр) в текстуру egui.
fn decode_png(png: &[u8]) -> Option<egui::ColorImage> {
    let img = image::load_from_memory(png).ok()?;
    let rgba = img.to_rgba8();
    Some(egui::ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        rgba.as_raw(),
    ))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // тесты вправе паниковать
    use freecut_core::new_id;

    use super::*;

    #[derive(Debug)]
    struct Dropped(PathBuf);

    impl egui::DroppedFile for Dropped {
        fn path(&self) -> &std::path::Path {
            &self.0
        }

        fn bytes(&self) -> Result<Vec<u8>, String> {
            std::fs::read(&self.0).map_err(|error| error.to_string())
        }
    }

    #[test]
    fn editor_imports_local_media_previews_it_and_runs_edit_shortcuts() {
        let Some(ffmpeg) = freecut_media::probe::find_tool("ffmpeg") else {
            return;
        };
        if freecut_media::probe::find_tool("ffprobe").is_none() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("flowcut-ui-{}", new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("Исходник.mp4");
        let result = std::process::Command::new(ffmpeg)
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=s=320x180:r=30",
                "-t",
                "2",
                "-c:v",
                "mpeg4",
            ])
            .arg(&file)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let ctx = Context::default();
        theme::install(&ctx);
        let mut app = FlowCutApp {
            st: UiState::new(),
            player: Player::default(),
            imports: None,
            thumbnails: None,
            last_tick: Instant::now(),
        };
        let frame = |app: &mut FlowCutApp, mut input: egui::RawInput| {
            input.screen_rect = Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 720.0),
            ));
            ctx.run_ui(input, |ui| app.editor_ui(ui))
                .textures_delta
                .clear();
        };
        let wait_until = |app: &mut FlowCutApp, ready: fn(&FlowCutApp) -> bool| {
            let start = Instant::now();
            while !ready(app) && start.elapsed().as_secs_f64() < 5.0 {
                frame(app, Default::default());
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            assert!(ready(app), "{:?}", app.st.notice);
        };
        frame(
            &mut app,
            egui::RawInput {
                dropped_files: vec![
                    std::sync::Arc::new(Dropped(dir.join("missing.mp4"))),
                    std::sync::Arc::new(Dropped(file.clone())),
                ],
                ..Default::default()
            },
        );
        wait_until(&mut app, |app| !app.st.importing);
        assert_eq!(app.st.doc.assets.len(), 1);
        assert_eq!(app.st.duration(), 0.0);
        assert!(
            app.st
                .notice
                .as_ref()
                .is_some_and(|notice| notice.contains("missing.mp4"))
        );
        let id = app.st.doc.assets[0].id;
        assert!(app.st.thumbs.contains_key(&id));
        let _ = app.st.add_asset_to_timeline(id);
        wait_until(&mut app, |app| app.st.preview_tex.is_some());
        app.st.seek(0.75);
        let key = |app: &mut FlowCutApp, key, modifiers| {
            frame(
                app,
                egui::RawInput {
                    events: vec![
                        egui::Event::ModifiersChanged(modifiers),
                        egui::Event::Key {
                            key,
                            physical_key: Some(key),
                            pressed: true,
                            repeat: false,
                            modifiers,
                        },
                    ],
                    ..Default::default()
                },
            );
        };
        key(&mut app, egui::Key::S, egui::Modifiers::NONE);
        assert_eq!(app.st.doc.timeline.tracks[0].clips.len(), 2);
        key(&mut app, egui::Key::Delete, egui::Modifiers::NONE);
        assert_eq!(app.st.duration(), 0.75);
        key(&mut app, egui::Key::Z, egui::Modifiers::CTRL);
        assert_eq!(app.st.duration(), 2.0);
        key(&mut app, egui::Key::Y, egui::Modifiers::CTRL);
        assert_eq!(app.st.duration(), 0.75);
        key(&mut app, egui::Key::Space, egui::Modifiers::NONE);
        assert!(app.st.playing);
        assert_eq!(app.st.playhead, 0.0);
        wait_until(&mut app, |app| {
            app.player
                .video
                .stream
                .as_ref()
                .is_some_and(|stream| stream.frames_decoded() > 0)
        });
        key(&mut app, egui::Key::K, egui::Modifiers::NONE);
        assert!(!app.st.playing);
        drop(app);
        // Seek worker may still be releasing the input file.
        for _ in 0..50 {
            if std::fs::remove_file(&file).is_ok() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        std::fs::remove_dir(dir).unwrap();
    }

    fn clip(start: f64, dur: f64) -> Clip {
        let mut c = Clip::new(new_id());
        c.track_start = start;
        c.source_duration = dur;
        c
    }

    #[test]
    fn plan_appends_magnetically_to_last_clip() {
        let clips = vec![clip(0.0, 5.0), clip(5.0, 4.0)];
        let (start, idx) = plan_clips(&clips, 3.0, 100.0, true);
        assert!(
            (start - 9.0).abs() < 1e-9,
            "виток к концу, получено {start}"
        );
        assert_eq!(idx, 2);
    }

    #[test]
    fn plan_midpoint_insert_before_next() {
        let clips = vec![clip(0.0, 5.0), clip(5.0, 4.0)];
        // t=4 — внутри первого клипа, но правее его «полу»:
        // вставка встык после первого (idx = 1), второй сдвигается на 2с.
        let (start, idx) = plan_clips(&clips, 2.0, 4.0, true);
        assert_eq!(idx, 1);
        assert!((start - 5.0).abs() < 1e-9, "start = {start}");
    }

    #[test]
    fn plan_drops_before_first_clip_magnetically() {
        let clips = vec![clip(30.0, 5.0)];
        // t=10 — до «полу» первого клипа: встаём в его начало.
        let (start, idx) = plan_clips(&clips, 2.0, 10.0, true);
        assert_eq!(idx, 0);
        assert!(start.abs() < 1e-9, "start = {start}");
    }

    #[test]
    fn plan_empty_track_places_under_cursor_and_append_snaps_to_zero() {
        let (start, idx) = plan_clips(&[], 2.0, 10.0, false);
        assert_eq!(idx, 0);
        assert!((start - 9.0).abs() < 1e-9, "центр под курсором = {start}");

        // append (t = INFINITY) на пустой дорожке — в начало.
        let (start, _) = plan_clips(&[], 2.0, f64::INFINITY, true);
        assert!((start - 0.0).abs() < 1e-9, "start = {start}");
    }

    #[test]
    fn insert_clip_shifts_later_clips() {
        let mut st = UiState::new();
        st.doc.timeline.magnetic = true;
        let mut asset = Asset::new(AssetKind::Video, "a.mp4".into());
        asset.duration = 3.0;
        st.doc.assets.push(asset);
        let asset_id = st.doc.assets[0].id;
        let track_id = st.doc.timeline.tracks[0].id;
        st.doc.timeline.tracks[0].clips.push(clip(0.0, 5.0));
        st.doc.timeline.tracks[0].clips.push(clip(5.0, 4.0));

        // Вставка t=4 → между клипами: новый [5..8], второй сдвигается на 8.
        let clip_id = st.insert_clip(asset_id, track_id, 4.0).unwrap();
        let clips = &st.doc.timeline.tracks[0].clips;
        assert_eq!(clips.len(), 3);
        let new_one = clips.iter().find(|c| c.id == clip_id).unwrap();
        assert!((new_one.track_start - 5.0).abs() < 1e-9);
        assert!(
            (clips[2].track_start - 8.0).abs() < 1e-9,
            "сдвиг = {}",
            clips[2].track_start
        );
    }

    #[test]
    fn video_sound_follows_move_until_detached_even_with_magnet() {
        let mut st = UiState::new();
        let mut asset = Asset::new(AssetKind::Video, "with-sound.mp4".into());
        asset.duration = 4.0;
        asset.has_audio = true;
        let asset_id = asset.id;
        st.doc.assets.push(asset);
        let video_id = st.add_asset_to_timeline(asset_id).unwrap();
        st.sync_video_sound();
        let audio = st
            .doc
            .timeline
            .tracks
            .iter()
            .find(|row| row.video_sound)
            .unwrap();
        let audio_id = audio.clips[0].id;
        assert!(st.doc.timeline.find_clip(video_id).unwrap().separate_audio);
        assert_eq!(audio.clips[0].audio_parent, Some(video_id));
        st.toggle_magnet();
        st.move_clip(video_id, 8.0);
        st.sync_video_sound();
        assert_eq!(
            st.doc.timeline.find_clip(video_id).unwrap().track_start,
            6.0
        );
        assert_eq!(
            st.doc.timeline.find_clip(audio_id).unwrap().track_start,
            6.0
        );
        st.detach_audio(audio_id);
        st.move_clip(video_id, 11.0);
        st.sync_video_sound();
        assert_eq!(
            st.doc.timeline.find_clip(video_id).unwrap().track_start,
            9.0
        );
        assert_eq!(
            st.doc.timeline.find_clip(audio_id).unwrap().track_start,
            6.0
        );
        assert_eq!(
            st.doc.timeline.find_clip(audio_id).unwrap().audio_parent,
            None
        );
    }
}
