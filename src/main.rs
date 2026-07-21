#[cfg(not(windows))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod app;
mod backend;
mod config;
mod observe;
mod platform;

fn main() -> eframe::Result<()> {
    let start_minimized = platform::has_minimized_arg();
    let icon = eframe::icon_data::from_png_bytes(platform::app_icon_bytes())
        .expect("embedded app icon should be a valid png");
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("RiProxy")
            .with_inner_size([1120.0, 760.0])
            .with_min_inner_size([860.0, 560.0])
            .with_icon(icon)
            .with_visible(!start_minimized),
        ..Default::default()
    };

    eframe::run_native(
        "RiProxy",
        options,
        Box::new(move |cc| Ok(Box::new(app::ProxyGuiApp::new(cc, start_minimized)))),
    )
}
