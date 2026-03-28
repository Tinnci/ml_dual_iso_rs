use crate::app::{App, AppStatus};
use egui::{Context, Ui};

fn format_duration(secs: f64) -> String {
    if secs < 60.0 {
        format!("{:.0}s", secs)
    } else {
        format!("{:.0}m {:.0}s", (secs / 60.0).floor(), secs % 60.0)
    }
}

pub struct ProgressPanel;

impl ProgressPanel {
    pub fn show(app: &mut App, ui: &mut Ui, ctx: &Context) {
        let running = app.status == AppStatus::Running;
        let n_files = app.files.len();

        // ── Convert button + status row ───────────────────────────────────
        ui.horizontal(|ui| {
            let btn_label = if running { "Processing..." } else { "Convert" };
            let btn = ui.add_enabled(!running && n_files > 0, egui::Button::new(btn_label));
            if btn.clicked() {
                app.start_processing(ctx.clone());
            }

            match &app.status {
                AppStatus::Idle => {
                    ui.weak(format!("{n_files} file(s) queued"));
                }
                AppStatus::Running => {
                    ui.spinner();
                    let bp = &app.batch_progress;
                    if bp.total > 0 {
                        ui.label(format!("{}/{} files", bp.done, bp.total));
                    } else {
                        ui.label("Starting...");
                    }
                }
                AppStatus::Done => {
                    ui.colored_label(egui::Color32::from_rgb(80, 200, 120), "Done");
                }
                AppStatus::Error(e) => {
                    ui.colored_label(egui::Color32::RED, format!("Error: {e}"));
                }
            }
        });

        // ── Progress bar + timing (only when running or done) ─────────────
        if matches!(app.status, AppStatus::Running | AppStatus::Done) {
            let bp = &app.batch_progress;

            let bar_text = format!("{}/{}", bp.done, bp.total);
            ui.add(
                egui::ProgressBar::new(bp.fraction())
                    .text(bar_text)
                    .animate(running),
            );

            ui.horizontal(|ui| {
                ui.weak(format!("Elapsed: {}", format_duration(bp.elapsed_secs)));
                if let Some(eta) = bp.eta_secs {
                    ui.separator();
                    ui.weak(format!("ETA: {}", format_duration(eta)));
                }
                if !bp.current_file.is_empty() {
                    ui.separator();
                    ui.weak(&bp.current_file);
                }
            });
        }

        // ── Log scroll area ───────────────────────────────────────────────
        if !app.log.is_empty() {
            ui.separator();
            egui::ScrollArea::vertical()
                .max_height(120.0)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for line in &app.log {
                        let color = if line.starts_with("[OK]") {
                            egui::Color32::from_rgb(80, 200, 120)
                        } else if line.starts_with("[FAIL]") {
                            egui::Color32::from_rgb(220, 80, 60)
                        } else {
                            ui.visuals().text_color()
                        };
                        ui.colored_label(color, line);
                    }
                });
        }
    }
}
