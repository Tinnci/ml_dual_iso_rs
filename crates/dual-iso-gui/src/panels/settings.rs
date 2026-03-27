use egui::Ui;

use dual_iso_core::{
    BadPixelFix, ChromaSmoothSize, Compression, InterpolationMethod, WhiteBalance,
};
use crate::app::App;

pub struct SettingsPanel;

impl SettingsPanel {
    pub fn show(app: &mut App, ui: &mut Ui) {
        ui.heading("Processing settings");
        ui.separator();

        egui::ScrollArea::vertical().show(ui, |ui| {
            Self::section(ui, "Interpolation", |ui| {
                ui.radio_value(
                    &mut app.config.interp_method,
                    InterpolationMethod::AmazeEdge,
                    "AMaZE + edge-directed (high quality)",
                );
                ui.radio_value(
                    &mut app.config.interp_method,
                    InterpolationMethod::Mean23,
                    "Mean-2/3 (fast)",
                );
            });

            Self::section(ui, "Chroma smoothing", |ui| {
                ui.radio_value(&mut app.config.chroma_smooth, ChromaSmoothSize::TwoByTwo,     "2×2 (default)");
                ui.radio_value(&mut app.config.chroma_smooth, ChromaSmoothSize::ThreeByThree, "3×3");
                ui.radio_value(&mut app.config.chroma_smooth, ChromaSmoothSize::FiveByFive,   "5×5");
                ui.radio_value(&mut app.config.chroma_smooth, ChromaSmoothSize::None,         "None");
            });

            Self::section(ui, "Bad pixel fix", |ui| {
                ui.radio_value(&mut app.config.bad_pixels, BadPixelFix::Normal,     "Normal (default)");
                ui.radio_value(&mut app.config.bad_pixels, BadPixelFix::Aggressive, "Aggressive");
                ui.radio_value(&mut app.config.bad_pixels, BadPixelFix::Disabled,   "Disabled");
            });

            Self::section(ui, "White balance", |ui| {
                let is_graymax  = matches!(app.config.white_balance, WhiteBalance::GrayMax);
                let is_graymed  = matches!(app.config.white_balance, WhiteBalance::GrayMedian);
                let is_exif     = matches!(app.config.white_balance, WhiteBalance::Exif);
                let is_custom   = matches!(app.config.white_balance, WhiteBalance::Custom(..));

                if ui.radio(is_graymax,  "Gray-max (default)") .clicked() { app.config.white_balance = WhiteBalance::GrayMax;     }
                if ui.radio(is_graymed,  "Gray-median")         .clicked() { app.config.white_balance = WhiteBalance::GrayMedian;  }
                if ui.radio(is_exif,     "EXIF")                .clicked() { app.config.white_balance = WhiteBalance::Exif;        }
                if ui.radio(is_custom,   "Custom…")             .clicked() { app.config.white_balance = WhiteBalance::Custom(1.0, 1.0, 1.0); }

                if let WhiteBalance::Custom(ref mut r, ref mut g, ref mut b) = app.config.white_balance {
                    ui.horizontal(|ui| {
                        ui.label("R"); ui.add(egui::DragValue::new(r).speed(0.01).range(0.1_f32..=10.0_f32));
                        ui.label("G"); ui.add(egui::DragValue::new(g).speed(0.01).range(0.1_f32..=10.0_f32));
                        ui.label("B"); ui.add(egui::DragValue::new(b).speed(0.01).range(0.1_f32..=10.0_f32));
                    });
                }
            });

            Self::section(ui, "Post-processing", |ui| {
                ui.checkbox(&mut app.config.use_fullres,    "Full-resolution blending");
                ui.checkbox(&mut app.config.use_alias_map,  "Alias map (shadow aliasing fix)");
                ui.checkbox(&mut app.config.use_stripe_fix, "Horizontal stripe fix");
            });

            Self::section(ui, "Soft-film curve", |ui| {
                let mut enabled = app.config.soft_film_ev != 0.0;
                ui.checkbox(&mut enabled, "Apply soft-film curve");
                if enabled {
                    if app.config.soft_film_ev == 0.0 { app.config.soft_film_ev = 1.0; }
                    ui.horizontal(|ui| {
                        ui.label("EV lift:");
                        ui.add(egui::Slider::new(&mut app.config.soft_film_ev, 0.5..=4.0));
                    });
                } else {
                    app.config.soft_film_ev = 0.0;
                }
            });

            Self::section(ui, "Compression", |ui| {
                ui.radio_value(&mut app.config.compression, Compression::None,     "None (default)");
                ui.radio_value(&mut app.config.compression, Compression::Lossless, "Lossless");
                ui.radio_value(&mut app.config.compression, Compression::Lossy,    "Lossy (use with care)");
            });

            Self::section(ui, "Other", |ui| {
                ui.checkbox(&mut app.config.skip_existing,  "Skip existing DNGs");
                ui.checkbox(&mut app.config.same_levels,    "Same levels (flicker prevention)");
            });
        });
    }

    fn section(ui: &mut Ui, title: &str, add_contents: impl FnOnce(&mut Ui)) {
        ui.add_space(4.0);
        ui.strong(title);
        egui::Frame::group(ui.style()).show(ui, add_contents);
    }
}
