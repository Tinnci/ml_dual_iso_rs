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

        egui::ScrollArea::vertical().show(ui, |ui| {
            for (i, path) in app.files.iter().enumerate() {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.display().to_string());

                ui.horizontal(|ui| {
                    ui.label(&name).on_hover_text(path.display().to_string());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.small_button("✕").clicked() {
                            remove_idx = Some(i);
                        }
                    });
                });
            }
        });

        if let Some(i) = remove_idx {
            app.files.remove(i);
        }
    }
}
