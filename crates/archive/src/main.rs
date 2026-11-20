#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::SystemTime;

use archive_core::{
    ArchiveEntry, ArchiveFormat, OperationReport, add_to_zip, create_7z, create_zip, extract_all,
    extract_entry, extract_selection, list_entries, remove_entries_from_zip, rename_entry_in_zip,
    replace_entry_in_zip, test_archive,
};
use eframe::egui;

/// Office-Akzentfarbe.
const ACCENT: egui::Color32 = egui::Color32::from_rgb(0x0F, 0x6C, 0xBD);

// Codepoints aus "Segoe Fluent Icons" bzw. "Segoe MDL2 Assets".
const ICON_FOLDER: u32 = 0xE8B7;
const ICON_FILE: u32 = 0xE7C3;
const ICON_OPEN: u32 = 0xE8E5;
const ICON_CREATE: u32 = 0xE710;
const ICON_ADD: u32 = 0xE8F4;
const ICON_EXTRACT: u32 = 0xE896;
const ICON_DELETE: u32 = 0xE74D;
const ICON_RENAME: u32 = 0xE8AC;
const ICON_TEST: u32 = 0xE73E;
const ICON_PROPERTIES: u32 = 0xE946;
const ICON_THEME: u32 = 0xE706;
const ICON_MOON: u32 = 0xE708;
const ICON_SEARCH: u32 = 0xE721;

const ICON_FAMILY: &str = "segoe_icons";

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
            let app = ArchiveApp {
                icons: Icons::install(&cc.egui_ctx),
                ..Default::default()
            };
            Ok(Box::new(app))
        }),
    )
}

/// Symbolschrift für Ribbon-Icons. Fällt auf Unicode-Zeichen zurück, wenn die
/// Windows-Icon-Schrift nicht vorhanden ist.
#[derive(Default)]
struct Icons {
    family: Option<egui::FontFamily>,
}

impl Icons {
    fn install(ctx: &egui::Context) -> Self {
        #[cfg(target_os = "windows")]
        {
            let fonts_dir = std::env::var_os("WINDIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(r"C:\Windows"))
                .join("Fonts");

            for file in ["SegoeIcons.ttf", "segmdl2.ttf"] {
                let Ok(bytes) = std::fs::read(fonts_dir.join(file)) else {
                    continue;
                };

                let mut definitions = egui::FontDefinitions::default();
                definitions.font_data.insert(
                    ICON_FAMILY.to_owned(),
                    Arc::new(egui::FontData::from_owned(bytes)),
                );
                definitions.families.insert(
                    egui::FontFamily::Name(ICON_FAMILY.into()),
                    vec![ICON_FAMILY.to_owned()],
                );
                ctx.set_fonts(definitions);
                return Self {
                    family: Some(egui::FontFamily::Name(ICON_FAMILY.into())),
                };
            }
        }

        let _ = ctx;
        Self::default()
    }

    fn glyph(&self, codepoint: u32, fallback: &'static str) -> String {
        if self.family.is_some() {
            char::from_u32(codepoint).unwrap_or(' ').to_string()
        } else {
            fallback.to_owned()
        }
    }

    fn family(&self) -> egui::FontFamily {
        self.family
            .clone()
            .unwrap_or(egui::FontFamily::Proportional)
    }

    fn font(&self, size: f32) -> egui::FontId {
        egui::FontId::new(size, self.family())
    }
}

#[derive(Debug, Clone, Copy)]
enum ClickKind {
    Single,
    Toggle,
    Range,
}

/// Sortierbare Spalten der Dateiliste.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum SortColumn {
    #[default]
    Name,
    Size,
    Compressed,
    Kind,
}

/// Aktionen aus dem Kontextmenü der Dateiliste.
#[derive(Debug, Clone, Copy)]
enum RowAction {
    Extract,
    Edit,
    Rename,
    Delete,
    Properties,
}

/// Meldung mit mehreren Details, z. B. Fehler pro Eintrag.
#[derive(Clone, Default)]
struct ReportWindow {
    title: String,
    summary: String,
    details: Vec<String>,
}

struct EditSession {
    archive_path: PathBuf,
    entry_name: String,
    temp_dir: PathBuf,
    file_path: PathBuf,
    original_size: u64,
    original_modified: Option<SystemTime>,
    error: Option<String>,
}

struct ArchiveApp {
    archive_path: Option<PathBuf>,
    entries: Vec<ArchiveEntry>,
    selection: BTreeSet<usize>,
    anchor: Option<usize>,
    status: String,
    search: String,
    dark_mode: bool,
    icons: Icons,
    confirm_delete: bool,
    show_properties: bool,
    report: Option<ReportWindow>,
    sort_column: SortColumn,
    sort_ascending: bool,
    rename_from: Option<String>,
    rename_to: String,
    rename_error: Option<String>,
    rename_focus: bool,
    edit_session: Option<EditSession>,
    confirm_close_edit: bool,
    allow_close: bool,
}

impl Default for ArchiveApp {
    fn default() -> Self {
        Self {
            archive_path: None,
            entries: Vec::new(),
            selection: BTreeSet::new(),
            anchor: None,
            status: String::new(),
            search: String::new(),
            dark_mode: false,
            icons: Icons::default(),
            confirm_delete: false,
            show_properties: false,
            report: None,
            sort_column: SortColumn::Name,
            sort_ascending: true,
            rename_from: None,
            rename_to: String::new(),
            rename_error: None,
            rename_focus: false,
            edit_session: None,
            confirm_close_edit: false,
            allow_close: false,
        }
    }
}

impl ArchiveApp {
    fn open_archive_path(&mut self, path: PathBuf) {
        if self
            .edit_session
            .as_ref()
            .is_some_and(|session| session.archive_path != path)
        {
            self.status = "Bitte zuerst die offene Bearbeitung übernehmen oder verwerfen.".into();
            return;
        }
        match list_entries(&path) {
            Ok(entries) => {
                self.archive_path = Some(path.clone());
                self.entries = entries;
                self.selection.clear();
                self.anchor = None;
                self.search.clear();
                self.status = format!(
                    "{} Einträge aus {} geladen",
                    self.entries.len(),
                    path.file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("Archiv")
                );
            }
            Err(error) => self.status = format!("Archiv konnte nicht geöffnet werden: {error}"),
        }
    }

    fn open_archive(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter(
                "Archive",
                &[
                    "zip", "7z", "tar", "gz", "tgz", "bz2", "tbz2", "xz", "txz", "zst", "tzst",
                    "lz4", "tlz4",
                ],
            )
            .pick_file()
        {
            self.open_archive_path(path);
        }
    }

    fn handle_dropped_files(&mut self, files: Vec<PathBuf>) {
        if files.is_empty() {
            return;
        }

        // Archive werden geöffnet; normale Dateien und Ordner werden in das
        // aktuell geöffnete ZIP übernommen. Die Erkennung erfolgt über Inhalt
        // und Endung, damit auch TAR- und TAR.GZ-Archive funktionieren.
        let archives: Vec<_> = files
            .iter()
            .filter(|path| ArchiveFormat::detect(path).is_ok())
            .cloned()
            .collect();
        if !archives.is_empty() {
            if files.len() != 1 {
                self.status = "Bitte Archive einzeln ablegen.".into();
            } else if archives.len() == 1 {
                self.open_archive_path(archives[0].clone());
            }
            return;
        }

        if self.can_update_zip() {
            self.apply_add(files);
        }
    }

    fn create_archive(&mut self) {
        if self.edit_session.is_some() {
            self.status = "Bitte zuerst die offene Bearbeitung übernehmen oder verwerfen.".into();
            return;
        }
        let Some(inputs) = rfd::FileDialog::new().pick_files() else {
            return;
        };
        if inputs.is_empty() {
            return;
        }

        let Some(output) = rfd::FileDialog::new()
            .add_filter("ZIP-Archiv", &["zip"])
            .add_filter("7z-Archiv", &["7z"])
            .set_file_name("archiv.zip")
            .save_file()
        else {
            return;
        };

        let creation = match output.extension().and_then(|extension| extension.to_str()) {
            Some(extension) if extension.eq_ignore_ascii_case("7z") => create_7z(&output, &inputs),
            Some(extension) if extension.eq_ignore_ascii_case("zip") => {
                create_zip(&output, &inputs)
            }
            _ => {
                self.status = "Bitte als .zip- oder .7z-Datei speichern.".into();
                return;
            }
        };

        match creation {
            Ok(()) => match list_entries(&output) {
                Ok(entries) => {
                    self.archive_path = Some(output.clone());
                    self.entries = entries;
                    self.selection.clear();
                    self.anchor = None;
                    self.search.clear();
                    self.status = format!("Archiv erstellt: {}", output.display());
                }
                Err(error) => {
                    self.status =
                        format!("Archiv erstellt, konnte aber nicht geöffnet werden: {error}")
                }
            },
            Err(error) => self.status = format!("Archiv konnte nicht erstellt werden: {error}"),
        }
    }

    /// Lädt die Einträge des aktuellen Archivs neu, ohne die Auswahl zu verlieren.
    fn reload(&mut self) {
        let Some(path) = self.archive_path.clone() else {
            return;
        };
        match list_entries(&path) {
            Ok(entries) => {
                self.entries = entries;
                self.selection.retain(|index| *index < self.entries.len());
                if self.entries.is_empty() {
                    self.selection.clear();
                    self.anchor = None;
                }
            }
            Err(error) => {
                self.status = format!("Archiv konnte nicht neu geladen werden: {error}");
            }
        }
    }

    fn selected_names(&self) -> Vec<String> {
        self.selection
            .iter()
            .filter_map(|index| self.entries.get(*index))
            .map(|entry| entry.name.clone())
            .collect()
    }

    fn primary_selection(&self) -> Option<usize> {
        self.anchor
            .filter(|index| self.selection.contains(index))
            .or_else(|| self.selection.iter().next().copied())
    }

    fn apply_click(&mut self, index: usize, kind: ClickKind) {
        match kind {
            ClickKind::Single => {
                self.selection.clear();
                self.selection.insert(index);
                self.anchor = Some(index);
            }
            ClickKind::Toggle => {
                if !self.selection.remove(&index) {
                    self.selection.insert(index);
                }
                self.anchor = Some(index);
            }
            ClickKind::Range => {
                // Bereiche beziehen sich auf die sichtbare Reihenfolge, damit
                // die Auswahl auch nach dem Sortieren oder Filtern stimmt.
                let visible: Vec<usize> = self
                    .visible_entries()
                    .into_iter()
                    .map(|(entry_index, _)| entry_index)
                    .collect();
                let anchor = self.anchor.unwrap_or(index);
                let anchor_pos = visible.iter().position(|item| *item == anchor).unwrap_or(0);
                let index_pos = visible
                    .iter()
                    .position(|item| *item == index)
                    .unwrap_or(anchor_pos);
                let (start, end) = if anchor_pos <= index_pos {
                    (anchor_pos, index_pos)
                } else {
                    (index_pos, anchor_pos)
                };
                self.selection.clear();
                for item in &visible[start..=end] {
                    self.selection.insert(*item);
                }
            }
        }
    }

    fn extract_selected(&mut self) {
        let Some(archive) = self.archive_path.clone() else {
            self.status = "Bitte zuerst ein Archiv öffnen.".into();
            return;
        };
        let names = self.selected_names();
        let Some(destination) = rfd::FileDialog::new().pick_folder() else {
            return;
        };

        let result = if names.is_empty() {
            extract_all(&archive, &destination)
        } else {
            extract_selection(&archive, &names, &destination)
        };

        match result {
            Ok(report) => self.finish_operation(report, "Extrahiert"),
            Err(error) => self.status = format!("Extraktion fehlgeschlagen: {error}"),
        }
    }

    fn add_files(&mut self) {
        if !self.can_update_zip() {
            return;
        }
        let Some(inputs) = rfd::FileDialog::new().pick_files() else {
            return;
        };
        self.apply_add(inputs);
    }

    fn add_folder(&mut self) {
        if !self.can_update_zip() {
            return;
        }
        let Some(directory) = rfd::FileDialog::new().pick_folder() else {
            return;
        };
        self.apply_add(vec![directory]);
    }

    fn can_update_zip(&mut self) -> bool {
        if self.edit_session.is_some() {
            self.status = "Bitte zuerst die offene Bearbeitung übernehmen oder verwerfen.".into();
            return false;
        }
        let Some(archive) = self.archive_path.clone() else {
            self.status = "Bitte zuerst ein Archiv öffnen.".into();
            return false;
        };
        match ArchiveFormat::detect(&archive) {
            Ok(ArchiveFormat::Zip) => true,
            Ok(format) => {
                self.status = format!("Hinzufügen für {format} wird noch nicht unterstützt.");
                false
            }
            Err(error) => {
                self.status = format!("Archivformat konnte nicht erkannt werden: {error}");
                false
            }
        }
    }

    fn apply_add(&mut self, inputs: Vec<PathBuf>) {
        let Some(archive) = self.archive_path.clone() else {
            return;
        };
        if inputs.is_empty() {
            return;
        }

        match add_to_zip(&archive, &inputs) {
            Ok(report) => {
                self.reload();
                self.selection.clear();
                self.anchor = None;
                self.status = format!(
                    "Hinzugefügt: {} · ersetzt: {}",
                    report.added, report.replaced
                );
            }
            Err(error) => self.status = format!("Hinzufügen fehlgeschlagen: {error}"),
        }
    }

    fn request_delete(&mut self) {
        if self.selection.is_empty() {
            self.status = "Bitte zuerst Einträge auswählen.".into();
            return;
        }
        self.confirm_delete = true;
    }

    fn perform_delete(&mut self) {
        if self.edit_session.is_some() {
            self.status = "Bitte zuerst die offene Bearbeitung übernehmen oder verwerfen.".into();
            return;
        }
        let Some(archive) = self.archive_path.clone() else {
            return;
        };
        let names = self.selected_names();
        if names.is_empty() {
            return;
        }

        match remove_entries_from_zip(&archive, &names) {
            Ok(report) => {
                self.reload();
                self.selection.clear();
                self.anchor = None;
                self.status = format!("Entfernt: {} Einträge", report.removed);
            }
            Err(error) => self.status = format!("Löschen fehlgeschlagen: {error}"),
        }
    }

    fn run_test(&mut self) {
        let Some(archive) = self.archive_path.clone() else {
            self.status = "Bitte zuerst ein Archiv öffnen.".into();
            return;
        };
        match test_archive(&archive) {
            Ok(report) => self.finish_operation(report, "Geprüft"),
            Err(error) => self.status = format!("Archivprüfung fehlgeschlagen: {error}"),
        }
    }

    fn finish_operation(&mut self, report: OperationReport, verb: &str) {
        if report.is_success() {
            self.status = format!("{verb}: {} Einträge", report.processed);
            return;
        }

        self.status = format!(
            "{verb}: {} von {} Einträgen fehlgeschlagen",
            report.failures.len(),
            report.processed + report.failures.len()
        );
        self.report = Some(ReportWindow {
            title: format!("{verb} – Fehler"),
            summary: self.status.clone(),
            details: report
                .failures
                .iter()
                .map(|failure| format!("{}: {}", failure.name, failure.message))
                .collect(),
        });
    }

    fn visible_entries(&self) -> Vec<(usize, &ArchiveEntry)> {
        let query = self.search.trim().to_lowercase();
        let mut items: Vec<(usize, &ArchiveEntry)> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| query.is_empty() || entry.name.to_lowercase().contains(&query))
            .collect();

        items.sort_by(|(_, left), (_, right)| {
            let ordering = match self.sort_column {
                SortColumn::Name => left.name.to_lowercase().cmp(&right.name.to_lowercase()),
                SortColumn::Size => left.size.cmp(&right.size),
                SortColumn::Compressed => left
                    .compressed_size
                    .unwrap_or(0)
                    .cmp(&right.compressed_size.unwrap_or(0)),
                SortColumn::Kind => left
                    .is_directory
                    .cmp(&right.is_directory)
                    .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase())),
            };
            if self.sort_ascending {
                ordering
            } else {
                ordering.reverse()
            }
        });

        items
    }

    fn begin_rename(&mut self) {
        if self.selection.is_empty() {
            self.status = "Bitte zuerst einen Eintrag auswählen.".into();
            return;
        }
        if self.selection.len() > 1 {
            self.status = "Bitte genau einen Eintrag zum Umbenennen auswählen.".into();
            return;
        }
        let Some(path) = self.archive_path.clone() else {
            return;
        };
        match ArchiveFormat::detect(&path) {
            Ok(ArchiveFormat::Zip) => {}
            Ok(format) => {
                self.status = format!("Umbenennen für {format} wird noch nicht unterstützt.");
                return;
            }
            Err(error) => {
                self.status = format!("Archivformat konnte nicht erkannt werden: {error}");
                return;
            }
        }
        let Some(index) = self.primary_selection() else {
            return;
        };
        let Some(entry) = self.entries.get(index) else {
            return;
        };
        self.rename_from = Some(entry.name.clone());
        self.rename_to = entry.name.trim_end_matches('/').to_string();
        self.rename_error = None;
        self.rename_focus = true;
    }

    fn begin_external_edit(&mut self) {
        if self.edit_session.is_some() {
            self.status = "Bitte zuerst die offene Bearbeitung übernehmen oder verwerfen.".into();
            return;
        }
        if self.selection.len() != 1 {
            self.status = "Bitte genau eine Datei zum Bearbeiten auswählen.".into();
            return;
        }
        let Some(archive) = self.archive_path.clone() else {
            self.status = "Bitte zuerst ein Archiv öffnen.".into();
            return;
        };
        match ArchiveFormat::detect(&archive) {
            Ok(ArchiveFormat::Zip) => {}
            Ok(format) => {
                self.status =
                    format!("Externe Bearbeitung für {format} wird noch nicht unterstützt.");
                return;
            }
            Err(error) => {
                self.status = format!("Archivformat konnte nicht erkannt werden: {error}");
                return;
            }
        }
        let Some(entry) = self
            .primary_selection()
            .and_then(|index| self.entries.get(index))
        else {
            return;
        };
        if entry.is_directory {
            self.status = "Ordner können nicht direkt extern bearbeitet werden.".into();
            return;
        }
        let entry_name = entry.name.clone();

        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let temp_dir =
            std::env::temp_dir().join(format!("archivetool-edit-{}-{unique}", std::process::id()));
        let result = (|| -> Result<PathBuf, String> {
            std::fs::create_dir_all(&temp_dir).map_err(|error| error.to_string())?;
            extract_entry(&archive, &entry_name, &temp_dir).map_err(|error| error.to_string())?;
            let relative = PathBuf::from(entry_name.replace('\\', "/"));
            let file_path = temp_dir.join(relative);
            if !file_path.is_file() {
                return Err("Der extrahierte Eintrag ist keine Datei.".into());
            }
            open_with_default_editor(&file_path).map_err(|error| error.to_string())?;
            Ok(file_path)
        })();

        match result {
            Ok(file_path) => {
                let metadata = std::fs::metadata(&file_path).ok();
                self.edit_session = Some(EditSession {
                    archive_path: archive,
                    entry_name: entry_name.clone(),
                    temp_dir,
                    file_path,
                    original_size: metadata.as_ref().map_or(0, |metadata| metadata.len()),
                    original_modified: metadata.and_then(|metadata| metadata.modified().ok()),
                    error: None,
                });
                self.status = format!("Extern zum Bearbeiten geöffnet: {entry_name}");
            }
            Err(error) => {
                self.status = format!("Datei konnte nicht extern geöffnet werden: {error}");
                // Bei einem fehlgeschlagenen Editorstart bleiben temporäre
                // Dateien bewusst erhalten, damit keine Daten verloren gehen.
            }
        }
    }

    fn apply_external_edit(&mut self) {
        let Some(session) = self.edit_session.as_ref() else {
            return;
        };
        match replace_entry_in_zip(
            &session.archive_path,
            &session.entry_name,
            &session.file_path,
        ) {
            Ok(_) => {
                let entry_name = session.entry_name.clone();
                let temp_dir = session.temp_dir.clone();
                self.edit_session = None;
                let _ = std::fs::remove_dir_all(temp_dir);
                self.reload();
                self.status = format!("Änderung übernommen: {entry_name}");
            }
            Err(error) => {
                if let Some(session) = self.edit_session.as_mut() {
                    session.error = Some(error.to_string());
                }
            }
        }
    }

    fn discard_external_edit(&mut self) {
        if let Some(session) = self.edit_session.take() {
            let _ = std::fs::remove_dir_all(session.temp_dir);
            self.status = format!("Temporäre Änderung verworfen: {}", session.entry_name);
        }
    }

    fn perform_rename(&mut self) {
        if self.edit_session.is_some() {
            self.rename_error =
                Some("Bitte zuerst die offene Bearbeitung übernehmen oder verwerfen.".into());
            return;
        }
        let Some(archive) = self.archive_path.clone() else {
            return;
        };
        let Some(from) = self.rename_from.clone() else {
            return;
        };
        let target = self.rename_to.trim().trim_end_matches('/').to_string();
        if target.is_empty() {
            self.rename_error = Some("Bitte einen neuen Namen angeben.".into());
            return;
        }
        if target == from.trim_end_matches('/') {
            self.rename_from = None;
            return;
        }

        match rename_entry_in_zip(&archive, &from, &target) {
            Ok(report) => {
                self.reload();
                self.selection.clear();
                self.anchor = None;
                self.rename_from = None;
                self.rename_error = None;
                self.status = format!("Umbenannt: {} Einträge", report.renamed);
            }
            Err(error) => self.rename_error = Some(error.to_string()),
        }
    }

    fn total_size(&self) -> u64 {
        self.entries.iter().map(|entry| entry.size).sum()
    }

    fn selection_size(&self) -> u64 {
        self.selection
            .iter()
            .filter_map(|index| self.entries.get(*index))
            .map(|entry| entry.size)
            .sum()
    }

    /// Dateiname des geöffneten Archivs.
    fn archive_name(&self) -> Option<&str> {
        self.archive_path
            .as_ref()
            .and_then(|path| path.file_name())
            .and_then(|name| name.to_str())
    }

    fn ribbon(&mut self, ctx: &egui::Context) {
        let (background, border, muted) = if self.dark_mode {
            (
                egui::Color32::from_rgb(0x23, 0x25, 0x2A),
                egui::Color32::from_rgb(0x36, 0x39, 0x40),
                egui::Color32::from_rgb(0x9A, 0xA0, 0xAB),
            )
        } else {
            (
                egui::Color32::from_rgb(0xF7, 0xF8, 0xFA),
                egui::Color32::from_rgb(0xE1, 0xE4, 0xEA),
                egui::Color32::from_rgb(0x5E, 0x66, 0x72),
            )
        };

        egui::TopBottomPanel::top("ribbon")
            .frame(
                egui::Frame::new()
                    .fill(background)
                    .inner_margin(egui::Margin {
                        left: 14,
                        right: 14,
                        top: 10,
                        bottom: 8,
                    })
                    .stroke(egui::Stroke::new(1.0_f32, border)),
            )
            .show(ctx, |ui| {
                // Flaches Ribbon: Schaltflächen bleiben bis zum Hover transparent.
                ui.visuals_mut().widgets.inactive.weak_bg_fill = egui::Color32::TRANSPARENT;
                ui.visuals_mut().widgets.inactive.bg_stroke = egui::Stroke::NONE;
                ui.visuals_mut().widgets.hovered.weak_bg_fill = ACCENT.gamma_multiply(0.16);
                ui.visuals_mut().widgets.hovered.bg_stroke = egui::Stroke::NONE;
                ui.visuals_mut().widgets.active.weak_bg_fill = ACCENT.gamma_multiply(0.24);

                ui.horizontal(|ui| {
                    let (logo, _) =
                        ui.allocate_exact_size(egui::vec2(34.0, 34.0), egui::Sense::hover());
                    ui.painter()
                        .rect_filled(logo, egui::CornerRadius::same(9), ACCENT);
                    ui.painter().text(
                        logo.center(),
                        egui::Align2::CENTER_CENTER,
                        self.icons.glyph(ICON_FOLDER, "▦"),
                        self.icons.font(19.0),
                        egui::Color32::WHITE,
                    );

                    ui.add_space(10.0);
                    ui.vertical(|ui| {
                        ui.label(egui::RichText::new("Archive").size(16.0).strong());
                        ui.label(
                            egui::RichText::new(
                                self.archive_name().unwrap_or("Kein Archiv geöffnet"),
                            )
                            .size(10.5)
                            .color(muted),
                        );
                    });

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let (theme_icon, theme_fallback) = if self.dark_mode {
                            (ICON_MOON, "☾")
                        } else {
                            (ICON_THEME, "☀")
                        };
                        if icon_button(
                            ui,
                            &self.icons,
                            theme_icon,
                            theme_fallback,
                            "Darstellung wechseln",
                        )
                        .clicked()
                        {
                            self.dark_mode = !self.dark_mode;
                        }
                        ui.add_space(6.0);
                        ui.add(
                            egui::TextEdit::singleline(&mut self.search)
                                .hint_text("Suchen …")
                                .desired_width(240.0),
                        );
                        ui.add_space(2.0);
                        ui.label(
                            egui::RichText::new(self.icons.glyph(ICON_SEARCH, "⌕"))
                                .font(self.icons.font(13.0))
                                .color(muted),
                        );
                    });
                });

                ui.add_space(4.0);

                let archive_open = self.archive_path.is_some();
                let has_selection = !self.selection.is_empty();
                let single_selection = self.selection.len() == 1;

                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ribbon_group(ui, "Datei", |ui| {
                        if ribbon_button(ui, &self.icons, ICON_OPEN, "📂", "Öffnen", true).clicked()
                        {
                            self.open_archive();
                        }
                        if ribbon_button(ui, &self.icons, ICON_CREATE, "＋", "Erstellen", true)
                            .clicked()
                        {
                            self.create_archive();
                        }
                    });

                    ribbon_divider(ui);

                    ribbon_group(ui, "Bearbeiten", |ui| {
                        if ribbon_button(ui, &self.icons, ICON_FILE, "📄", "Dateien", archive_open)
                            .clicked()
                        {
                            self.add_files();
                        }
                        if ribbon_button(ui, &self.icons, ICON_ADD, "＋", "Ordner", archive_open)
                            .clicked()
                        {
                            self.add_folder();
                        }
                        if ribbon_button(
                            ui,
                            &self.icons,
                            ICON_EXTRACT,
                            "⬇",
                            "Entpacken",
                            archive_open,
                        )
                        .clicked()
                        {
                            self.extract_selected();
                        }
                        if ribbon_button(
                            ui,
                            &self.icons,
                            ICON_FILE,
                            "✎",
                            "Extern bearbeiten",
                            single_selection,
                        )
                        .clicked()
                        {
                            self.begin_external_edit();
                        }
                        if ribbon_button(
                            ui,
                            &self.icons,
                            ICON_RENAME,
                            "✎",
                            "Umbenennen",
                            single_selection,
                        )
                        .clicked()
                        {
                            self.begin_rename();
                        }
                        if ribbon_button(
                            ui,
                            &self.icons,
                            ICON_DELETE,
                            "🗑",
                            "Löschen",
                            has_selection,
                        )
                        .clicked()
                        {
                            self.request_delete();
                        }
                    });

                    ribbon_divider(ui);

                    ribbon_group(ui, "Prüfen", |ui| {
                        if ribbon_button(ui, &self.icons, ICON_TEST, "✓", "Testen", archive_open)
                            .clicked()
                        {
                            self.run_test();
                        }
                        if ribbon_button(
                            ui,
                            &self.icons,
                            ICON_PROPERTIES,
                            "ℹ",
                            "Eigenschaften",
                            archive_open,
                        )
                        .clicked()
                        {
                            self.show_properties = true;
                        }
                    });

                    ribbon_divider(ui);

                    ribbon_group(ui, "Ansicht", |ui| {
                        let (theme_icon, theme_fallback) = if self.dark_mode {
                            (ICON_MOON, "☾")
                        } else {
                            (ICON_THEME, "☀")
                        };
                        if ribbon_button(
                            ui,
                            &self.icons,
                            theme_icon,
                            theme_fallback,
                            "Design",
                            true,
                        )
                        .clicked()
                        {
                            self.dark_mode = !self.dark_mode;
                        }
                    });
                });
            });
    }

    fn sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("navigation")
            .resizable(false)
            .exact_width(205.0)
            .show(ctx, |ui| {
                ui.add_space(18.0);
                ui.label(
                    egui::RichText::new("ARBEITSBEREICH")
                        .size(11.0)
                        .weak()
                        .strong(),
                );
                ui.add_space(8.0);
                let _ = nav_item(ui, &self.icons, ICON_FOLDER, "▣", "Archiv", true);
                ui.add_space(22.0);
                ui.label(
                    egui::RichText::new("ARCHIVINFORMATIONEN")
                        .size(11.0)
                        .weak()
                        .strong(),
                );
                ui.add_space(10.0);

                if let Some(name) = self.archive_name() {
                    ui.label(egui::RichText::new("DATEI").size(10.0).weak());
                    ui.label(name);
                    ui.add_space(12.0);
                    ui.label(egui::RichText::new("INHALT").size(10.0).weak());
                    ui.label(format!("{} Einträge", self.entries.len()));
                    ui.add_space(10.0);
                    ui.label(egui::RichText::new("GESAMTGRÖSSE").size(10.0).weak());
                    ui.label(format_bytes(self.total_size()));

                    if !self.selection.is_empty() {
                        ui.add_space(12.0);
                        ui.label(egui::RichText::new("AUSWAHL").size(10.0).weak());
                        ui.label(format!(
                            "{} Einträge · {}",
                            self.selection.len(),
                            format_bytes(self.selection_size())
                        ));
                    }
                } else {
                    ui.label("Noch kein Archiv geöffnet.");
                }

                ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                    ui.separator();
                    ui.label(egui::RichText::new("ArchiveTool 0.2.0").weak().small());
                });
            });
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
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
                        if self.selection.is_empty() {
                            ui.label(format!("{} Elemente", self.entries.len()));
                        } else {
                            ui.label(format!(
                                "{} von {} ausgewählt",
                                self.selection.len(),
                                self.entries.len()
                            ));
                        }
                        ui.add_space(8.0);
                    });
                });
            });
    }

    fn content(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(12.0);
            ui.horizontal(|ui| {
                ui.heading("Archivinhalt");
                if let Some(name) = self.archive_name() {
                    ui.label(egui::RichText::new(name).weak());
                }
            });
            ui.add_space(8.0);

            if self.archive_path.is_none() {
                ui.add_space(60.0);
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new(self.icons.glyph(ICON_FOLDER, "📂"))
                            .font(self.icons.font(46.0))
                            .color(ACCENT.gamma_multiply(0.85)),
                    );
                    ui.add_space(6.0);
                    ui.heading(egui::RichText::new("Kein Archiv geöffnet").size(24.0));
                    ui.add_space(6.0);
                    ui.label("Öffne ein ZIP-, 7z-, TAR- oder TAR.GZ-Archiv, um dessen Inhalt anzuzeigen.");
                    ui.add_space(18.0);
                    if inline_button(ui, &self.icons, ICON_OPEN, "📂", "Archiv öffnen").clicked()
                    {
                        self.open_archive();
                    }
                    ui.add_space(6.0);
                    if inline_button(ui, &self.icons, ICON_CREATE, "＋", "ZIP erstellen").clicked()
                    {
                        self.create_archive();
                    }
                });
                return;
            }

            egui::Frame::group(ui.style())
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    egui::ScrollArea::both()
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            egui::Grid::new("archive_entries")
                                .num_columns(4)
                                .striped(true)
                                .min_col_width(70.0)
                                .show(ui, |ui| {
                                    let mut sort_click = None;
                                    let mut clicked = None;
                                    let mut row_action = None;

                                    if sort_header(
                                        ui,
                                        "Name",
                                        SortColumn::Name,
                                        self.sort_column,
                                        self.sort_ascending,
                                    ) {
                                        sort_click = Some(SortColumn::Name);
                                    }
                                    if sort_header(
                                        ui,
                                        "Größe",
                                        SortColumn::Size,
                                        self.sort_column,
                                        self.sort_ascending,
                                    ) {
                                        sort_click = Some(SortColumn::Size);
                                    }
                                    if sort_header(
                                        ui,
                                        "Komprimiert",
                                        SortColumn::Compressed,
                                        self.sort_column,
                                        self.sort_ascending,
                                    ) {
                                        sort_click = Some(SortColumn::Compressed);
                                    }
                                    if sort_header(
                                        ui,
                                        "Typ",
                                        SortColumn::Kind,
                                        self.sort_column,
                                        self.sort_ascending,
                                    ) {
                                        sort_click = Some(SortColumn::Kind);
                                    }
                                    ui.end_row();

                                    for (index, entry) in self.visible_entries() {
                                        let selected = self.selection.contains(&index);
                                        let (icon, fallback) = if entry.is_directory {
                                            (ICON_FOLDER, "📁")
                                        } else {
                                            (ICON_FILE, "📄")
                                        };

                                        let mut job = egui::text::LayoutJob::default();
                                        job.append(
                                            &self.icons.glyph(icon, fallback),
                                            0.0,
                                            egui::TextFormat {
                                                font_id: self.icons.font(15.0),
                                                color: if entry.is_directory {
                                                    ACCENT
                                                } else {
                                                    ui.visuals().weak_text_color()
                                                },
                                                valign: egui::Align::Center,
                                                ..Default::default()
                                            },
                                        );
                                        job.append(
                                            "   ",
                                            0.0,
                                            egui::TextFormat {
                                                font_id: egui::FontId::proportional(13.0),
                                                ..Default::default()
                                            },
                                        );
                                        job.append(
                                            &entry.name,
                                            0.0,
                                            egui::TextFormat {
                                                font_id: egui::FontId::proportional(13.0),
                                                color: ui.visuals().text_color(),
                                                valign: egui::Align::Center,
                                                ..Default::default()
                                            },
                                        );

                                        let response = ui
                                            .selectable_label(selected, job)
                                            .on_hover_text(entry.name.clone());
                                        response.context_menu(|ui| {
                                            if ui.button("Entpacken").clicked() {
                                                row_action = Some((RowAction::Extract, index));
                                                ui.close();
                                            }
                                            if ui.button("Extern bearbeiten").clicked() {
                                                row_action = Some((RowAction::Edit, index));
                                                ui.close();
                                            }
                                            let rename_allowed =
                                                !selected || self.selection.len() == 1;
                                            if ui
                                                .add_enabled(
                                                    rename_allowed,
                                                    egui::Button::new("Umbenennen"),
                                                )
                                                .clicked()
                                            {
                                                row_action = Some((RowAction::Rename, index));
                                                ui.close();
                                            }
                                            if ui.button("Löschen").clicked() {
                                                row_action = Some((RowAction::Delete, index));
                                                ui.close();
                                            }
                                            ui.separator();
                                            if ui.button("Eigenschaften").clicked() {
                                                row_action = Some((RowAction::Properties, index));
                                                ui.close();
                                            }
                                        });
                                        if response.clicked() {
                                            let (shift, toggle) = ui.input(|input| {
                                                (
                                                    input.modifiers.shift,
                                                    input.modifiers.command || input.modifiers.ctrl,
                                                )
                                            });
                                            let kind = if shift {
                                                ClickKind::Range
                                            } else if toggle {
                                                ClickKind::Toggle
                                            } else {
                                                ClickKind::Single
                                            };
                                            clicked = Some((index, kind));
                                        }
                                        ui.label(format_bytes(entry.size));
                                        ui.label(
                                            entry
                                                .compressed_size
                                                .map(format_bytes)
                                                .unwrap_or_else(|| "–".to_string()),
                                        );
                                        ui.label(if entry.is_directory {
                                            "Ordner"
                                        } else {
                                            "Datei"
                                        });
                                        ui.end_row();
                                    }

                                    if let Some(column) = sort_click {
                                        if self.sort_column == column {
                                            self.sort_ascending = !self.sort_ascending;
                                        } else {
                                            self.sort_column = column;
                                            self.sort_ascending = true;
                                        }
                                    }
                                    if let Some((index, kind)) = clicked {
                                        self.apply_click(index, kind);
                                    }
                                    if let Some((action, index)) = row_action {
                                        if !self.selection.contains(&index) {
                                            self.apply_click(index, ClickKind::Single);
                                        }
                                        match action {
                                            RowAction::Extract => self.extract_selected(),
                                            RowAction::Edit => self.begin_external_edit(),
                                            RowAction::Rename => self.begin_rename(),
                                            RowAction::Delete => self.request_delete(),
                                            RowAction::Properties => self.show_properties = true,
                                        }
                                    }
                                });
                        });
                });

            if let Some(entry) = self
                .primary_selection()
                .and_then(|index| self.entries.get(index))
            {
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if self.selection.len() > 1 {
                        ui.label(egui::RichText::new("Auswahl:").weak());
                        ui.label(format!("{} Einträge", self.selection.len()));
                        ui.separator();
                    }
                    ui.label(egui::RichText::new("Zuletzt gewählt:").weak());
                    ui.label(&entry.name);
                });
            }
        });
    }

    fn delete_dialog(&mut self, ctx: &egui::Context) {
        if !self.confirm_delete {
            return;
        }

        let names = self.selected_names();
        let mut confirmed = false;
        let mut cancelled = false;

        egui::Window::new("Einträge löschen")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(format!(
                    "{} Einträge werden aus dem Archiv entfernt.",
                    names.len()
                ));
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .max_height(180.0)
                    .show(ui, |ui| {
                        for name in &names {
                            ui.label(egui::RichText::new(name).small());
                        }
                    });
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(
                        "Das Archiv wird neu aufgebaut. Das Original bleibt erhalten, bis die neue Datei geprüft wurde.",
                    )
                    .weak()
                    .small(),
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let delete = egui::Button::new(
                        egui::RichText::new("Löschen").color(egui::Color32::WHITE),
                    )
                    .fill(egui::Color32::from_rgb(0xC4, 0x2B, 0x1C));
                    if ui.add(delete).clicked() {
                        confirmed = true;
                    }
                    if ui.button("Abbrechen").clicked() {
                        cancelled = true;
                    }
                });
            });

        if confirmed {
            self.confirm_delete = false;
            self.perform_delete();
        } else if cancelled {
            self.confirm_delete = false;
        }
    }

    fn rename_dialog(&mut self, ctx: &egui::Context) {
        let Some(from) = self.rename_from.clone() else {
            return;
        };

        let mut confirmed = false;
        let mut cancelled = false;

        egui::Window::new("Eintrag umbenennen")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(egui::RichText::new("Bisher").weak().small());
                ui.label(&from);
                ui.add_space(8.0);
                ui.label(egui::RichText::new("Neuer Name").weak().small());
                let response = ui.add(
                    egui::TextEdit::singleline(&mut self.rename_to)
                        .desired_width(360.0)
                        .hint_text("Name im Archiv"),
                );
                if std::mem::take(&mut self.rename_focus) {
                    response.request_focus();
                }
                if response.lost_focus() && ui.input(|input| input.key_pressed(egui::Key::Enter)) {
                    confirmed = true;
                }
                if let Some(error) = &self.rename_error {
                    ui.add_space(4.0);
                    ui.label(
                        egui::RichText::new(error)
                            .color(egui::Color32::from_rgb(0xC4, 0x2B, 0x1C))
                            .small(),
                    );
                }
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(
                        "Verzeichnisse werden mit allen enthaltenen Einträgen umbenannt. Das Archiv wird neu aufgebaut und vor dem Ersetzen geprüft.",
                    )
                    .weak()
                    .small(),
                );
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    let rename = egui::Button::new(
                        egui::RichText::new("Umbenennen").color(egui::Color32::WHITE),
                    )
                    .fill(ACCENT);
                    if ui.add(rename).clicked() {
                        confirmed = true;
                    }
                    if ui.button("Abbrechen").clicked() {
                        cancelled = true;
                    }
                });
            });

        if confirmed {
            self.perform_rename();
        } else if cancelled {
            self.rename_from = None;
            self.rename_error = None;
        }
    }

    fn properties_window(&mut self, ctx: &egui::Context) {
        if !self.show_properties {
            return;
        }

        let mut rows: Vec<(String, String)> = Vec::new();
        if let Some(path) = &self.archive_path {
            rows.push(("Pfad".into(), path.display().to_string()));
            if let Ok(metadata) = std::fs::metadata(path) {
                rows.push(("Dateigröße".into(), format_bytes(metadata.len())));
            }
            if let Ok(format) = ArchiveFormat::detect(path) {
                rows.push(("Format".into(), format.to_string()));
            }
        }
        rows.push(("Einträge".into(), self.entries.len().to_string()));
        rows.push(("Gesamtgröße".into(), format_bytes(self.total_size())));
        let compressed: u64 = self
            .entries
            .iter()
            .filter_map(|entry| entry.compressed_size)
            .sum();
        let has_compressed = self
            .entries
            .iter()
            .any(|entry| entry.compressed_size.is_some());
        rows.push((
            "Komprimiert".into(),
            if has_compressed {
                format_bytes(compressed)
            } else {
                "–".to_string()
            },
        ));
        rows.push(("Auswahl".into(), self.selection.len().to_string()));

        let selected = self
            .primary_selection()
            .and_then(|index| self.entries.get(index))
            .cloned();

        let mut close = false;
        egui::Window::new("Eigenschaften")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                egui::Grid::new("properties_grid")
                    .num_columns(2)
                    .spacing([18.0, 6.0])
                    .show(ui, |ui| {
                        for (label, value) in &rows {
                            ui.label(egui::RichText::new(label).weak());
                            ui.label(value);
                            ui.end_row();
                        }
                    });

                if let Some(entry) = &selected {
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new("GEWÄHLTER EINTRAG").size(10.0).weak());
                    ui.add_space(4.0);
                    ui.label(&entry.name);
                    ui.label(format!(
                        "{} · {}",
                        if entry.is_directory {
                            "Ordner"
                        } else {
                            "Datei"
                        },
                        format_bytes(entry.size)
                    ));
                }

                ui.add_space(10.0);
                if ui.button("Schließen").clicked() {
                    close = true;
                }
            });

        if close {
            self.show_properties = false;
        }
    }

    fn report_window(&mut self, ctx: &egui::Context) {
        let Some(report) = self.report.clone() else {
            return;
        };

        let mut close = false;
        egui::Window::new(&report.title)
            .collapsible(false)
            .resizable(true)
            .show(ctx, |ui| {
                ui.label(&report.summary);
                ui.add_space(6.0);
                egui::ScrollArea::vertical()
                    .max_height(240.0)
                    .show(ui, |ui| {
                        for detail in &report.details {
                            ui.label(egui::RichText::new(detail).small());
                        }
                    });
                ui.add_space(10.0);
                if ui.button("Schließen").clicked() {
                    close = true;
                }
            });

        if close {
            self.report = None;
        }
    }

    fn edit_session_window(&mut self, ctx: &egui::Context) {
        let Some(session) = self.edit_session.as_ref() else {
            return;
        };
        let entry_name = session.entry_name.clone();
        let original_size = session.original_size;
        let original_modified = session.original_modified;
        let file_path = session.file_path.clone();
        let error = session.error.clone();
        let metadata = std::fs::metadata(&file_path).ok();
        let changed = metadata.as_ref().is_some_and(|metadata| {
            metadata.len() != original_size || metadata.modified().ok() != original_modified
        });

        let mut apply = false;
        let mut discard = false;
        egui::Window::new("Externe Bearbeitung")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(format!("Eintrag: {entry_name}"));
                ui.add_space(6.0);
                ui.label("Die Datei ist in deinem externen Editor geöffnet.");
                ui.label("Speichere dort und übernimm die Änderung anschließend bewusst.");
                ui.add_space(6.0);
                ui.label(if changed {
                    "Änderung erkannt und bereit zur Übernahme."
                } else {
                    "Noch keine Änderung erkannt. Nach dem Speichern hier erneut prüfen."
                });
                if let Some(error) = &error {
                    ui.add_space(4.0);
                    ui.colored_label(egui::Color32::from_rgb(0xC4, 0x2B, 0x1C), error);
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(changed, egui::Button::new("Änderung übernehmen"))
                        .clicked()
                    {
                        apply = true;
                    }
                    if ui.button("Änderung verwerfen").clicked() {
                        discard = true;
                    }
                });
            });

        if apply {
            self.apply_external_edit();
        } else if discard {
            self.discard_external_edit();
        }
    }

    fn close_edit_warning(&mut self, ctx: &egui::Context) {
        if !self.confirm_close_edit {
            return;
        }
        let temp_path = self
            .edit_session
            .as_ref()
            .map(|session| session.file_path.display().to_string())
            .unwrap_or_default();
        let mut close = false;
        let mut stay = false;
        egui::Window::new("Bearbeitung noch offen")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label("Die Datei wurde noch nicht ins Archiv übernommen.");
                ui.label("Die temporäre Datei bleibt erhalten:");
                ui.label(egui::RichText::new(&temp_path).small().monospace());
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button("Schließen und Datei behalten").clicked() {
                        close = true;
                    }
                    if ui.button("Weiter bearbeiten").clicked() {
                        stay = true;
                    }
                });
            });
        if close {
            self.confirm_close_edit = false;
            self.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        } else if stay {
            self.confirm_close_edit = false;
        }
    }
}

impl eframe::App for ArchiveApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let close_requested = ctx.input(|input| input.viewport().close_requested());
        if close_requested && self.edit_session.is_some() && !self.allow_close {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.confirm_close_edit = true;
        }
        let (dropped, hovering_files) = ctx.input(|input| {
            (
                input
                    .raw
                    .dropped_files
                    .iter()
                    .filter_map(|file| file.path.clone())
                    .collect::<Vec<_>>(),
                !input.raw.hovered_files.is_empty(),
            )
        });
        if !dropped.is_empty() {
            self.handle_dropped_files(dropped);
        }

        apply_theme(ctx, self.dark_mode);
        self.ribbon(ctx);
        self.sidebar(ctx);
        self.status_bar(ctx);
        self.content(ctx);
        self.delete_dialog(ctx);
        self.rename_dialog(ctx);
        self.properties_window(ctx);
        self.report_window(ctx);
        self.edit_session_window(ctx);
        self.close_edit_warning(ctx);

        if hovering_files {
            egui::Area::new(egui::Id::new("file_drop_overlay"))
                .order(egui::Order::Foreground)
                .fixed_pos(ctx.content_rect().center() - egui::vec2(190.0, 42.0))
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style())
                        .fill(ACCENT)
                        .inner_margin(egui::Margin::symmetric(20, 14))
                        .show(ui, |ui| {
                            ui.label(
                                egui::RichText::new(if self.archive_path.is_some() {
                                    "Dateien/Ordner hinzufügen · Archiv öffnen"
                                } else {
                                    "ZIP- oder 7z-Archiv öffnen"
                                })
                                .color(egui::Color32::WHITE)
                                .strong(),
                            );
                        });
                });
        }
    }
}

/// Sortierbare Spaltenüberschrift mit Richtungspfeil.
fn sort_header(
    ui: &mut egui::Ui,
    label: &str,
    column: SortColumn,
    current: SortColumn,
    ascending: bool,
) -> bool {
    let marker = if column == current {
        if ascending { " (auf)" } else { " (ab)" }
    } else {
        ""
    };
    ui.add(egui::Button::new(egui::RichText::new(format!("{label}{marker}")).strong()).frame(false))
        .on_hover_text("Nach dieser Spalte sortieren")
        .clicked()
}

fn open_with_default_editor(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut command = Command::new("explorer.exe");
        command.arg(path);
        command
    };
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut command = Command::new("open");
        command.arg(path);
        command
    };
    #[cfg(all(unix, not(target_os = "macos")))]
    let mut command = {
        let mut command = Command::new("xdg-open");
        command.arg(path);
        command
    };
    command.spawn().map(|_| ())
}

/// Setzt die Farben und Rundungen im Stil eines modernen Office-Themes.
fn apply_theme(ctx: &egui::Context, dark: bool) {
    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };

    visuals.panel_fill = if dark {
        egui::Color32::from_rgb(0x1B, 0x1D, 0x21)
    } else {
        egui::Color32::from_rgb(0xFA, 0xFB, 0xFC)
    };
    visuals.window_fill = if dark {
        egui::Color32::from_rgb(0x25, 0x27, 0x2C)
    } else {
        egui::Color32::from_rgb(0xFF, 0xFF, 0xFF)
    };
    visuals.extreme_bg_color = if dark {
        egui::Color32::from_rgb(0x2A, 0x2D, 0x33)
    } else {
        egui::Color32::from_rgb(0xFF, 0xFF, 0xFF)
    };
    visuals.selection.bg_fill = ACCENT.gamma_multiply(0.55);
    visuals.selection.stroke = egui::Stroke::new(1.0_f32, egui::Color32::WHITE);

    for widget in [
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = egui::CornerRadius::same(6);
    }

    ctx.set_visuals(visuals);

    ctx.style_mut(|style| {
        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.button_padding = egui::vec2(10.0, 5.0);
    });
}

/// Große Ribbon-Schaltfläche: Symbol über Beschriftung.
fn ribbon_button(
    ui: &mut egui::Ui,
    icons: &Icons,
    codepoint: u32,
    fallback: &'static str,
    label: &str,
    enabled: bool,
) -> egui::Response {
    let color = if enabled {
        ui.visuals().text_color()
    } else {
        ui.visuals().weak_text_color().gamma_multiply(0.6)
    };

    let mut job = egui::text::LayoutJob {
        halign: egui::Align::Center,
        ..Default::default()
    };
    job.append(
        &icons.glyph(codepoint, fallback),
        0.0,
        egui::TextFormat {
            font_id: icons.font(23.0),
            color,
            ..Default::default()
        },
    );
    job.append(
        "\n",
        0.0,
        egui::TextFormat {
            font_id: egui::FontId::proportional(4.0),
            color: egui::Color32::TRANSPARENT,
            ..Default::default()
        },
    );
    job.append(
        label,
        0.0,
        egui::TextFormat {
            font_id: egui::FontId::proportional(11.0),
            color,
            ..Default::default()
        },
    );

    ui.add_enabled(
        enabled,
        egui::Button::new(job)
            .min_size(egui::vec2(74.0, 58.0))
            .corner_radius(8),
    )
    .on_hover_text(label.to_owned())
}

/// Kleine quadratische Symbolschaltfläche.
fn icon_button(
    ui: &mut egui::Ui,
    icons: &Icons,
    codepoint: u32,
    fallback: &'static str,
    tooltip: &str,
) -> egui::Response {
    ui.add(
        egui::Button::new(
            egui::RichText::new(icons.glyph(codepoint, fallback)).font(icons.font(15.0)),
        )
        .min_size(egui::vec2(30.0, 30.0))
        .corner_radius(6),
    )
    .on_hover_text(tooltip)
}

/// Schaltfläche mit Symbol und Beschriftung nebeneinander.
fn inline_button(
    ui: &mut egui::Ui,
    icons: &Icons,
    codepoint: u32,
    fallback: &'static str,
    label: &str,
) -> egui::Response {
    let color = ui.visuals().text_color();
    let mut job = egui::text::LayoutJob::default();
    job.append(
        &icons.glyph(codepoint, fallback),
        0.0,
        egui::TextFormat {
            font_id: icons.font(15.0),
            color,
            valign: egui::Align::Center,
            ..Default::default()
        },
    );
    job.append(
        "   ",
        0.0,
        egui::TextFormat {
            font_id: egui::FontId::proportional(13.0),
            color,
            ..Default::default()
        },
    );
    job.append(
        label,
        0.0,
        egui::TextFormat {
            font_id: egui::FontId::proportional(13.0),
            color,
            valign: egui::Align::Center,
            ..Default::default()
        },
    );

    ui.add(
        egui::Button::new(job)
            .min_size(egui::vec2(180.0, 40.0))
            .corner_radius(7),
    )
}

/// Gruppiert Ribbon-Schaltflächen und zeigt die Gruppenbeschriftung darunter.
fn ribbon_group(ui: &mut egui::Ui, caption: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    ui.vertical(|ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            add_contents(ui);
        });
        ui.add_space(1.0);
        let width = ui.min_rect().width();
        ui.allocate_ui_with_layout(
            egui::vec2(width, 12.0),
            egui::Layout::top_down(egui::Align::Center),
            |ui| {
                ui.label(
                    egui::RichText::new(caption)
                        .size(10.0)
                        .color(ui.visuals().weak_text_color()),
                );
            },
        );
    });
}

/// Senkrechter Trenner zwischen Ribbon-Gruppen.
fn ribbon_divider(ui: &mut egui::Ui) {
    ui.add_space(4.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, 56.0), egui::Sense::hover());
    let stroke = egui::Stroke::new(1.0_f32, ui.visuals().widgets.noninteractive.bg_stroke.color);
    ui.painter()
        .line_segment([rect.center_top(), rect.center_bottom()], stroke);
    ui.add_space(4.0);
}

/// Navigationseintrag in der Seitenleiste.
fn nav_item(
    ui: &mut egui::Ui,
    icons: &Icons,
    codepoint: u32,
    fallback: &'static str,
    label: &str,
    active: bool,
) -> egui::Response {
    let mut job = egui::text::LayoutJob::default();
    job.append(
        &icons.glyph(codepoint, fallback),
        0.0,
        egui::TextFormat {
            font_id: icons.font(15.0),
            color: if active {
                ACCENT
            } else {
                ui.visuals().text_color()
            },
            valign: egui::Align::Center,
            ..Default::default()
        },
    );
    job.append(
        "   ",
        0.0,
        egui::TextFormat {
            font_id: egui::FontId::proportional(13.0),
            ..Default::default()
        },
    );
    job.append(
        label,
        0.0,
        egui::TextFormat {
            font_id: egui::FontId::proportional(13.0),
            color: ui.visuals().text_color(),
            valign: egui::Align::Center,
            ..Default::default()
        },
    );

    ui.selectable_label(active, job)
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
