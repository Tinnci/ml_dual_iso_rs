use crate::app::{App, AppStatus};
use egui::{Context, Ui};

pub struct ProgressPanel;

impl ProgressPanel {
    pub fn show(app: &mut App, ui: &mut Ui, ctx: &Context) {
        ui.horizontal(|ui| {
            let running = app.status == AppStatus::Running;
            let n_files = app.files.len();

            let btn_label = if running {
                "⏳ Processing…"
            } else {
                "▶  Convert"
            };
            let btn = ui.add_enabled(!running && n_files > 0, egui::Button::new(btn_label));

            if btn.clicked() {
                app.start_processing(ctx.clone());
            }

            // Status chip.
            match &app.status {
                AppStatus::Idle => {
                    ui.weak(format!("{n_files} file(s) queued"));
                }
                AppStatus::Running => {
                    ui.spinner();
                    ui.label(&app.progress_msg);
                }
                AppStatus::Done => {
                    ui.colored_label(egui::Color32::GREEN, "✓ Done");
                }
                AppStatus::Error(e) => {
                    ui.colored_label(egui::Color32::RED, format!("✗ {e}"));
                }
            }
        });

        // Log scroll area.
        if !app.log.is_empty() {
            ui.separator();
            egui::ScrollArea::vertical()
                .max_height(100.0)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for line in &app.log {
                        ui.label(line);
                    }
                });
        }
    }
}
