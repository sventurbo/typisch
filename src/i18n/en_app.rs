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
    // Saving, and files changed by another program (saving.rs)
    ("„{rel}“ konnte nicht gespeichert werden: {e}", "“{rel}” could not be saved: {e}"),
    (
        " Noch einmal schließen beendet Typisch ohne diese Änderungen.",
        " Closing again quits Typisch without these changes.",
    ),
    ("„{rel}“ ließ sich nicht neu laden.", "“{rel}” could not be reloaded."),
    ("Neu laden", "Reload"),
    ("Überschreiben", "Overwrite"),
    ("Wieder anlegen", "Create again"),
    (
        "„{rel}“ wurde außerhalb von Typisch geändert",
        "“{rel}” was changed outside Typisch",
    ),
    (
        "Ein anderes Programm hat die Datei geändert, während du sie hier bearbeitet hast. „Neu laden“ zeigt die Datei, wie sie jetzt auf der Festplatte liegt, und verwirft deine ungespeicherten Änderungen. „Überschreiben“ speichert deinen Stand und ersetzt damit die Änderung des anderen Programms.",
        "Another program changed the file while you were editing it here. “Reload” shows the file as it now is on disk and discards your unsaved changes. “Overwrite” saves your version and replaces the other program's change.",
    ),
    ("„{rel}“ gibt es nicht mehr", "“{rel}” no longer exists"),
    (
        "Die Datei wurde gelöscht, umbenannt oder verschoben, während du sie hier bearbeitet hast. „Wieder anlegen“ speichert deinen Stand unter dem alten Namen. „Tab schließen“ verwirft deine ungespeicherten Änderungen.",
        "The file was deleted, renamed or moved while you were editing it here. “Create again” saves your version under the old name. “Close Tab” discards your unsaved changes.",
    ),
];
