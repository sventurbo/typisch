//! English translations for the start screen, the template picker and the settings (screens.rs, theme.rs, widgets.rs).

/// German UI string → English text.
pub const EN: &[(&str, &str)] = &[
    // Start screen
    ("Letzte Projekte", "Recent projects"),
    ("Noch keine Projekte", "No projects yet"),
    ("Öffnen", "Open"),
    ("Im Finder zeigen", "Show in Finder"),
    ("Im Dateimanager zeigen", "Show in file manager"),
    ("Aus der Liste entfernen", "Remove from list"),
    ("Typst-Editor mit Live-Vorschau", "Typst editor with live preview"),
    (
        "Beim Öffnen einer Datei wird ihr Ordner als Projekt geöffnet.",
        "Opening a file opens its folder as the project.",
    ),
    // Template picker
    ("Schließen (Esc)", "Close (Esc)"),
    ("Template auswählen", "Choose a template"),
    ("Templates suchen", "Search templates"),
    ("Alle", "All"),
    ("Erstellen …", "Create…"),
    ("Abbrechen", "Cancel"),
    ("Keine Templates gefunden", "No templates found"),
    // Settings: tabs
    ("Allgemein", "General"),
    ("Pakete", "Packages"),
    ("Konten", "Accounts"),
    // Settings: general
    (
        "Wechselt automatisch zwischen deinem hellen und deinem dunklen Theme, wenn das System umschaltet.",
        "Switches automatically between your light and your dark theme when the system does.",
    ),
    ("Oberflächengröße", "Interface size"),
    (
        "Vergrößert oder verkleinert die ganze Oberfläche ({bigger} / {smaller}). Die Bildschirmskalierung des Systems wird schon berücksichtigt.",
        "Makes the whole interface larger or smaller ({bigger} / {smaller}). The system's display scaling is already taken into account.",
    ),
    ("Live-Vorschau", "Live preview"),
    ("Editor-Position", "Editor position"),
    ("Links", "Left"),
    ("Rechts", "Right"),
    (
        "Auf welcher Seite des Fensters der Editor liegt – die Live-Vorschau nimmt die andere Seite ein.",
        "Which side of the window the editor is on – the live preview takes the other side.",
    ),
    ("Welle beim Klicken", "Ripple on click"),
    (
        "Lässt eine sanfte Welle vom angeklickten Punkt in der Vorschau ausgehen.",
        "Sends a gentle ripple out from the point you click in the preview.",
    ),
    ("Diagnose", "Diagnostics"),
    ("Bibliothekswarnungen ignorieren", "Ignore library warnings"),
    (
        "Blendet Warnungen aus importierten Paketen aus (z. B. @preview/…). Warnungen in eigenen Dateien und alle Fehler werden weiterhin angezeigt.",
        "Hides warnings from imported packages (e.g. @preview/…). Warnings in your own files and all errors are still shown.",
    ),
    ("Typst-Compiler", "Typst compiler"),
    ("Integriert", "Built in"),
    ("Ausgewählt", "Selected"),
    (
        "Der eingebaute Typst-Compiler (Version 0.15). Er läuft vollständig auf diesem Rechner.",
        "The built-in Typst compiler (version 0.15). It runs entirely on this computer.",
    ),
    ("Letzter Durchlauf: {ms} ms", "Last run: {ms} ms"),
    // Settings: editor
    ("Editor-Typografie", "Editor typography"),
    ("Schrift", "Font"),
    ("Größe", "Size"),
    (
        "#let answer = 42\n_Typst_ ist *großartig*!",
        "#let answer = 42\n_Typst_ is *awesome*!",
    ),
    ("Blinkende Einfügemarke", "Blinking cursor"),
    ("Zeilennummern", "Line numbers"),
    ("Wortanzahl anzeigen", "Show word count"),
    ("Relative Zeilennummern", "Relative line numbers"),
    ("Automatisch schließen", "Auto-close"),
    ("Sofort schließen", "Close immediately"),
    ("Nur vor Leerraum", "Only before whitespace"),
    ("Nie", "Never"),
    (
        "Klammern, Anführungszeichen und $ werden beim Tippen automatisch geschlossen.",
        "Brackets, quotation marks and $ are closed automatically as you type.",
    ),
    ("Vorschläge beim Tippen", "Suggestions while typing"),
    (
        "Funktionen, Parameter, Symbole und Verweise, wie in der Typst-Web-App. Mit {keys} jederzeit.",
        "Functions, parameters, symbols and references, as in the Typst web app. Any time with {keys}.",
    ),
    // Settings: packages
    ("Online-Pakete", "Online packages"),
    (
        "Werden automatisch geladen, wenn Sie #import \"@preview/…\" verwenden. Laden Sie eines vorab per Angabe:",
        "Downloaded automatically when you use #import \"@preview/…\". Download one in advance by its name:",
    ),
    ("Laden", "Download"),
    ("Wird geladen …", "Downloading…"),
    ("Noch keine Pakete geladen", "No packages downloaded yet"),
    ("Aus dem Zwischenspeicher löschen", "Delete from the cache"),
    ("Lokale Pakete", "Local packages"),
    (
        "Registrieren Sie einen Ordner als Namespace. Ein Ordner mit meine_lib/<version>/, registriert als „privat“, wird als @privat/meine_lib:<version> importiert. Beim Entfernen wird nur die Zuordnung vergessen – Ihre Dateien bleiben unangetastet.",
        "Register a folder as a namespace. A folder containing my_lib/<version>/, registered as “private”, is imported as @private/my_lib:<version>. Removing it only forgets the link – your files stay untouched.",
    ),
    ("Ordner verknüpfen …", "Link folder…"),
    ("Noch keine Namespaces registriert", "No namespaces registered yet"),
    ("Zuordnung entfernen", "Remove link"),
    ("Kein Datenordner gefunden", "No data folder found"),
    (
        "Der Namespace „{ns}“ ist bereits ein echter Ordner.",
        "The namespace “{ns}” already is a real folder.",
    ),
    (
        "Lokale Pakete werden auf diesem System nicht unterstützt.",
        "Local packages are not supported on this system.",
    ),
    // Widgets
    ("Leeren", "Clear"),
];
