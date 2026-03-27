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

        let Some(exif) = app.exif_cache.get(path) else {
            ui.spinner();
            ui.weak("Loading…");
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
