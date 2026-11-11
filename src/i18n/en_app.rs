//! English translations for app.

/// German UI string → English text.
pub const EN: &[(&str, &str)] = &[
    ("Strg+", "Ctrl+"),
    ("Umschalt+", "Shift+"),
    ("Mit Zotero verbunden als {name}.", "Connected to Zotero as {name}."),
    (
        "Mit Zotero verbunden als {name}. Dateien aus der Bibliothek des vorigen Kontos lassen sich damit nicht mehr synchronisieren.",
        "Connected to Zotero as {name}. Files from the previous account's library can no longer be synced.",
    ),
    ("Diese Datei ist keine Textdatei.", "This file is not a text file."),
    ("Konnte {rel} nicht öffnen: {e}", "Could not open {rel}: {e}"),
    ("Speichern fehlgeschlagen: {e}", "Saving failed: {e}"),
    ("Speicherort für das neue Projekt wählen", "Choose a Location for the New Project"),
    ("Ordner konnte nicht angelegt werden: {e}", "Could not create the folder: {e}"),
    ("PDF gespeichert: {p}", "PDF saved: {p}"),
    ("Export fehlgeschlagen: {e}", "Export failed: {e}"),
    ("{s} wurde geladen.", "{s} was loaded."),
    ("Das aktuelle Dokument als PDF exportieren", "Export the Current Document as PDF"),
    ("Ordner öffnen", "Open Folder"),
    ("Datei öffnen", "Open File"),
    (
        "Typisch {VERSION} ({RELEASE_DATE}) – Typst-Editor mit Live-Vorschau (Typst 0.15)",
        "Typisch {VERSION} ({RELEASE_DATE}) – Typst editor with live preview (Typst 0.15)",
    ),
    ("Dieser Ordner existiert nicht mehr.", "This folder no longer exists."),
    ("Template auswählen – Typisch", "Choose Template – Typisch"),
    (
        "Bitte einen Ordner oder eine .typ-Datei hierher ziehen.",
        "Drop a folder or .typ file here.",
    ),
];
