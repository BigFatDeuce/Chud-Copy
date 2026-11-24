//! Conseiller de nettoyage : repère les installateurs périmés, les fichiers temporaires
//! abandonnés, les téléchargements inachevés ou anciens et les dossiers les plus volumineux.
//!
//! L'analyse ne fait que lire. Rien n'est jamais supprimé automatiquement : l'utilisateur passe
//! la liste en revue, coche ce qu'il veut retirer, confirme, et les fichiers partent à la
//! corbeille, donc restent récupérables. Un fichier modifié depuis l'analyse n'est pas touché.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Instant, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::engine::Control;
use crate::filters::is_skipped_dir;
use crate::tools::ToolProgress;
use crate::{fail_text, Result};

const MAX_ITEMS: usize = 5000;
const MAX_FOLDERS: usize = 40;
const DAY: i64 = 86_400;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CleanupOptions {
    /// Âge minimal, en jours, d'un installateur pour qu'il soit proposé.
    pub installer_days: u32,
    pub temp_days: u32,
    pub download_days: u32,
    /// Taille à partir de laquelle un dossier est signalé.
    pub big_folder: u64,
}

impl Default for CleanupOptions {
    fn default() -> Self {
        Self {
            installer_days: 30,
            temp_days: 7,
            download_days: 180,
            big_folder: 1 << 30,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CleanupItem {
    pub path: String,
    /// « installer », « superseded », « temp », « partial » ou « download ».
    pub kind: String,
    pub size: u64,
    /// Dernière modification, en secondes depuis 1970.
    pub modified: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BigFolder {
    pub path: String,
    pub size: u64,
    pub files: u64,
    /// Fichier le plus récent du dossier, en secondes depuis 1970.
    pub newest: i64,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CleanupReport {
    pub roots: Vec<String>,
    pub scanned: u64,
    pub scanned_bytes: u64,
    pub items: Vec<CleanupItem>,
    pub folders: Vec<BigFolder>,
    pub cancelled: bool,
    pub elapsed: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Place {
    /// « downloads » ou « temp ».
    pub kind: String,
    pub path: String,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrashOutcome {
    pub trashed: Vec<String>,
    /// Fichiers laissés en place parce qu'ils ont changé ou disparu depuis l'analyse.
    pub skipped: Vec<String>,
    pub bytes: u64,
    /// Corbeilles Chud Copy utilisées sur les volumes sans corbeille du système.
    pub holding: Vec<String>,
}

const INSTALLER_EXTENSIONS: &[&str] = &[
    "msi",
    "msix",
    "msixbundle",
    "appx",
    "appxbundle",
    "dmg",
    "pkg",
    "deb",
    "rpm",
    "apk",
    "flatpakref",
];
const PARTIAL_EXTENSIONS: &[&str] = &["crdownload", "part", "partial", "download", "opdownload", "!ut", "aria2"];
const TEMP_EXTENSIONS: &[&str] = &["tmp", "temp"];
const INSTALLER_WORDS: &[&str] = &[
    "setup",
    "install",
    "installer",
    "update",
    "updater",
    "patch",
    "redist",
    "x64",
    "x86",
    "win64",
    "win32",
    "amd64",
    "arm64",
];
const DOWNLOAD_FOLDERS: &[&str] = &[
    "downloads",
    "download",
    "téléchargements",
    "telechargements",
    "descargas",
    "transferências",
    "downloads-ul",
    "загрузки",
    "下载",
    "التنزيلات",
    "डाउनलोड",
];

/// Les emplacements utiles à proposer : le dossier des téléchargements et le dossier temporaire.
pub fn places() -> Vec<Place> {
    let mut out = Vec::new();
    if let Some(downloads) = downloads_dir().filter(|p| p.is_dir()) {
        out.push(Place {
            kind: "downloads".into(),
            path: downloads.display().to_string(),
        });
    }
    let temp = std::env::temp_dir();
    if temp.is_dir() {
        out.push(Place {
            kind: "temp".into(),
            path: temp.display().to_string().trim_end_matches(['\\', '/']).to_string(),
        });
    }
    out
}

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from)
}

/// Le dossier des téléchargements de l'utilisateur, y compris quand il a été renommé sous Linux.
pub fn downloads_dir() -> Option<PathBuf> {
    let home = home()?;
    if cfg!(unix) {
        if let Ok(text) = fs::read_to_string(home.join(".config/user-dirs.dirs")) {
            for line in text.lines() {
                if let Some(value) = line.trim().strip_prefix("XDG_DOWNLOAD_DIR=") {
                    let value = value.trim().trim_matches('"').replace("$HOME", &home.to_string_lossy());
                    return Some(PathBuf::from(value));
                }
            }
        }
    }
    Some(home.join("Downloads"))
}

fn temp_roots() -> Vec<PathBuf> {
    let mut roots = vec![std::env::temp_dir()];
    if cfg!(unix) {
        roots.push(PathBuf::from("/tmp"));
        roots.push(PathBuf::from("/var/tmp"));
    }
    roots.into_iter().map(canonical).collect()
}

/// Chemin résolu sous Linux ; tel quel sous Windows, où la forme résolue (« \?C: ») est illisible.
fn canonical(path: PathBuf) -> PathBuf {
    if cfg!(windows) {
        path
    } else {
        fs::canonicalize(&path).unwrap_or(path)
    }
}

fn extension(name: &str) -> String {
    name.rsplit_once('.').map(|(_, ext)| ext.to_lowercase()).unwrap_or_default()
}

/// Le nom d'un installateur sans ses numéros de version : « VLC-3.0.20-win64.exe » → « vlc-win64 ».
pub fn product_key(name: &str) -> String {
    let stem = name.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(name).to_lowercase();
    let mut parts = Vec::new();
    for part in stem.split(|c: char| c == '-' || c == '_' || c == ' ' || c == '(' || c == ')') {
        let cleaned = part.trim_start_matches('v');
        let is_version = !cleaned.is_empty() && cleaned.chars().all(|c| c.is_ascii_digit() || c == '.');
        if !part.is_empty() && !is_version {
            // « setup1.2 » garde sa partie lettres.
            let letters: String = part.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.').to_string();
            if !letters.is_empty() {
                parts.push(letters);
            }
        }
    }
    parts.join("-")
}

fn is_download_folder(path: &Path, downloads: Option<&Path>) -> bool {
    if downloads.is_some_and(|d| path.starts_with(d)) {
        return true;
    }
    path.parent()
        .into_iter()
        .flat_map(|p| p.components())
        .any(|c| DOWNLOAD_FOLDERS.contains(&c.as_os_str().to_string_lossy().to_lowercase().as_str()))
}

fn is_installer(name: &str, in_downloads: bool) -> bool {
    let ext = extension(name);
    if INSTALLER_EXTENSIONS.contains(&ext.as_str()) {
        return true;
    }
    if ext == "exe" {
        let lower = name.to_lowercase();
        return in_downloads || INSTALLER_WORDS.iter().any(|w| lower.contains(w));
    }
    false
}

/// Ce que l'on peut dire d'un fichier, sans tenir compte de son âge ni des autres versions.
pub fn classify(path: &Path, downloads: Option<&Path>, temps: &[PathBuf]) -> Option<&'static str> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    let ext = extension(&name);
    if PARTIAL_EXTENSIONS.contains(&ext.as_str()) {
        return Some("partial");
    }
    if TEMP_EXTENSIONS.contains(&ext.as_str()) || name.starts_with("~$") || name.starts_with(".~lock.") {
        return Some("temp");
    }
    // Les indices précis passent avant l'emplacement : un installateur reste un installateur,
    // même rangé dans un dossier temporaire.
    let in_downloads = is_download_folder(path, downloads);
    if is_installer(&name, in_downloads) {
        return Some("installer");
    }
    if in_downloads {
        return Some("download");
    }
    if temps.iter().any(|t| path.starts_with(t)) {
        return Some("temp");
    }
    None
}

fn modified_secs(metadata: &fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

struct Folder {
    size: u64,
    files: u64,
    newest: i64,
}

pub fn scan(roots: &[PathBuf], options: &CleanupOptions, control: &Control, emit: &mut dyn FnMut(&ToolProgress)) -> Result<CleanupReport> {
    let downloads = downloads_dir().map(canonical);
    scan_with(roots, options, downloads.as_deref(), &temp_roots(), control, emit)
}

fn scan_with(
    roots: &[PathBuf],
    options: &CleanupOptions,
    downloads: Option<&Path>,
    temps: &[PathBuf],
    control: &Control,
    emit: &mut dyn FnMut(&ToolProgress),
) -> Result<CleanupReport> {
    let started = Instant::now();
    for root in roots {
        if !root.is_dir() {
            return fail_text("err.path-inaccessible");
        }
    }
    let now = crate::now_ms() / 1000;
    let mut progress = ToolProgress {
        phase: "list".into(),
        ..ToolProgress::default()
    };
    let mut last = Instant::now();
    let mut report = CleanupReport {
        roots: roots.iter().map(|r| r.display().to_string()).collect(),
        ..CleanupReport::default()
    };
    let mut folders: HashMap<PathBuf, Folder> = HashMap::new();
    let mut installers: Vec<CleanupItem> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    'roots: for root in roots {
        let canonical_root = canonical(root.clone());
        let walker = walkdir::WalkDir::new(&canonical_root)
            .min_depth(1)
            .follow_links(false)
            .into_iter()
            .filter_entry(|e| {
                let name = e.file_name().to_string_lossy();
                !(e.file_type().is_dir() && is_skipped_dir(&name))
            });
        for entry in walker.flatten() {
            if control.is_cancelled() {
                break 'roots;
            }
            while control.is_paused() && !control.is_cancelled() {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            if !entry.file_type().is_file() || !seen.insert(entry.path().to_path_buf()) {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            let size = metadata.len();
            let modified = modified_secs(&metadata);
            report.scanned += 1;
            report.scanned_bytes += size;
            progress.total_files = report.scanned;
            progress.total_bytes = report.scanned_bytes;
            if last.elapsed().as_millis() >= 200 {
                progress.current = entry.path().display().to_string();
                emit(&progress);
                last = Instant::now();
            }
            // Taille cumulée de chaque dossier jusqu'à la racine choisie.
            let mut dir = entry.path().parent();
            while let Some(current) = dir {
                if !current.starts_with(&canonical_root) {
                    break;
                }
                let folder = folders.entry(current.to_path_buf()).or_insert(Folder { size: 0, files: 0, newest: 0 });
                folder.size += size;
                folder.files += 1;
                folder.newest = folder.newest.max(modified);
                if current == canonical_root {
                    break;
                }
                dir = current.parent();
            }

            let age_days = (now - modified).max(0) / DAY;
            let path = entry.path().display().to_string();
            let item = |kind: &str| CleanupItem {
                path: path.clone(),
                kind: kind.into(),
                size,
                modified,
            };
            match classify(entry.path(), downloads, temps) {
                Some("partial") if age_days >= 2 => report.items.push(item("partial")),
                Some("temp") if age_days >= options.temp_days as i64 => report.items.push(item("temp")),
                // Les installateurs sont triés plus bas : une ancienne version est périmée quel que soit son âge.
                Some("installer") => installers.push(item("installer")),
                Some("download") if age_days >= options.download_days as i64 => report.items.push(item("download")),
                _ => {}
            }
        }
    }

    // Plusieurs versions d'un même installateur dans un dossier : seule la plus récente sert encore.
    let mut groups: HashMap<(String, String), Vec<usize>> = HashMap::new();
    for (index, installer) in installers.iter().enumerate() {
        let path = Path::new(&installer.path);
        let folder = path.parent().map(|p| p.display().to_string()).unwrap_or_default();
        let key = product_key(&path.file_name().unwrap_or_default().to_string_lossy());
        groups.entry((folder, key)).or_default().push(index);
    }
    let mut superseded = vec![false; installers.len()];
    for indexes in groups.values().filter(|g| g.len() > 1) {
        let newest = indexes.iter().copied().max_by_key(|&i| installers[i].modified).unwrap();
        for &i in indexes.iter().filter(|&&i| i != newest) {
            superseded[i] = true;
        }
    }
    for (index, mut installer) in installers.into_iter().enumerate() {
        let age_days = (now - installer.modified).max(0) / DAY;
        if superseded[index] {
            installer.kind = "superseded".into();
            report.items.push(installer);
        } else if age_days >= options.installer_days as i64 {
            report.items.push(installer);
        }
    }

    // Les plus gros dossiers, au niveau le plus précis : un dossier dont un sous-dossier porte
    // presque tout le poids s'efface derrière lui.
    // Le plus lourd sous-dossier de chaque dossier, en un seul passage.
    let mut heaviest: HashMap<&Path, u64> = HashMap::new();
    for (path, folder) in folders.iter() {
        if let Some(parent) = path.parent() {
            let entry = heaviest.entry(parent).or_default();
            *entry = (*entry).max(folder.size);
        }
    }
    let mut big: Vec<BigFolder> = folders
        .iter()
        .filter(|(path, folder)| folder.size >= options.big_folder && heaviest.get(path.as_path()).is_none_or(|&child| child * 4 < folder.size * 3))
        .map(|(path, folder)| BigFolder {
            path: path.display().to_string(),
            size: folder.size,
            files: folder.files,
            newest: folder.newest,
        })
        .collect();
    big.sort_by_key(|f| std::cmp::Reverse(f.size));
    big.truncate(MAX_FOLDERS);
    report.folders = big;

    report.items.sort_by_key(|i| std::cmp::Reverse(i.size));
    report.items.truncate(MAX_ITEMS);
    report.cancelled = control.is_cancelled();
    report.elapsed = started.elapsed().as_secs_f64();
    progress.current.clear();
    emit(&progress);
    Ok(report)
}

/// Met à la corbeille les fichiers choisis par l'utilisateur, seulement s'ils sont restés tels
/// que l'analyse les a vus : même taille, même date. Les autres sont laissés en place.
pub fn trash_items(items: &[CleanupItem], owner: Option<isize>) -> Result<TrashOutcome> {
    let mut outcome = TrashOutcome::default();
    let mut verified = Vec::new();
    for item in items {
        let path = Path::new(&item.path);
        let unchanged = fs::symlink_metadata(path).is_ok_and(|m| m.is_file() && m.len() == item.size && modified_secs(&m) == item.modified);
        if unchanged {
            verified.push(path.to_path_buf());
            outcome.trashed.push(item.path.clone());
            outcome.bytes += item.size;
        } else {
            outcome.skipped.push(item.path.clone());
        }
    }
    if !verified.is_empty() {
        outcome.holding = crate::recycle::recycle(&verified, owner)?.holding;
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn aged(path: &Path, days: u64) {
        let file = fs::File::options().write(true).open(path).unwrap();
        file.set_modified(SystemTime::now() - Duration::from_secs(days * 86_400)).unwrap();
    }

    fn write(path: &Path, size: usize, days: u64) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, vec![7u8; size]).unwrap();
        aged(path, days);
    }

    #[test]
    fn versions_are_stripped_from_product_names() {
        assert_eq!(product_key("VLC-3.0.20-win64.exe"), product_key("vlc-3.0.21-win64.exe"));
        assert_eq!(product_key("Chud Copy_1.7.0_x64-setup.exe"), product_key("Chud Copy_1.7.1_x64-setup.exe"));
        assert_ne!(product_key("vlc-3.0.20-win64.exe"), product_key("gimp-2.10-setup.exe"));
    }

    #[test]
    fn findings_follow_age_and_kind() {
        let lab = tempfile::tempdir().unwrap();
        let root = lab.path().join("Downloads");
        write(&root.join("vlc-3.0.20-win64.exe"), 10, 5);
        write(&root.join("vlc-3.0.21-win64.exe"), 10, 1);
        write(&root.join("old-setup.msi"), 10, 90);
        write(&root.join("fresh-setup.msi"), 10, 3);
        write(&root.join("film.mkv.crdownload"), 10, 5);
        write(&root.join("rapport.pdf"), 10, 400);
        write(&root.join("recent.pdf"), 10, 10);
        write(&root.join("~$lettre.docx"), 10, 30);
        write(&root.join("sous/notes.tmp"), 10, 1);
        let report = scan_with(&[root.clone()], &CleanupOptions::default(), None, &[], &Control::default(), &mut |_| {}).unwrap();
        let kind = |name: &str| report.items.iter().find(|i| i.path.ends_with(name)).map(|i| i.kind.clone());
        assert_eq!(kind("vlc-3.0.20-win64.exe").as_deref(), Some("superseded"));
        assert_eq!(kind("vlc-3.0.21-win64.exe"), None, "la version la plus récente reste");
        assert_eq!(kind("old-setup.msi").as_deref(), Some("installer"));
        assert_eq!(kind("fresh-setup.msi"), None);
        assert_eq!(kind("film.mkv.crdownload").as_deref(), Some("partial"));
        assert_eq!(kind("rapport.pdf").as_deref(), Some("download"));
        assert_eq!(kind("recent.pdf"), None);
        assert_eq!(kind("~$lettre.docx").as_deref(), Some("temp"));
        assert_eq!(kind("notes.tmp"), None, "trop récent");
        assert_eq!(report.scanned, 9);
    }

    #[test]
    fn precise_signals_win_over_the_temporary_folder() {
        let temps = vec![PathBuf::from("/tmp")];
        assert_eq!(classify(Path::new("/tmp/lab/Downloads/old-setup.msi"), None, &temps), Some("installer"));
        assert_eq!(classify(Path::new("/tmp/lab/Downloads/facture.pdf"), None, &temps), Some("download"));
        assert_eq!(classify(Path::new("/tmp/session/cache.bin"), None, &temps), Some("temp"));
        assert_eq!(classify(Path::new("/home/moi/notes.txt"), None, &temps), None);
    }

    #[test]
    fn big_folders_point_to_the_heavy_subfolder() {
        let lab = tempfile::tempdir().unwrap();
        let root = lab.path().join("data");
        write(&root.join("films/a.bin"), 900, 1);
        write(&root.join("films/b.bin"), 900, 1);
        write(&root.join("docs/c.txt"), 50, 1);
        let options = CleanupOptions {
            big_folder: 1000,
            ..CleanupOptions::default()
        };
        let report = scan_with(&[root.clone()], &options, None, &[], &Control::default(), &mut |_| {}).unwrap();
        assert_eq!(report.folders.len(), 1);
        assert!(report.folders[0].path.ends_with("films"));
        assert_eq!(report.folders[0].size, 1800);
    }

    #[test]
    fn nothing_changed_since_the_scan_is_the_only_thing_trashed() {
        let lab = tempfile::tempdir().unwrap();
        let root = lab.path().join("Downloads");
        write(&root.join("old.msi"), 10, 90);
        write(&root.join("changed.msi"), 10, 90);
        let report = scan_with(&[root.clone()], &CleanupOptions::default(), None, &[], &Control::default(), &mut |_| {}).unwrap();
        assert_eq!(report.items.len(), 2);
        // Le fichier change entre l'analyse et la confirmation : il doit rester.
        fs::write(root.join("changed.msi"), vec![1u8; 20]).unwrap();
        // Seuls des fichiers à écarter sont passés : le test ne touche jamais à la vraie corbeille.
        let mut stale: Vec<CleanupItem> = report.items.iter().filter(|i| i.path.ends_with("changed.msi")).cloned().collect();
        stale.push(CleanupItem {
            path: root.join("absent.msi").display().to_string(),
            kind: "installer".into(),
            size: 1,
            modified: 0,
        });
        let outcome = trash_items(&stale, None).unwrap();
        assert_eq!(outcome.skipped.len(), 2);
        assert!(outcome.trashed.is_empty());
        assert!(root.join("changed.msi").exists());
        assert!(root.join("old.msi").exists());
    }
}
