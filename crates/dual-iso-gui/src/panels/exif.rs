use egui::Ui;

use crate::app::App;

pub struct ExifPanel;

impl ExifPanel {
    pub fn show(app: &App, ui: &mut Ui) {
        ui.heading("EXIF Info");
        ui.separator();

        let selected = app.selected_file.and_then(|i| app.files.get(i));

        let Some(path) = selected else {
            ui.weak("Select a file to view EXIF data.");
            return;
        };

        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        ui.label(egui::RichText::new(&name).strong());
        ui.add_space(4.0);

        // ── Dual-ISO analysis ────────────────────────────────────────────
        if let Some(analysis) = app.analysis_cache.get(path) {
            let (color, badge) = if analysis.is_dual_iso {
                (egui::Color32::from_rgb(80, 200, 120), "DUAL ISO")
            } else {
                (egui::Color32::from_rgb(200, 120, 80), "NOT DUAL ISO")
            };
            ui.horizontal(|ui| {
                ui.colored_label(color, badge);
                ui.label(&analysis.status);
            });
            if analysis.is_dual_iso {
                ui.horizontal(|ui| {
                    ui.weak(format!(
                        "Pattern: {}  Confidence: {:.0}%",
                        analysis.pattern,
                        analysis.confidence * 100.0
                    ));
                });
            }
            ui.add_space(4.0);
        }

        // ── Preview thumbnail ──────────────────────────────────────────────
        if let Some(texture) = app.preview_cache.get(path) {
            let avail_w = ui.available_width();
            let [tw, th] = [texture.size()[0] as f32, texture.size()[1] as f32];
            let scale = (avail_w / tw).min(1.0);
            let size = egui::vec2(tw * scale, th * scale);
            ui.image(egui::load::SizedTexture::new(texture.id(), size));
            ui.add_space(4.0);
        } else {
            // Show a spinner while loading thumbnail
            ui.horizontal(|ui| {
                ui.spinner();
                ui.weak("Loading preview…");
            });
            ui.add_space(4.0);
        }

        let Some(exif) = app.exif_cache.get(path) else {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.weak("Loading EXIF…");
            });
            return;
        };

        // Camera / lens
        egui::Grid::new("exif_grid")
            .num_columns(2)
            .spacing([12.0, 4.0])
            .striped(true)
            .show(ui, |ui| {
                exif_row(ui, "Shutter", &exif.exposure_time);
                exif_row(ui, "Aperture", &exif.fnumber);
                if let Some(iso) = exif.iso {
                    ui.label("ISO");
                    ui.label(iso.to_string());
                    ui.end_row();
                }
                exif_row(ui, "Focal length", &exif.focal_length);
                exif_row(ui, "Date/Time", &exif.date_time_original);
                exif_row(ui, "Lens", &exif.lens_model);
            });
    }
}

fn exif_row(ui: &mut Ui, label: &str, value: &Option<String>) {
    if let Some(v) = value {
        ui.label(label);
        ui.label(v);
        ui.end_row();
    }
}
