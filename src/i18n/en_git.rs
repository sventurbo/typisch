//! English translations for Git Sync's messages (git.rs).

/// German UI string → English text.
pub const EN: &[(&str, &str)] = &[
    // Errors
    (
        "Git ist nicht verfügbar. Installiere die Xcode Command Line Tools (xcode-select --install).",
        "Git is not available. Install the Xcode Command Line Tools (xcode-select --install).",
    ),
    ("Git ist nicht installiert.", "Git is not installed."),
    ("Dieser Ordner ist kein Git-Repository.", "This folder is not a Git repository."),
    (
        "Das Projekt ist mit keinem Repository verbunden.",
        "The project is not connected to a repository.",
    ),
    (
        "Es ist kein Branch ausgecheckt (losgelöster HEAD).",
        "No branch is checked out (detached HEAD).",
    ),
    (
        "Im Repository ist ein {op} nicht abgeschlossen. Schließe ihn zuerst im Terminal ab.",
        "The repository has an unfinished {op}. Finish it in the terminal first.",
    ),
    ("Anmeldung erforderlich.", "Sign-in required."),
    ("Anmeldung bei {host} erforderlich.", "Sign-in to {host} required."),
    ("Repository nicht gefunden: {url}", "Repository not found: {url}"),
    (
        "Im Repository gibt es neue Änderungen. Hole sie zuerst mit Pull und pushe dann erneut.",
        "The repository has new changes. Pull them first, then push again.",
    ),
    (
        "Der Branch „{branch}“ enthält eine andere Versionsgeschichte. Wähle einen neuen oder leeren Branch – oder klone das Repository als neues Projekt.",
        "The branch “{branch}” contains a different history. Choose a new or empty branch – or clone the repository as a new project.",
    ),
    (
        "Diese Dateien gibt es hier und im Repository mit unterschiedlichem Inhalt: {files}. Wähle einen neuen Branch oder klone das Repository.",
        "These files exist here and in the repository with different content: {files}. Choose a new branch or clone the repository.",
    ),
    (
        "Der Ordner {path} existiert bereits und ist nicht leer.",
        "The folder {path} already exists and is not empty.",
    ),
    ("Git hat zu lange nicht geantwortet.", "Git took too long to respond."),
    ("Bitte eine Repository-Adresse angeben.", "Please enter a repository address."),
    ("Interner Fehler bei Git Sync.", "Internal error in Git Sync."),
    ("Git ließ sich nicht starten: {e}", "Git could not be started: {e}"),
    (
        "Die Anmeldung ließ sich nicht an Git übergeben: {e}",
        "The sign-in could not be passed to Git: {e}",
    ),
    ("Git ließ sich nicht ausführen: {e}", "Git could not be run: {e}"),
    (
        "Git vertraut diesem Ordner nicht, weil er einem anderen Benutzer gehört. Erlaube ihn im Terminal mit: git config --global --add safe.directory {path}",
        "Git does not trust this folder because it belongs to another user. Allow it in the terminal with: git config --global --add safe.directory {path}",
    ),
    (
        "Diesen Branch gibt es im Repository nicht.",
        "This branch does not exist in the repository.",
    ),
    (
        "Keine Verbindung zum Git-Server. Prüfe die Internetverbindung.",
        "Could not connect to the Git server. Check your internet connection.",
    ),
    (
        "Keine Verbindung zu {host}. Prüfe die Internetverbindung.",
        "Could not connect to {host}. Check your internet connection.",
    ),
    ("Git ist mit Fehler {code} abgebrochen.", "Git stopped with error {code}."),
    ("Git wurde abgebrochen.", "Git was interrupted."),
    ("Dateifehler: {e}", "File error: {e}"),
    (
        "Git hat unerwartet geantwortet (rev-parse).",
        "Git gave an unexpected answer (rev-parse).",
    ),
    ("Git hat unerwartet geantwortet: {text}", "Git gave an unexpected answer: {text}"),
    (
        "Dieses Repository verweist über core.worktree auf einen anderen Ordner ({path}). Entferne die Einstellung im Terminal.",
        "This repository points to a different folder through core.worktree ({path}). Remove the setting in the terminal.",
    ),
    ("Git kennt „{rev}“ nicht.", "Git does not know “{rev}”."),
    (
        "Der Branch „{branch}“ wurde im Repository gelöscht oder umbenannt. Verbinde das Projekt im Terminal mit dem neuen Branch.",
        "The branch “{branch}” was deleted or renamed in the repository. Connect the project to the new branch in the terminal.",
    ),
    ("Name und E-Mail-Adresse fehlen.", "Name and email address are missing."),
    (
        "Der Branch „{local}“ verfolgt „{remote}/{branch}“. Ein Push würde nach „{target}“ schreiben. Benenne den Branch um oder verbinde ihn im Terminal neu.",
        "The branch “{local}” tracks “{remote}/{branch}”. A push would write to “{target}”. Rename the branch or reconnect it in the terminal.",
    ),
    ("Bitte einen Branch angeben.", "Please enter a branch."),
    ("Ungültiger Zielordner: {path}", "Invalid destination folder: {path}"),
    (
        "Datei- und Ordnernamen kollidieren zwischen Ordner und Repository: {files}. Benenne sie um und versuche es erneut.",
        "File and folder names collide between the folder and the repository: {files}. Rename them and try again.",
    ),
    (
        "Die Sicherung der lokalen Änderungen ließ sich nicht anlegen.",
        "The backup of the local changes could not be created.",
    ),
    (
        "Der Pull würde diese Dateien überschreiben: {files}. Verschiebe oder lösche sie zuerst.",
        "The pull would overwrite these files: {files}. Move or delete them first.",
    ),
    (
        "Datei- und Ordnernamen kollidieren ({path}). Benenne einen davon um und versuche es erneut.",
        "File and folder names collide ({path}). Rename one of them and try again.",
    ),
    (
        "Die Datei {path} gibt es hier nur in anderer Groß-/Kleinschreibung. Benenne sie um und versuche es erneut.",
        "The file {path} exists here only with different upper/lower case. Rename it and try again.",
    ),
    (
        "{error}\nDeine lokalen Änderungen sind gesichert: git stash apply {name} (Wiederherstellen: {restore})",
        "{error}\nYour local changes are backed up: git stash apply {name} (restoring: {restore})",
    ),
    (
        "\nDeine lokalen Änderungen sind gesichert: git stash apply {name}",
        "\nYour local changes are backed up: git stash apply {name}",
    ),
    (
        "Ein unterbrochener Pull ließ sich nicht wiederherstellen: {e}{hint}",
        "An interrupted pull could not be restored: {e}{hint}",
    ),
    (
        "Der Ordner enthält einen beschädigten .git-Ordner.",
        "The folder contains a damaged .git folder.",
    ),
    (
        "Der Projektordner liegt innerhalb des Git-Repositories {path}. Öffne diesen Ordner als Projekt oder klone das Repository als neues Projekt.",
        "The project folder lies inside the Git repository {path}. Open that folder as the project or clone the repository as a new project.",
    ),
    (
        "„{branch}“ ist kein gültiger Branch-Name.",
        "“{branch}” is not a valid branch name.",
    ),
    (
        "Den Branch „{wanted}“ gibt es im Repository nicht.",
        "The branch “{wanted}” does not exist in the repository.",
    ),
    // Reset
    (
        "Den Branch „{branch}“ gibt es im Repository noch nicht.",
        "The branch “{branch}” does not exist in the repository yet.",
    ),
    (
        "Der Branch hat hier noch keine Commits. Hole die Änderungen zuerst mit Pull.",
        "The branch has no commits here yet. Pull the changes first.",
    ),
    (
        "Im Repository gibt es Änderungen außerhalb des Projektordners: {files}. Sichere oder verwirf sie zuerst im Terminal.",
        "The repository has changes outside the project folder: {files}. Commit or discard them in the terminal first.",
    ),
    // Defaults and short ages
    ("Änderungen aus Typisch", "Changes from Typisch"),
    ("Projekt", "Project"),
    ("jetzt", "now"),
    ("{n} T", "{n} d"),
    ("{n} W", "{n} w"),
];
