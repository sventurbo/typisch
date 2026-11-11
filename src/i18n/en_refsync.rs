//! English translations for the Zotero sync and its settings (refsync.rs, refsync_ui.rs).

/// German UI string → English text.
pub const EN: &[(&str, &str)] = &[
    // Sync
    ("Meine Bibliothek", "My Library"),
    ("Zotero ist nicht verbunden.", "Zotero is not connected."),
    ("Kein Konfigurationsordner gefunden", "No configuration folder found"),
    (
        "Kein Zotero-Schlüssel gespeichert. Bitte in den Einstellungen unter „Konten“ einen Schlüssel eintragen.",
        "No Zotero key stored. Please enter a key in the settings under “Accounts”.",
    ),
    (
        "Der Zotero-Schlüssel konnte nicht aus dem Schlüsselbund gelesen werden: {e}",
        "The Zotero key could not be read from the keychain: {e}",
    ),
    (
        "Unter Einstellungen › Konten lässt sich der Schlüssel ersetzen.",
        "The key can be replaced under Settings › Accounts.",
    ),
    (
        "Dieser Schlüssel darf weder deine Bibliothek noch deine Gruppen lesen. Bitte einen Schlüssel mit Lesezugriff erstellen.",
        "This key may read neither your library nor your groups. Please create a key with read access.",
    ),
    (
        "Zotero kennt diesen API-Schlüssel nicht. Bitte prüfen, ob er vollständig eingefügt wurde.",
        "Zotero does not know this API key. Please check that it was pasted completely.",
    ),
    ("Kein Lesezugriff mit diesem Schlüssel", "No read access with this key"),
    (
        "% Automatisch aus Zotero synchronisiert von Typisch. Änderungen in dieser Datei werden überschrieben.\n% Quelle: {source}\n",
        "% Synced automatically from Zotero by Typisch. Changes to this file will be overwritten.\n% Source: {source}\n",
    ),
    ("% Wird gerade synchronisiert …", "% Syncing…"),
    ("Bitte einen Dateinamen eingeben.", "Please enter a file name."),
    (
        "Bitte einen Dateinamen innerhalb des Projekts angeben.",
        "Please enter a file name inside the project.",
    ),
    (
        "Versteckte Dateien und Ordner (mit Punkt am Anfang) sind nicht möglich.",
        "Hidden files and folders (starting with a dot) are not possible.",
    ),
    (
        "Der Schlüssel enthält ungültige Zeichen. Bitte nur den Schlüssel von zotero.org einfügen.",
        "The key contains invalid characters. Please paste only the key from zotero.org.",
    ),
    (
        "Zotero ist in dieser Sitzung nicht verfügbar.",
        "Zotero is not available in this session.",
    ),
    (
        "Der Schlüssel konnte nicht gelöscht werden: {e}",
        "The key could not be deleted: {e}",
    ),
    (
        "Der Schlüssel konnte nicht gespeichert werden: {e}",
        "The key could not be saved: {e}",
    ),
    ("Gruppe {id}", "Group {id}"),
    ("Bitte auswählen, was synchronisiert werden soll.", "Please choose what to sync."),
    (
        "„{rel}“ gibt es schon. Bitte einen anderen Namen wählen.",
        "“{rel}” already exists. Please choose a different name.",
    ),
    (
        "„{rel}“ liegt über einen verknüpften Ordner außerhalb des Projekts.",
        "“{rel}” lies outside the project through a linked folder.",
    ),
    (
        "Der Ordner konnte nicht angelegt werden: {e}",
        "The folder could not be created: {e}",
    ),
    ("Die Datei konnte nicht angelegt werden: {e}", "The file could not be created: {e}"),
    ("Der Pfad führt aus dem Projekt hinaus.", "The path leads out of the project."),
    (
        "Die Datei konnte nicht geschrieben werden: {e}",
        "The file could not be written: {e}",
    ),
    // Settings: accounts
    ("Verbindungen zu anderen Diensten", "Connections to other services"),
    ("Verbunden als {name}", "Connected as {name}"),
    ("Nicht verbunden", "Not connected"),
    (
        "Typisch liest die Bibliothek und die Gruppen von {name} ({username}) und hält damit Literaturdateien in deinen Projekten aktuell. Neue Dateien legst du in der Dateiliste über „Aus Zotero synchronisieren …“ an.",
        "Typisch reads the library and the groups of {name} ({username}) and uses them to keep bibliography files in your projects up to date. Create new files in the file list with “Sync from Zotero…”.",
    ),
    ("1 Datei wird aus Zotero synchronisiert.", "1 file is synced from Zotero."),
    ("{n} Dateien werden aus Zotero synchronisiert.", "{n} files are synced from Zotero."),
    ("Verbindung trennen", "Disconnect"),
    ("Nicht ersetzen", "Don't replace"),
    ("Schlüssel ersetzen …", "Replace key…"),
    ("Schlüssel bei Zotero verwalten …", "Manage keys at Zotero…"),
    (
        "Ein neuer Schlüssel, etwa wenn der alte widerrufen wurde. Alle synchronisierten Dateien bleiben verbunden.",
        "A new key, for example when the old one was revoked. All synced files stay connected.",
    ),
    (
        "Beim Trennen wird der Schlüssel von diesem Rechner gelöscht, und alle synchronisierten Literaturdateien werden zu normalen Textdateien, die du selbst bearbeiten kannst.",
        "Disconnecting deletes the key from this computer, and all synced bibliography files become normal text files that you can edit yourself.",
    ),
    ("Die Verbindung zu Zotero wurde getrennt.", "Zotero was disconnected."),
    (
        "Verbinde dein Zotero-Konto, um Literaturverzeichnisse aus deiner Bibliothek, einer Sammlung oder einer Gruppe als BibLaTeX-Datei in deine Projekte zu übernehmen. Typisch hält sie danach automatisch aktuell.",
        "Connect your Zotero account to bring bibliographies from your library, a collection or a group into your projects as a BibLaTeX file. Typisch then keeps them up to date automatically.",
    ),
    (
        "Einen Schlüssel mit Lesezugriff bei Zotero erstellen:",
        "Create a key with read access at Zotero:",
    ),
    ("Schlüssel bei Zotero erstellen …", "Create key at Zotero…"),
    (
        "Die Seite öffnet sich im Browser, die Rechte (nur lesen) sind schon gesetzt. Unten auf „Save Key“ klicken und den angezeigten Schlüssel kopieren.",
        "The page opens in the browser with the permissions (read only) already set. Click “Save Key” at the bottom and copy the key that is shown.",
    ),
    ("Den Schlüssel hier einfügen:", "Paste the key here:"),
    ("API-Schlüssel", "API key"),
    ("Wird bei Zotero geprüft …", "Checking with Zotero…"),
    (
        "Der Schlüssel wird im Schlüsselbund von macOS gespeichert und nur an api.zotero.org gesendet.",
        "The key is stored in the macOS keychain and only sent to api.zotero.org.",
    ),
    (
        "Der Schlüssel wird nur für dich lesbar in deinem Konfigurationsordner gespeichert und nur an api.zotero.org gesendet.",
        "The key is stored in your configuration folder, readable only by you, and only sent to api.zotero.org.",
    ),
    // Dialog: sync from Zotero
    ("Aus Zotero synchronisieren", "Sync from Zotero"),
    (
        "„{rel}“ wird aus Zotero synchronisiert. Einbinden mit #bibliography(\"{path}\").",
        "“{rel}” is synced from Zotero. Include it with #bibliography(\"{path}\").",
    ),
    ("Dateiname", "File name"),
    (
        "Eine BibLaTeX-Datei im Projekt, schreibgeschützt und automatisch aktuell. Einbinden mit #bibliography(\"{shown}\").",
        "A BibLaTeX file in the project, read-only and kept up to date automatically. Include it with #bibliography(\"{shown}\").",
    ),
    ("Was soll synchronisiert werden?", "What should be synced?"),
    ("Einträge aus Unter-Sammlungen einbeziehen", "Include entries from subcollections"),
    ("Erstellen & synchronisieren", "Create & sync"),
    ("Erneut versuchen", "Try again"),
    ("Bibliotheken werden von Zotero geladen …", "Loading libraries from Zotero…"),
    ("ganze Bibliothek", "entire library"),
    ("1 Eintrag", "1 entry"),
    ("{n} Einträge", "{n} entries"),
    // Banner of a synced file
    (
        "Zotero ist nicht verbunden – die Datei wird nicht aktualisiert.",
        "Zotero is not connected – the file is not being updated.",
    ),
    ("Wird synchronisiert …", "Syncing…"),
    ("Zuletzt synchronisiert {ago}", "Last synced {ago}"),
    ("Noch nicht synchronisiert", "Not synced yet"),
    ("Schreibgeschützt · aus Zotero: {source}", "Read-only · from Zotero: {source}"),
    (
        "Diese Datei wird aus Zotero synchronisiert ({source}) und kann hier nicht bearbeitet werden.\n{entries}. Zum Bearbeiten die Synchronisierung trennen.",
        "This file is synced from Zotero ({source}) and cannot be edited here.\n{entries}. Stop syncing to edit it.",
    ),
    (
        "Synchronisierung trennen – die Datei wird zu einer normalen, bearbeitbaren Datei",
        "Stop syncing – the file becomes a normal, editable file",
    ),
    ("Wird gerade synchronisiert", "Syncing right now"),
    ("Jetzt synchronisieren", "Sync now"),
    ("Verbinden …", "Connect…"),
    ("zuletzt {ago}", "last {ago}"),
    ("noch nicht synchronisiert", "not synced yet"),
    ("Aus Zotero synchronisiert ({when})", "Synced from Zotero ({when})"),
];
