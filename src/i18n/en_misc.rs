//! English translations for the language setting, the compiler and the templates.

/// German UI string → English text.
pub const EN: &[(&str, &str)] = &[
    ("Darstellung", "Appearance"),
    ("Sprache", "Language"),
    ("Wie das System", "Like the system"),
    (
        "Die Sprache der Oberfläche. „Wie das System“ folgt der Sprache, die im System eingestellt ist.",
        "The language of the interface. “Like the system” follows the language your system is set to.",
    ),
    ("Hell/Dunkel wie das System", "Light/dark like the system"),
    // Compiler
    ("unbekannter Fehler", "unknown error"),
    ("Interner Compilerfehler: {panic}", "Internal compiler error: {panic}"),
    (
        "Ein Fehler in Typst oder in einem Paket, nicht in deinem Dokument.",
        "An error in Typst or in a package, not in your document.",
    ),
    (
        "Das Dokument enthält Fehler und kann nicht exportiert werden.",
        "The document contains errors and cannot be exported.",
    ),
    // Template names
    ("Leeres Dokument", "Empty document"),
    ("Rechnung", "Invoice"),
    ("Brief", "Letter"),
    ("Lebenslauf", "CV"),
    ("Wissenschaftlicher Artikel", "Academic paper"),
    ("Präsentation", "Presentation"),
    ("Bericht", "Report"),
    ("Checkliste", "Checklist"),
    ("Buch", "Book"),
    ("Formelsammlung", "Formula collection"),
    // Template descriptions
    (
        "Eine leere main.typ, um bei null anzufangen.",
        "An empty main.typ to start from scratch.",
    ),
    (
        "Eine schlichte Rechnung mit Posten, Summe und Bankverbindung.",
        "A simple invoice with items, total and bank details.",
    ),
    (
        "Ein Geschäftsbrief mit Absender, Anschrift und Betreff.",
        "A business letter with sender, address and subject line.",
    ),
    ("Ein klarer, einspaltiger Lebenslauf.", "A clean, single-column CV."),
    (
        "Zweispaltiger Artikel mit Titel, Zusammenfassung und Abschnitten.",
        "Two-column paper with title, abstract and sections.",
    ),
    ("Folien im 16:9-Format mit großem Titel.", "16:9 slides with a large title."),
    (
        "Bericht mit Titelseite, Inhaltsverzeichnis und Kapiteln.",
        "Report with title page, table of contents and chapters.",
    ),
    ("Eine Checkliste mit Kästchen zum Abhaken.", "A checklist with boxes to tick off."),
    ("Ein farbiger Flyer für Veranstaltungen.", "A colorful flyer for events."),
    ("Ein Buch mit Kapiteln und Kopfzeile.", "A book with chapters and a running header."),
    (
        "Mathematische Formeln, schön gesetzt.",
        "Mathematical formulas, beautifully typeset.",
    ),
];
