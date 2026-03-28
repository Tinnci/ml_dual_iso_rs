use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;

use dual_iso_core::{DualIsoAnalysis, ExifInfo, ProcessConfig};
use egui::{ColorImage, DroppedFile, TextureHandle};

use crate::panels::{ExifPanel, FilesPanel, ProgressPanel, SettingsPanel};

// ─── Task progress ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum TaskMsg {
    Progress {
        file: String,
        done: usize,
        total: usize,
    },
    Done,
    Error(String),
}

#[derive(Debug, Default, PartialEq, Eq)]
pub enum AppStatus {
    #[default]
    Idle,
    Running,
    Done,
    Error(String),
}

// ─── App state ───────────────────────────────────────────────────────────────

pub struct App {
    pub files: Vec<PathBuf>,
    pub config: ProcessConfig,
    pub status: AppStatus,
    pub progress_msg: String,
    pub log: Vec<String>,
    /// Index of the currently selected file in the list.
    pub selected_file: Option<usize>,
    /// EXIF metadata cache: path → ExifInfo.
    pub exif_cache: HashMap<PathBuf, ExifInfo>,
    /// Sender half for background EXIF loads (files.rs uses this).
    pub exif_tx: Sender<(PathBuf, ExifInfo)>,
    /// Preview thumbnail cache: path → egui TextureHandle.
    pub preview_cache: HashMap<PathBuf, TextureHandle>,
    /// Sender half for background thumbnail loads.
    pub preview_tx: Sender<(PathBuf, ColorImage)>,
    /// Dual-ISO detection result cache: path → analysis.
    pub analysis_cache: HashMap<PathBuf, DualIsoAnalysis>,
    /// Sender half for background dual-ISO analysis.
    pub analysis_tx: Sender<(PathBuf, DualIsoAnalysis)>,
    /// Paths for which analysis has already been requested.
    pub analysis_pending: HashSet<PathBuf>,

    // Channel for background thread → UI communication.
    msg_rx: Receiver<TaskMsg>,
    msg_tx: Sender<TaskMsg>,
    // Channel for EXIF background loads.
    exif_rx: Receiver<(PathBuf, ExifInfo)>,
    // Channel for thumbnail background loads.
    preview_rx: Receiver<(PathBuf, ColorImage)>,
    // Channel for dual-ISO analysis results.
    analysis_rx: Receiver<(PathBuf, DualIsoAnalysis)>,
}

impl App {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<TaskMsg>();
        let (exif_tx, exif_rx) = std::sync::mpsc::channel::<(PathBuf, ExifInfo)>();
        let (preview_tx, preview_rx) = std::sync::mpsc::channel::<(PathBuf, ColorImage)>();
        let (analysis_tx, analysis_rx) = std::sync::mpsc::channel::<(PathBuf, DualIsoAnalysis)>();
        Self {
            files: Vec::new(),
            config: ProcessConfig::default(),
            status: AppStatus::Idle,
            progress_msg: String::new(),
            log: Vec::new(),
            selected_file: None,
            exif_cache: HashMap::new(),
            exif_tx,
            preview_cache: HashMap::new(),
            preview_tx,
            analysis_cache: HashMap::new(),
            analysis_tx,
            analysis_pending: HashSet::new(),
            msg_rx: rx,
            msg_tx: tx,
            exif_rx,
            preview_rx,
            analysis_rx,
        }
    }

    pub fn add_files(&mut self, paths: impl IntoIterator<Item = PathBuf>) {
        for p in paths {
            if !self.files.contains(&p) {
                self.files.push(p);
            }
        }
    }

    pub fn start_processing(&mut self, ctx: egui::Context) {
        if self.files.is_empty() || self.status == AppStatus::Running {
            return;
        }
        self.status = AppStatus::Running;
        self.log.clear();

        let files = self.files.clone();
        let config = self.config.clone();
        let tx = self.msg_tx.clone();
        let total = files.len();

        thread::spawn(move || {
            use rayon::prelude::*;
            use std::sync::atomic::{AtomicUsize, Ordering};

            let done_count = AtomicUsize::new(0);

            // Process files in parallel via rayon.
            let results: Vec<(String, Result<PathBuf, String>)> = files
                .par_iter()
                .map(|path| {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();

                    let result = (|| -> anyhow::Result<PathBuf> {
                        let raw = dual_iso_core::raw_io::read_raw(path)?;
                        let processed = dual_iso_core::process(raw, &config)?;
                        let out = {
                            let stem = path
                                .file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or("output");
                            let dir = path.parent().unwrap_or(std::path::Path::new("."));
                            dir.join(format!("{stem}.DNG"))
                        };
                        dual_iso_core::dng_output::write_dng(&out, &processed, &config)?;
                        Ok(out)
                    })();

                    let done = done_count.fetch_add(1, Ordering::Relaxed) + 1;
                    let msg = match &result {
                        Ok(out) => format!(
                            "✓ {name} → {}",
                            out.file_name().unwrap_or_default().to_string_lossy()
                        ),
                        Err(e) => format!("✗ {name}: {e:#}"),
                    };
                    let _ = tx.send(TaskMsg::Progress {
                        file: msg,
                        done,
                        total,
                    });
                    ctx.request_repaint();

                    (name, result.map_err(|e| format!("{e:#}")))
                })
                .collect();

            // Report any errors.
            for (name, result) in &results {
                if let Err(e) = result {
                    let _ = tx.send(TaskMsg::Error(format!("{name}: {e}")));
                }
            }

            let _ = tx.send(TaskMsg::Done);
            ctx.request_repaint();
        });
    }

    fn poll_messages(&mut self, ctx: &egui::Context) {
        for msg in self.msg_rx.try_iter() {
            match msg {
                TaskMsg::Progress { file, done, total } => {
                    self.progress_msg = format!("[{done}/{total}] {file}");
                    self.log.push(self.progress_msg.clone());
                }
                TaskMsg::Error(e) => {
                    self.log.push(format!("ERROR: {e}"));
                    self.status = AppStatus::Error(e);
                }
                TaskMsg::Done => {
                    self.status = AppStatus::Done;
                }
            }
        }
        // Drain completed EXIF loads.
        for (path, exif) in self.exif_rx.try_iter() {
            self.exif_cache.insert(path, exif);
        }
        // Drain completed thumbnail loads and register textures.
        for (path, color_image) in self.preview_rx.try_iter() {
            let key = path.to_string_lossy().into_owned();
            let handle = ctx.load_texture(key, color_image, egui::TextureOptions::default());
            self.preview_cache.insert(path, handle);
        }
        // Drain completed dual-ISO analysis results.
        for (path, analysis) in self.analysis_rx.try_iter() {
            self.analysis_cache.insert(path, analysis);
        }
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_messages(ctx);

        // Handle drag-and-drop.
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f: &DroppedFile| f.path.clone())
                .collect()
        });
        if !dropped.is_empty() {
            self.add_files(dropped);
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // Top menu.
        egui::Panel::top("menu").show_inside(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Add files…").clicked() {
                        if let Some(paths) = rfd::FileDialog::new()
                            .add_filter("RAW", &["CR2", "cr2", "DNG", "dng"])
                            .pick_files()
                        {
                            self.add_files(paths);
                        }
                        ui.close();
                    }
                    if ui.button("Clear list").clicked() {
                        self.files.clear();
                        ui.close();
                    }
                });
            });
        });

        // Bottom: progress / action bar.
        egui::Panel::bottom("actions").show_inside(ui, |ui| {
            ProgressPanel::show(self, ui, &ctx);
        });

        // Left: file list + EXIF info.
        egui::Panel::left("files")
            .default_size(340.0)
            .show_inside(ui, |ui| {
                FilesPanel::show(self, ui);
                ui.add_space(8.0);
                ui.separator();
                ExifPanel::show(self, ui);
            });

        // Right: settings.
        egui::CentralPanel::default().show_inside(ui, |ui| {
            SettingsPanel::show(self, ui);
        });
    }
}
