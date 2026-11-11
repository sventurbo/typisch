//! English translations for Zotero's error messages (zotero.rs).

/// German UI string → English text.
pub const EN: &[(&str, &str)] = &[
    (
        "Zotero verweigert den Zugriff. Der API-Schlüssel ist ungültig, wurde widerrufen oder darf diese Bibliothek nicht lesen.",
        "Zotero denies access. The API key is invalid, was revoked or may not read this library.",
    ),
    (
        "Die Bibliothek oder Sammlung wurde in Zotero nicht gefunden (gelöscht oder kein Zugriff mehr).",
        "The library or collection was not found in Zotero (deleted or no longer accessible).",
    ),
    (
        "Zotero ist gerade überlastet oder wird gewartet. Später erneut versuchen.",
        "Zotero is currently overloaded or under maintenance. Try again later.",
    ),
    ("Zotero antwortet mit Fehler {s}.", "Zotero answers with error {s}."),
    ("Keine Verbindung zu Zotero: {e}", "Could not connect to Zotero: {e}"),
    ("Unerwartete Antwort von Zotero: {e}", "Unexpected answer from Zotero: {e}"),
    ("Ungültige Zeichen in der Anfrage", "Invalid characters in the request"),
];
