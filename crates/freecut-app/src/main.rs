//! Точка входа FlowCut: одно окно, сразу в редактор.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")] // без консоли

fn main() -> eframe::Result {
    install_tracing();

    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title("FlowCut")
        .with_inner_size([1440.0, 900.0])
        .with_min_inner_size([1024.0, 640.0]);
    match eframe::icon_data::from_png_bytes(include_bytes!("../assets/flowcut.png")) {
        Ok(icon) => viewport = viewport.with_icon(icon),
        Err(error) => tracing::warn!(%error, "не удалось загрузить иконку окна"),
    }

    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };

    tracing::info!("окно FlowCut запускается (wgpu)");
    eframe::run_native(
        "FlowCut",
        options,
        Box::new(|cc| Ok(Box::new(freecut_ui::FlowCutApp::new(cc)))),
    )
}

fn install_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,freecut=debug"));
    // Логи пишутся только при подключённом консольном выводе, окно без консоли
    // (windows_subsystem = "windows"), поэтому вывод игнорируется по умолчанию.
    #[allow(clippy::let_unit_value)]
    let _ = tracing_subscriber::fmt().with_env_filter(filter).init();
}
