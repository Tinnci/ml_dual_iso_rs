mod app;
mod panels;

fn main() -> eframe::Result {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .init();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Dual ISO Converter")
            .with_inner_size([900.0, 620.0])
            .with_min_inner_size([700.0, 450.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };

    eframe::run_native(
        "Dual ISO Converter",
        native_options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
