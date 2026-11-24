//! Traduit les messages du moteur en explications compréhensibles.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FriendlyError {
    pub category: String,
    pub title: String,
    pub path: Option<String>,
    pub detail: String,
}

pub fn humanize(path: Option<&str>, message: &str) -> FriendlyError {
    let lower = message.to_lowercase();
    let has = |needles: &[&str]| needles.iter().any(|n| lower.contains(n));

    let (category, title) = if has(&["max-delete threshold"]) {
        (
            "delete-limit",
            "Suppressions arrêtées : plus de fichiers à supprimer que dans l'analyse validée",
        )
    } else if has(&["no space left", "not enough space", "disk full", "there is not enough"]) {
        ("space", "Espace disque insuffisant")
    } else if has(&["being used by another process", "sharing violation", "has locked a portion", "lock violation"]) {
        ("locked", "Fichier verrouillé par une autre application")
    } else if has(&["access is denied", "permission denied", "operation not permitted", "a required privilege"]) {
        ("denied", "Accès refusé")
    } else if has(&["source file is being updated", "can't copy - source file"]) {
        ("changed", "Le fichier a changé pendant sa copie")
    } else if has(&["file name too long", "filename too long", "path too long", "filename or extension is too long"]) {
        ("long-path", "Chemin trop long pour ce système de fichiers")
    } else if has(&["corrupted on transfer", "sizes differ", "md5 differ", "hashes differ"]) {
        ("corrupt", "Le fichier copié ne correspond pas à l'original")
    } else if has(&["directory not found", "cannot find the path", "system cannot find", "no such file"]) {
        ("not-found", "Fichier ou dossier introuvable")
    } else if has(&[
        "input/output error",
        "device is not ready",
        "i/o error",
        "cyclic redundancy",
        "device not ready",
    ]) {
        ("io", "Erreur de lecture ou d'écriture du disque")
    } else if has(&["not deleting", "not deleting directories", "not deleting files"]) {
        ("delete-skipped", "Suppressions annulées par sécurité à cause d'autres erreurs")
    } else {
        ("other", "Erreur du moteur de copie")
    };

    FriendlyError {
        category: category.to_string(),
        title: title.to_string(),
        path: path.map(str::to_string),
        detail: message.trim().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_real_messages() {
        let cases = [
            (
                "Failed to copy: open x: The process cannot access the file because it is being used by another process.",
                "locked",
            ),
            ("Failed to copy: open /a: permission denied", "denied"),
            ("Got fatal error on delete: --max-delete threshold reached", "delete-limit"),
            ("write /a: no space left on device", "space"),
            (
                "Failed to copy: can't copy - source file is being updated (size changed from 1 to 2)",
                "changed",
            ),
            ("not deleting directories as there were IO errors", "delete-skipped"),
            ("mystère", "other"),
        ];
        for (message, category) in cases {
            assert_eq!(humanize(None, message).category, category, "{message}");
        }
    }
}
