use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Instant;

use egui::{Context, Ui};
use freecut_core::{
    Id,
    project::{self, ExportSettings, Recovery, Settings},
};

use crate::app::UiState;
use crate::theme::Palette;

#[derive(Clone)]
pub(crate) enum ProjectAction {
    Save,
    Open,
    OpenRecent(PathBuf),
    New,
    Close,
    Relink(Id),
}

enum ExportEvent {
    Progress(f64),
    Done(freecut_media::Result<Option<PathBuf>>),
}

struct ExportJob {
    rx: Receiver<ExportEvent>,
    cancel: Arc<AtomicBool>,
    started: Instant,
    progress: f64,
}

impl Drop for ExportJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

pub(crate) struct ProjectUi {
    pub path: Option<PathBuf>,
    pub settings: Settings,
    pub action: Option<ProjectAction>,
    pub confirmation: Option<ProjectAction>,
    pub export_open: bool,
    pub completed: Option<PathBuf>,
    pub closing: bool,
    pub export_name: Option<String>,
    export: Option<ExportJob>,
    data: PathBuf,
    autosave: Instant,
}

impl Default for ProjectUi {
    fn default() -> Self {
        Self {
            path: None,
            settings: Settings::default(),
            action: None,
            confirmation: None,
            export_open: false,
            completed: None,
            closing: false,
            export_name: None,
            export: None,
            data: project::data_dir(),
            autosave: Instant::now(),
        }
    }
}

impl ProjectUi {
    pub fn restore(st: &mut UiState) {
        let project = &mut st.project;
        project.settings =
            project::read_json(&project.data.join("settings.json")).unwrap_or_default();
        if let Ok(recovery) = project::read_json::<Recovery>(&project.data.join("recovery.json"))
            && project::validate(&recovery.document).is_ok()
        {
            st.doc = recovery.document;
            project.path = recovery.project_path;
            st.dirty = true;
            st.notice = Some("Восстановлены несохранённые изменения".into());
            return;
        }
        for path in &project.settings.recents {
            if let Ok(doc) = project::open(path) {
                st.doc = doc;
                project.path = Some(path.clone());
                break;
            }
        }
    }

    fn save_settings(&self) -> std::io::Result<()> {
        project::write_json(&self.data.join("settings.json"), &self.settings)
    }

    fn clear_recovery(&self) {
        let _ = std::fs::remove_file(self.data.join("recovery.json"));
    }

    fn remember(&mut self, path: PathBuf) -> std::io::Result<()> {
        self.path = Some(path.clone());
        self.settings.recents.retain(|recent| recent != &path);
        self.settings.recents.insert(0, path);
        self.settings.recents.truncate(12);
        self.save_settings()
    }

    pub fn handle_close(st: &mut UiState, ctx: &Context) {
        if ctx.input(|i| i.viewport().close_requested()) {
            if st.project.closing && st.project.export.is_some() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            } else if !st.project.closing {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                st.project.confirmation = Some(ProjectAction::Close);
                st.playing = false;
                ctx.request_repaint();
            } else {
                st.project.clear_recovery();
            }
        }
    }

    pub fn tick(st: &mut UiState, ctx: &Context) -> bool {
        let mut replaced = false;
        if let Some(action) = st.project.action.take() {
            match action {
                ProjectAction::Save => {
                    Self::save(st);
                }
                ProjectAction::Relink(id) => replaced = Self::relink(st, id),
                _ if st.dirty => st.project.confirmation = Some(action),
                _ => replaced = Self::perform(st, ctx, action),
            }
        }
        if st.dirty {
            let interval = std::time::Duration::from_secs(20);
            if st.project.autosave.elapsed() >= interval {
                st.project.autosave = Instant::now();
                let recovery = Recovery {
                    project_path: st.project.path.clone(),
                    document: st.doc.clone(),
                };
                if let Err(error) =
                    project::write_json(&st.project.data.join("recovery.json"), &recovery)
                {
                    st.notice = Some(format!("Автосохранение: {error}"));
                }
            }
            ctx.request_repaint_after(interval.saturating_sub(st.project.autosave.elapsed()));
        }
        Self::poll_export(st);
        if st.project.closing && st.project.export.is_none() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        replaced
    }

    fn save(st: &mut UiState) -> bool {
        st.playing = false;
        st.sync_video_sound();
        let path = st.project.path.clone().or_else(|| {
            rfd::FileDialog::new()
                .add_filter("Проект FlowCut", &["flowcut"])
                .set_file_name(format!("{}.flowcut", st.doc.display_name()))
                .save_file()
                .map(|path| {
                    if path.extension().is_none() {
                        path.with_extension("flowcut")
                    } else {
                        path
                    }
                })
        });
        let Some(path) = path else {
            return false;
        };
        let mut doc = st.doc.clone();
        if doc.name.trim().is_empty() {
            doc.name = path
                .file_stem()
                .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
        }
        match project::write_json(&path, &doc) {
            Ok(()) => {
                st.doc.name = doc.name;
                st.dirty = false;
                st.project.clear_recovery();
                st.project.completed = Some(path.clone());
                st.notice = Some("Проект сохранён".into());
                if let Err(error) = st.project.remember(path) {
                    st.notice = Some(format!("Проект сохранён. Настройки: {error}"));
                }
                true
            }
            Err(error) => {
                st.notice = Some(format!("Не удалось сохранить проект: {error}"));
                false
            }
        }
    }

    fn perform(st: &mut UiState, ctx: &Context, action: ProjectAction) -> bool {
        let path = match action {
            ProjectAction::Open => {
                let Some(path) = rfd::FileDialog::new()
                    .add_filter("Проект FlowCut", &["flowcut", "freecut"])
                    .pick_file()
                else {
                    return false;
                };
                Some(path)
            }
            ProjectAction::OpenRecent(path) => Some(path),
            ProjectAction::New => None,
            ProjectAction::Close => {
                st.project.clear_recovery();
                st.project.closing = true;
                if let Some(export) = &st.project.export {
                    export.cancel.store(true, Ordering::Relaxed);
                    st.notice = Some("Останавливаем экспорт перед закрытием…".into());
                } else {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                return false;
            }
            ProjectAction::Save | ProjectAction::Relink(_) => return false,
        };
        let doc = match path.as_deref().map(project::open).transpose() {
            Ok(doc) => doc.unwrap_or_default(),
            Err(error) => {
                st.notice = Some(format!("Не удалось открыть проект: {error}"));
                return false;
            }
        };
        st.project.clear_recovery();
        st.replace_document(doc);
        st.project.path = None;
        if let Some(path) = path
            && let Err(error) = st.project.remember(path)
        {
            st.notice = Some(format!("Настройки: {error}"));
        }
        true
    }

    fn relink(st: &mut UiState, id: Id) -> bool {
        let Some(asset) = st.doc.asset(id) else {
            return false;
        };
        let Some(path) = rfd::FileDialog::new().pick_file() else {
            return false;
        };
        match crate::app::import_asset(path) {
            Ok((replacement, _))
                if replacement.kind == asset.kind
                    && replacement.duration + 1e-6 >= asset.duration =>
            {
                if let Some(asset) = st.doc.assets.iter_mut().find(|asset| asset.id == id) {
                    *asset = freecut_core::Asset { id, ..replacement };
                    st.dirty = true;
                    return true;
                }
            }
            Ok(_) => st.notice = Some("Выберите источник того же типа и длительности".into()),
            Err(error) => st.notice = Some(error),
        }
        false
    }

    pub fn confirmation(ui: &mut Ui, st: &mut UiState) -> bool {
        let Some(action) = st.project.confirmation.take() else {
            return false;
        };
        let mut replaced = false;
        let mut keep = true;
        let closing = matches!(action, ProjectAction::Close);
        let dialog =
            egui::Modal::new(egui::Id::unique("project_confirmation")).show(ui.ctx(), |ui| {
                ui.heading(if closing {
                    "Закрыть FlowCut?"
                } else {
                    "Сохранить проект?"
                });
                ui.label(if st.dirty {
                    "В проекте есть несохранённые изменения."
                } else if st.project.export.is_some() {
                    "Экспорт ещё выполняется."
                } else {
                    "Проект можно сохранить перед выходом."
                });
                ui.horizontal_wrapped(|ui| {
                    if ui
                        .button(if closing {
                            "Сохранить и выйти"
                        } else {
                            "Сохранить и продолжить"
                        })
                        .clicked()
                        && Self::save(st)
                    {
                        replaced = Self::perform(st, ui.ctx(), action.clone());
                        keep = false;
                    }
                    if ui
                        .button(if closing {
                            "Выйти без сохранения"
                        } else {
                            "Продолжить без сохранения"
                        })
                        .clicked()
                    {
                        replaced = Self::perform(st, ui.ctx(), action.clone());
                        keep = false;
                    }
                    if ui
                        .button(if closing {
                            "Отменить выход"
                        } else {
                            "Отмена"
                        })
                        .clicked()
                    {
                        keep = false;
                    }
                });
            });
        if keep && !dialog.should_close() {
            st.project.confirmation = Some(action);
        }
        replaced
    }

    pub fn show_export(ui: &mut Ui, st: &mut UiState) {
        egui::Panel::right("export_panel")
            .default_size(300.0)
            .min_size(240.0)
            .frame(egui::Frame::new().fill(Palette::PANEL).inner_margin(12))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(crate::theme::section_title("Экспорт MP4"));
                    if ui.small_button("Закрыть").clicked() {
                        st.project.export_open = false;
                    }
                });
                ui.separator();
                if let Some(job) = &st.project.export {
                    ui.add(egui::ProgressBar::new(job.progress as f32).show_percentage());
                    if job.progress > 0.01 {
                        let remaining = job.started.elapsed().as_secs_f64() * (1.0 - job.progress)
                            / job.progress;
                        ui.label(format!("Осталось примерно {:.0} с", remaining));
                    }
                    if ui
                        .add_enabled(
                            !job.cancel.load(Ordering::Relaxed),
                            egui::Button::new("Отменить экспорт"),
                        )
                        .clicked()
                    {
                        job.cancel.store(true, Ordering::Relaxed);
                    }
                    return;
                }
                egui::ScrollArea::vertical().show(ui, |ui| {
                    ui.label("Имя файла");
                    let name = st
                        .project
                        .export_name
                        .get_or_insert_with(|| st.doc.display_name());
                    ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
                    ui.label(
                        egui::RichText::new("Расширение .mp4 добавляется автоматически").small(),
                    );
                    let before = st.project.settings.export;
                    export_settings(ui, &mut st.project.settings.export);
                    let mut persist = before != st.project.settings.export;
                    ui.separator();
                    if let Some(path) = &st.project.settings.export_dir {
                        ui.label(path.display().to_string());
                    }
                    if ui.button("Выбрать папку").clicked() {
                        persist |= Self::choose_export_dir(st);
                    }
                    if persist {
                        Self::persist_settings(st);
                    }
                    if ui
                        .add_enabled(st.duration() > 0.0, egui::Button::new("Экспортировать"))
                        .clicked()
                    {
                        Self::start_export(st, ui.ctx());
                    }
                });
            });
    }

    fn choose_export_dir(st: &mut UiState) -> bool {
        let mut dialog = rfd::FileDialog::new();
        if let Some(path) = &st.project.settings.export_dir {
            dialog = dialog.set_directory(path);
        }
        if let Some(path) = dialog.pick_folder() {
            st.project.settings.export_dir = Some(path);
            true
        } else {
            false
        }
    }

    fn persist_settings(st: &mut UiState) {
        if let Err(error) = st.project.save_settings() {
            st.notice = Some(format!("Не удалось сохранить настройки: {error}"));
        }
    }

    fn start_export(st: &mut UiState, ctx: &Context) {
        st.playing = false;
        let raw_name = st.project.export_name.as_deref().unwrap_or("").trim();
        let name = if raw_name.len() >= 4
            && raw_name
                .get(raw_name.len() - 4..)
                .is_some_and(|suffix| suffix.eq_ignore_ascii_case(".mp4"))
        {
            raw_name[..raw_name.len() - 4].trim()
        } else {
            raw_name
        };
        if name.is_empty()
            || name.ends_with('.')
            || name
                .chars()
                .any(|c| c.is_control() || "<>:\"/\\|?*".contains(c))
        {
            st.notice = Some("Введите имя файла без символов \\ / : * ? \" < > |".into());
            return;
        }
        let name = name.to_owned();
        if st
            .project
            .settings
            .export_dir
            .as_ref()
            .is_none_or(|dir| !dir.is_dir())
            && !Self::choose_export_dir(st)
        {
            return;
        }
        let Some(dir) = st.project.settings.export_dir.clone() else {
            return;
        };
        Self::persist_settings(st);
        let doc = st.doc.clone();
        let settings = st.project.settings.export;
        let cancel = Arc::new(AtomicBool::new(false));
        let signal = Arc::clone(&cancel);
        let (tx, rx) = std::sync::mpsc::channel();
        let repaint = ctx.clone();
        match std::thread::Builder::new()
            .name("flowcut_export".into())
            .spawn(move || {
                let result = freecut_media::export::render_named(
                    &doc,
                    &dir,
                    &name,
                    settings,
                    &signal,
                    |progress| {
                        let _ = tx.send(ExportEvent::Progress(progress));
                        repaint.request_repaint();
                    },
                );
                let _ = tx.send(ExportEvent::Done(result));
                repaint.request_repaint();
            }) {
            Ok(_) => {
                st.project.export = Some(ExportJob {
                    rx,
                    cancel,
                    started: Instant::now(),
                    progress: 0.0,
                });
                st.project.completed = None;
                st.notice = None;
            }
            Err(error) => st.notice = Some(error.to_string()),
        }
    }

    fn poll_export(st: &mut UiState) {
        let Some(job) = &mut st.project.export else {
            return;
        };
        loop {
            let notice = match job.rx.try_recv() {
                Ok(ExportEvent::Progress(progress)) => {
                    if progress < job.progress {
                        job.started = Instant::now();
                    }
                    job.progress = progress;
                    continue;
                }
                Ok(ExportEvent::Done(Ok(Some(path)))) => {
                    st.project.completed = Some(path);
                    "Экспорт готов".into()
                }
                Ok(ExportEvent::Done(Ok(None))) => "Экспорт отменён".into(),
                Ok(ExportEvent::Done(Err(error))) => format!("Ошибка экспорта: {error}"),
                Err(TryRecvError::Disconnected) => "Экспорт неожиданно завершился".into(),
                Err(TryRecvError::Empty) => return,
            };
            st.notice = Some(notice);
            st.project.export = None;
            break;
        }
    }
}

fn export_settings(ui: &mut Ui, settings: &mut ExportSettings) {
    egui::ComboBox::from_label("Разрешение")
        .selected_text(format!("{}p", settings.height))
        .show_ui(ui, |ui| {
            for height in [720, 1080, 1440, 2160] {
                ui.selectable_value(&mut settings.height, height, format!("{height}p"));
            }
        });
    egui::ComboBox::from_label("Кадров в секунду")
        .selected_text(settings.fps.to_string())
        .show_ui(ui, |ui| {
            for fps in [24, 25, 30, 50, 60] {
                ui.selectable_value(&mut settings.fps, fps, fps.to_string());
            }
        });
    egui::ComboBox::from_label("Кодек")
        .selected_text(if settings.hevc { "H.265" } else { "H.264" })
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut settings.hevc, false, "H.264");
            ui.selectable_value(&mut settings.hevc, true, "H.265");
        });
    let labels = ["Низкое", "Среднее", "Высокое"];
    if let Some(kbps) = &mut settings.bitrate_kbps {
        ui.add(
            egui::DragValue::new(kbps)
                .range(500..=100_000)
                .suffix(" кбит/с"),
        );
        if ui.button("Пресет качества").clicked() {
            settings.bitrate_kbps = None;
        }
    } else {
        egui::ComboBox::from_label("Качество")
            .selected_text(labels[usize::from(settings.quality.min(2))])
            .show_ui(ui, |ui| {
                for (quality, label) in labels.into_iter().enumerate() {
                    ui.selectable_value(&mut settings.quality, quality as u8, label);
                }
            });
        if ui.button("Задать битрейт").clicked() {
            settings.bitrate_kbps = Some(8_000);
        }
    }
}

pub(crate) fn reveal(path: &Path) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut command = std::process::Command::new("explorer.exe");
        command
            .raw_arg(format!("/select,\"{}\"", path.display()))
            .spawn()?;
    }
    #[cfg(target_os = "macos")]
    std::process::Command::new("open")
        .arg("-R")
        .arg(path)
        .spawn()?;
    #[cfg(all(not(windows), not(target_os = "macos")))]
    if let Some(dir) = path.parent() {
        std::process::Command::new("xdg-open").arg(dir).spawn()?;
    }
    Ok(())
}
