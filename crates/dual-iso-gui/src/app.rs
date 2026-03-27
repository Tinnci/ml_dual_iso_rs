use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender};
use std::thread;

use dual_iso_core::ProcessConfig;
use egui::DroppedFile;

use crate::panels::{FilesPanel, ProgressPanel, SettingsPanel};

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

    // Channel for background thread → UI communication.
    msg_rx: Receiver<TaskMsg>,
    msg_tx: Sender<TaskMsg>,
}

impl App {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let (tx, rx) = std::sync::mpsc::channel::<TaskMsg>();
        Self {
            files: Vec::new(),
            config: ProcessConfig::default(),
            status: AppStatus::Idle,
            progress_msg: String::new(),
            log: Vec::new(),
            msg_rx: rx,
            msg_tx: tx,
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
            for (i, path) in files.iter().enumerate() {
                let name = path
                    .file_name()
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_default();

                let _ = tx.send(TaskMsg::Progress {
                    file: name.clone(),
                    done: i,
                    total,
                });
                ctx.request_repaint();

                // Read → process → write.
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

                match result {
                    Ok(out) => {
                        let _ = tx.send(TaskMsg::Progress {
                            file: format!(
                                "✓ {name} → {}",
                                out.file_name().unwrap_or_default().to_string_lossy()
                            ),
                            done: i + 1,
                            total,
                        });
                    }
                    Err(e) => {
                        let _ = tx.send(TaskMsg::Error(format!("{name}: {e:#}")));
                    }
                }
                ctx.request_repaint();
            }
            let _ = tx.send(TaskMsg::Done);
            ctx.request_repaint();
        });
    }

    fn poll_messages(&mut self) {
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
    }
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_messages();

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
        egui::Panel::top("menu").show(&ctx, |ui| {
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
        egui::Panel::bottom("actions").show(&ctx, |ui| {
            ProgressPanel::show(self, ui, &ctx);
        });

        // Left: file list.
        egui::Panel::left("files")
            .default_size(340.0)
            .show(&ctx, |ui| {
                FilesPanel::show(self, ui);
            });

        // Right: settings.
        egui::CentralPanel::default().show_inside(ui, |ui| {
            SettingsPanel::show(self, ui);
        });
    }
}
