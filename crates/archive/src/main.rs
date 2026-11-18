#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::path::PathBuf;

use archive_core::{extract_entry, list_entries, ArchiveEntry};
use eframe::egui;

fn main() -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1120.0, 720.0])
            .with_min_inner_size([760.0, 520.0])
            .with_title("Archive"),
        ..Default::default()
    };

    eframe::run_native(
        "Archive",
        options,
        Box::new(|cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(ArchiveApp::default()))
        }),
    )
}

#[derive(Default)]
struct ArchiveApp {
    archive_path: Option<PathBuf>,
    entries: Vec<ArchiveEntry>,
    selected: Option<usize>,
    status: String,
    search: String,
    dark_mode: bool,
}

impl ArchiveApp {
    fn open_archive(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Archive", &["zip", "7z"])
            .pick_file()
        {
            match list_entries(&path) {
                Ok(entries) => {
                    self.archive_path = Some(path.clone());
                    self.entries = entries;
                    self.selected = None;
                    self.search.clear();
                    self.status = format!(
                        "{} Einträge aus {} geladen",
                        self.entries.len(),
                        path.file_name().and_then(|n| n.to_str()).unwrap_or("Archiv")
                    );
                }
                Err(error) => self.status = format!("Archiv konnte nicht geöffnet werden: {error}"),
            }
        }
    }

    fn extract_selected(&mut self) {
        let Some(index) = self.selected else {
            self.status = "Bitte zuerst eine Datei auswählen.".into();
            return;
        };
        let Some(archive) = self.archive_path.clone() else {
            return;
        };
        let entry = &self.entries[index];
        if entry.is_directory {
            self.status = "Die Einzel-Extraktion von Verzeichnissen ist noch nicht verfügbar.".into();
            return;
        }
        let Some(destination) = rfd::FileDialog::new().pick_folder() else {
            return;
        };
        match extract_entry(&archive, &entry.name, &destination) {
            Ok(()) => self.status = format!("Extrahiert: {}", entry.name),
            Err(error) => self.status = format!("Extraktion fehlgeschlagen: {error}"),
        }
    }

    fn visible_entries(&self) -> Vec<(usize, &ArchiveEntry)> {
        let query = self.search.trim().to_lowercase();
        self.entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| query.is_empty() || entry.name.to_lowercase().contains(&query))
            .collect()
    }

    fn total_size(&self) -> u64 {
        self.entries.iter().map(|entry| entry.size).sum()
    }
}

impl eframe::App for ArchiveApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.set_visuals(if self.dark_mode {
            egui::Visuals::dark()
        } else {
            egui::Visuals::light()
        });

        egui::TopBottomPanel::top("top_bar")
            .exact_height(64.0)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(12.0);
                    ui.heading(egui::RichText::new("▦  Archive").size(23.0).strong());
                    ui.add_space(24.0);

                    if ui.add_sized([132.0, 36.0], egui::Button::new("＋  Öffnen")).clicked() {
                        self.open_archive();
                    }
                    let extract_enabled = self.selected.is_some();
                    if ui.add_enabled(
                        extract_enabled,
                        egui::Button::new("↓  Extrahieren"),
                    ).clicked() {
                        self.extract_selected();
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(if self.dark_mode { "☀" } else { "☾" })
                            .on_hover_text("Darstellung wechseln")
                            .clicked()
                        {
                            self.dark_mode = !self.dark_mode;
                        }
                        ui.add_space(8.0);
                    });
                });
            });

        egui::SidePanel::left("navigation")
            .resizable(false)
            .exact_width(205.0)
            .show(ctx, |ui| {
                ui.add_space(18.0);
                ui.label(egui::RichText::new("ARBEITSBEREICH").size(11.0).weak().strong());
                ui.add_space(8.0);
                ui.selectable_label(true, "▣  Archiv");
                ui.add_space(22.0);
                ui.label(egui::RichText::new("ARCHIVINFORMATIONEN").size(11.0).weak().strong());
                ui.add_space(10.0);

                if let Some(path) = &self.archive_path {
                    ui.label(egui::RichText::new("DATEI").size(10.0).weak());
                    ui.label(
                        path.file_name()
                            .and_then(|name| name.to_str())
                            .unwrap_or("Unbenanntes Archiv"),
                    );
                    ui.add_space(12.0);
                    ui.label(egui::RichText::new("INHALT").size(10.0).weak());
                    ui.label(format!("{} Einträge", self.entries.len()));
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new("GESAMTGRÖSSE").size(10.0).weak());
                    ui.label(format_bytes(self.total_size()));
                } else {
                    ui.label("Noch kein Archiv geöffnet.");
                }

                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.separator();
                    ui.label(egui::RichText::new("ArchiveTool 0.2.0").weak().small());
                });
            });

        egui::TopBottomPanel::bottom("status_bar")
            .exact_height(30.0)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.add_space(8.0);
                    ui.label(egui::RichText::new("●").color(egui::Color32::from_rgb(70, 180, 120)));
                    ui.label(if self.status.is_empty() {
                        "Bereit".to_string()
                    } else {
                        self.status.clone()
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(format!("{} Elemente", self.entries.len()));
                        ui.add_space(8.0);
                    });
                });
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.heading("Archivinhalt");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.search)
                            .hint_text("⌕  Dateien suchen …")
                            .desired_width(230.0),
                    );
                });
            });
            ui.add_space(8.0);

            if self.archive_path.is_none() {
                ui.add_space(70.0);
                ui.vertical_centered(|ui| {
                    ui.heading(egui::RichText::new("Kein Archiv geöffnet").size(25.0));
                    ui.add_space(8.0);
                    ui.label("Öffne ein ZIP- oder 7z-Archiv, um dessen Inhalt anzuzeigen.");
                    ui.add_space(18.0);
                    if ui.add_sized([170.0, 40.0], egui::Button::new("＋  Archiv öffnen")).clicked() {
                        self.open_archive();
                    }
                });
                return;
            }

            egui::Frame::group(ui.style())
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    egui::ScrollArea::both().auto_shrink([false, false]).show(ui, |ui| {
                        egui::Grid::new("archive_entries")
                            .num_columns(4)
                            .striped(true)
                            .min_col_width(70.0)
                            .show(ui, |ui| {
                                ui.strong("Name");
                                ui.strong("Größe");
                                ui.strong("Komprimiert");
                                ui.strong("Typ");
                                ui.end_row();

                                for (index, entry) in self.visible_entries() {
                                    let selected = self.selected == Some(index);
                                    let icon = if entry.is_directory { "📁" } else { "📄" };
                                    if ui.selectable_label(
                                        selected,
                                        format!("{icon}  {}", entry.name),
                                    ).clicked() {
                                        self.selected = Some(index);
                                    }
                                    ui.label(format_bytes(entry.size));
                                    ui.label(
                                    entry.compressed_size
                                    .map(format_bytes)
                                    .unwrap_or_else(|| "–".to_string())
                                    );
                                    ui.label(if entry.is_directory { "Ordner" } else { "Datei" });
                                    ui.end_row();
                                }
                            });
                    });
                });

            if let Some(index) = self.selected {
                if let Some(entry) = self.entries.get(index) {
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("Ausgewählt:").weak());
                        ui.label(&entry.name);
                    });
                }
            }
        });
    }
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}
