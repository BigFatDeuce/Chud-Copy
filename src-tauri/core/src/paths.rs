//! Comparaison sûre des chemins.
//!
//! Deux écritures différentes peuvent désigner le même dossier : casse sous Windows, préfixe
//! `\\?\`, barres obliques, lien symbolique ou jonction. Toute détection de chevauchement
//! passe donc par une clé normalisée, résolue sur le disque quand c'est possible.

use std::path::{Component, Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Temps laissé à un disque pour répondre avant de le déclarer inaccessible.
const PROBE_TIMEOUT: Duration = Duration::from_secs(4);

/// Résout le chemin le plus loin possible sur le disque, sans exiger qu'il existe en entier.
pub fn resolve_existing(path: &Path) -> PathBuf {
    let mut suffix = Vec::new();
    let mut cursor = path;
    loop {
        if let Ok(canonical) = std::fs::canonicalize(cursor) {
            let mut out = canonical;
            for part in suffix.iter().rev() {
                out.push(part);
            }
            return out;
        }
        match (cursor.parent(), cursor.file_name()) {
            (Some(parent), Some(name)) => {
                suffix.push(name.to_os_string());
                cursor = parent;
            }
            _ => return path.to_path_buf(),
        }
    }
}

/// Découpe un chemin en éléments comparables.
pub fn key(path: &Path) -> Vec<String> {
    key_of_text(&resolve_existing(path).to_string_lossy())
}

/// Variante purement textuelle, sans accès au disque.
pub fn key_of_text(text: &str) -> Vec<String> {
    let cleaned = text
        .strip_prefix(r"\\?\UNC\")
        .map(|rest| format!(r"\\{rest}"))
        .or_else(|| text.strip_prefix(r"\\?\").map(str::to_string))
        .unwrap_or_else(|| text.to_string());
    let cleaned = if cfg!(windows) { cleaned.to_lowercase() } else { cleaned };
    let separators: &[char] = if cfg!(windows) { &['\\', '/'] } else { &['/'] };
    cleaned
        .split(separators)
        .filter(|part| !part.is_empty() && *part != ".")
        .map(str::to_string)
        .collect()
}

pub fn same(a: &[String], b: &[String]) -> bool {
    a == b
}

/// Vrai si `child` est strictement sous `parent`.
pub fn inside(child: &[String], parent: &[String]) -> bool {
    child.len() > parent.len() && child[..parent.len()] == *parent
}

pub fn overlap(a: &[String], b: &[String]) -> bool {
    same(a, b) || inside(a, b) || inside(b, a)
}

/// Une racine de disque : `D:\`, `/` ou un partage réseau nu.
pub fn is_root(path: &Path) -> bool {
    let mut normal = 0;
    for component in path.components() {
        if matches!(component, Component::Normal(_)) {
            normal += 1;
        }
    }
    normal == 0
}

/// Dossiers système qu'un miroir ne doit jamais prendre pour cible.
pub fn is_protected(path: &Path) -> bool {
    is_protected_key(&key(path))
}

/// Comme `is_protected`, pour une clé déjà calculée : aucun accès au disque visé.
pub fn is_protected_key(target: &[String]) -> bool {
    let mut protected: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        for var in [
            "SystemDrive",
            "SystemRoot",
            "ProgramFiles",
            "ProgramFiles(x86)",
            "ProgramData",
            "USERPROFILE",
            "APPDATA",
            "LOCALAPPDATA",
        ] {
            if let Some(value) = std::env::var_os(var) {
                let mut p = PathBuf::from(value);
                if var == "SystemDrive" {
                    p.push("\\");
                }
                protected.push(p);
            }
        }
        if let Some(drive) = std::env::var_os("SystemDrive") {
            protected.push(PathBuf::from(drive).join("Users"));
        }
    } else {
        for dir in [
            "/", "/bin", "/boot", "/dev", "/etc", "/home", "/lib", "/lib64", "/mnt", "/media", "/opt", "/proc", "/root", "/run", "/sbin", "/srv", "/sys",
            "/tmp", "/usr", "/var",
        ] {
            protected.push(PathBuf::from(dir));
        }
        if let Some(home) = std::env::var_os("HOME") {
            protected.push(PathBuf::from(home));
        }
    }
    protected.iter().any(|p| key(p) == target)
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PathState {
    Ready,
    /// Le chemin n'existe pas encore mais son disque est présent.
    WillBeCreated,
    /// Le chemin ou son disque sont absents.
    Missing,
    NotADirectory,
    Denied,
    /// Le disque n'a pas répondu à temps.
    Unresponsive,
}

/// Inspecte un chemin sans jamais le modifier, ni bloquer sur un disque endormi ou absent.
pub fn probe(path: &Path, is_source: bool) -> PathState {
    let owned = path.to_path_buf();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(probe_blocking(&owned, is_source));
    });
    rx.recv_timeout(PROBE_TIMEOUT).unwrap_or(PathState::Unresponsive)
}

/// Inspecte plusieurs chemins en même temps : l'état de chacun et sa clé de comparaison. Tous
/// partagent le même délai : un NAS éteint coûte 4 secondes au total, pas 4 secondes par chemin.
/// Un chemin qui n'a pas répondu à temps est inaccessible, et sa clé est tirée de son seul texte,
/// sans attendre un disque qui pourrait ne jamais répondre.
pub fn inspect_all(items: &[(PathBuf, bool)]) -> Vec<(PathState, Vec<String>)> {
    let (tx, rx) = mpsc::channel();
    for (index, (path, is_source)) in items.iter().enumerate() {
        let tx = tx.clone();
        let (path, is_source) = (path.clone(), *is_source);
        std::thread::spawn(move || {
            let state = probe_blocking(&path, is_source);
            let _ = tx.send((index, state, key(&path)));
        });
    }
    drop(tx);
    let deadline = Instant::now() + PROBE_TIMEOUT;
    let mut out: Vec<Option<(PathState, Vec<String>)>> = vec![None; items.len()];
    let mut pending = items.len();
    while pending > 0 {
        match rx.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok((index, state, key)) => {
                out[index] = Some((state, key));
                pending -= 1;
            }
            Err(_) => break,
        }
    }
    out.into_iter()
        .zip(items)
        .map(|(found, (path, _))| found.unwrap_or_else(|| (PathState::Unresponsive, key_of_text(&path.to_string_lossy()))))
        .collect()
}

fn probe_blocking(path: &Path, is_source: bool) -> PathState {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_dir() => match std::fs::read_dir(path) {
            Ok(_) => {
                if !is_source && !writable(path) {
                    PathState::Denied
                } else {
                    PathState::Ready
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => PathState::Denied,
            Err(_) => PathState::Missing,
        },
        Ok(_) => PathState::NotADirectory,
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => PathState::Denied,
        Err(_) if is_source => PathState::Missing,
        Err(_) => {
            // Une destination absente sera créée, à condition que son disque soit là.
            match path.ancestors().skip(1).find(|p| p.is_dir()) {
                Some(existing) if writable(existing) => PathState::WillBeCreated,
                Some(_) => PathState::Denied,
                None => PathState::Missing,
            }
        }
    }
}

#[cfg(unix)]
fn writable(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    unsafe { libc::access(c.as_ptr(), libc::W_OK | libc::X_OK) == 0 }
}

#[cfg(windows)]
fn writable(_path: &Path) -> bool {
    // Sous Windows les ACL ne se résolvent pas de façon fiable sans écrire : un refus réel
    // est remonté à l'exécution plutôt que de créer un fichier témoin chez l'utilisateur.
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_paths_are_detected_on_the_written_form() {
        let a = key_of_text(if cfg!(windows) { r"D:\Photos" } else { "/d/Photos" });
        let b = key_of_text(if cfg!(windows) { r"d:/photos/2024/" } else { "/d/Photos/2024/" });
        assert!(inside(&b, &a));
        assert!(overlap(&a, &b));
        assert!(!same(&a, &b));
    }

    #[test]
    fn sibling_with_common_prefix_is_not_nested() {
        let a = key_of_text(if cfg!(windows) { r"D:\Photos" } else { "/d/Photos" });
        let b = key_of_text(if cfg!(windows) { r"D:\Photos2" } else { "/d/Photos2" });
        assert!(!overlap(&a, &b));
    }

    #[cfg(windows)]
    #[test]
    fn windows_forms_are_equivalent() {
        assert_eq!(key_of_text(r"\\?\C:\Temp\A"), key_of_text(r"c:\temp\a\"));
        assert_eq!(key_of_text(r"\\?\UNC\nas\share"), key_of_text(r"\\NAS\Share"));
    }

    #[test]
    fn roots_are_recognised() {
        assert!(is_root(Path::new(if cfg!(windows) { r"H:\" } else { "/" })));
        assert!(!is_root(Path::new(if cfg!(windows) { r"H:\Backup" } else { "/backup" })));
    }
}
