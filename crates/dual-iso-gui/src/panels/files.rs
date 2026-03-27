use crate::app::App;
use egui::Ui;

pub struct FilesPanel;

impl FilesPanel {
    pub fn show(app: &mut App, ui: &mut Ui) {
        ui.heading("Input files");
        ui.separator();

        if app.files.is_empty() {
            ui.weak("Drop CR2 / DNG files here, or use File → Add files…");
        }

        let mut remove_idx: Option<usize> = None;
        let mut load_exif_for: Option<std::path::PathBuf> = None;
        let mut load_preview_for: Option<std::path::PathBuf> = None;

        egui::ScrollArea::vertical()
            .max_height(200.0)
            .show(ui, |ui| {
                for (i, path) in app.files.iter().enumerate() {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| path.display().to_string());

                    let selected = app.selected_file == Some(i);
                    ui.horizontal(|ui| {
                        let resp = ui
                            .selectable_label(selected, &name)
                            .on_hover_text(path.display().to_string());
                        if resp.clicked() {
                            app.selected_file = Some(i);
                            if !app.exif_cache.contains_key(path) {
                                load_exif_for = Some(path.clone());
                            }
                            if !app.preview_cache.contains_key(path) {
                                load_preview_for = Some(path.clone());
                            }
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("✕").clicked() {
                                remove_idx = Some(i);
                            }
                        });
                    });
                }
            });

        // Kick off a blocking EXIF read on a background thread.
        if let Some(p) = load_exif_for {
            let path_clone = p.clone();
            let tx = app.exif_tx.clone();
            std::thread::spawn(move || {
                let raw = dual_iso_core::raw_io::read_raw(&path_clone);
                if let Ok(img) = raw {
                    let _ = tx.send((path_clone, img.meta.exif));
                }
            });
        }

        // Kick off thumbnail extraction on a background thread.
        if let Some(p) = load_preview_for {
            let path_clone = p.clone();
            let tx = app.preview_tx.clone();
            std::thread::spawn(move || {
                if let Some((w, h, rgb)) = dual_iso_core::raw_io::extract_thumbnail(&path_clone) {
                    let size = [w as usize, h as usize];
                    let color_image = egui::ColorImage::from_rgb(size, &rgb);
                    let _ = tx.send((path_clone, color_image));
                }
            });
        }

        if let Some(i) = remove_idx {
            if app.selected_file == Some(i) {
                app.selected_file = None;
            }
            app.files.remove(i);
        }
    }
}
