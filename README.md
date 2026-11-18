# ArchiveTool

Ein modularer Archivmanager in Rust mit getrennten Anwendungen für CLI und GUI.

## Komponenten

- `archive-core`: gemeinsame Archivfunktionen
- `archive-cli`: Kommandozeilenprogramm
- `archive`: grafischer Archivmanager auf Basis von egui/eframe

## Aktueller Stand (0.2.0)

- ZIP: Archiv öffnen, Einträge auflisten, einzelne Dateien extrahieren, ZIP erstellen.
- 7z: Archiv öffnen, Einträge auflisten und einzelne Dateien extrahieren.
- GUI: Archiv öffnen, Inhalt anzeigen und ausgewählte Datei extrahieren.
- CLI: `list`, `extract`, `create`.

Die 7z-Kompression und das Aktualisieren bestehender Archive werden in der nächsten Ausbaustufe ergänzt. Nicht alle 7z-Operationen sind bereits implementiert.

## Voraussetzungen

- Rust stable mit Edition 2024
- Unter Linux gegebenenfalls Systempakete für die Fenster-/Grafikbibliotheken von eframe

## Bauen

```bash
cargo build --workspace --release
```

## CLI

```bash
cargo run -p archive-cli -- list beispiel.zip
cargo run -p archive-cli -- extract beispiel.zip --entry config/settings.ini --output ./output
cargo run -p archive-cli -- create neu.zip ./datei.txt ./ordner
```

## GUI

```bash
cargo run -p archive
```

## Sicherheitshinweis

Die Extraktion prüft Archivpfade gegen Path Traversal. Die erste Version ist ein Entwicklungsstand und sollte noch nicht als Ersatz für etablierte Archivmanager mit beliebigen nicht vertrauenswürdigen Archiven eingesetzt werden. Größenlimits, umfangreiche Fuzz-Tests und weitere Schutzmaßnahmen sind noch auszubauen.
