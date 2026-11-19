# ArchiveTool

Ein modularer Archivmanager in Rust mit getrennten Anwendungen für CLI und GUI.

## Komponenten

- `archive-core`: gemeinsame Archivfunktionen
- `archive-cli`: Kommandozeilenprogramm
- `archive`: grafischer Archivmanager auf Basis von egui/eframe

## Aktueller Stand (0.2.0)

- ZIP: Archiv öffnen, Einträge auflisten, einzelne Dateien, Auswahlen oder das
  gesamte Archiv extrahieren, ZIP erstellen, Dateien hinzufügen/ersetzen,
  Einträge umbenennen/löschen und Integrität testen.
- 7z: Archiv öffnen, Einträge auflisten, Dateien, Auswahlen oder das gesamte
  Archiv extrahieren und Integrität testen.
- GUI: Office-Ribbon mit Icons, Mehrfachauswahl (Strg/Umschalt),
  Kontextmenü, sortierbare Spalten, Entpacken, Hinzufügen, Umbenennen,
  Löschen mit Bestätigung, Testen und Eigenschaften.
- CLI: `list`, `extract`, `create`, `add`, `rename`, `delete`, `test`, `info`.

Die 7z-Erstellung und das Bearbeiten externer Dateien werden in den nächsten
Ausbaustufen ergänzt.

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
cargo run -p archive-cli -- extract beispiel.zip --output ./output
cargo run -p archive-cli -- create neu.zip ./datei.txt ./ordner
cargo run -p archive-cli -- add neu.zip ./weitere-datei.txt
cargo run -p archive-cli -- rename neu.zip ordner/alt.txt ordner/neu.txt
cargo run -p archive-cli -- delete neu.zip datei.txt
cargo run -p archive-cli -- test neu.zip
cargo run -p archive-cli -- info neu.zip
```

Mehrere Einträge lassen sich mit wiederholtem `--entry` auswählen. Ohne
`--entry` wird das gesamte Archiv entpackt.

## GUI

```bash
cargo run -p archive
```

## Sicherheitshinweis

Die Extraktion prüft Archivpfade gegen Path Traversal. Die erste Version ist ein Entwicklungsstand und sollte noch nicht als Ersatz für etablierte Archivmanager mit beliebigen nicht vertrauenswürdigen Archiven eingesetzt werden. Größenlimits, umfangreiche Fuzz-Tests und weitere Schutzmaßnahmen sind noch auszubauen.
