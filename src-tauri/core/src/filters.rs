//! Traduction des filtres de l'utilisateur en règles ordonnées.
//!
//! La première règle qui correspond l'emporte : les exclusions passent donc avant les inclusions.
//! Les règles restent indépendantes du moteur tant qu'elles se limitent à la syntaxe `+ / -`.

use std::path::Path;

use crate::model::{Filters, Options};

/// Dossier d'archive du mode « miroir avec archive », jamais comparé ni supprimé, à quelque
/// niveau qu'il se trouve : une autre tâche peut l'avoir créé dans un sous-dossier de la destination.
/// Placé à la racine du dossier synchronisé, visible et en tête de liste dans l'explorateur.
pub const ARCHIVE_DIR: &str = "_Archives Chud Copy";
/// Nom des archives créées avant la version 1.5, toujours protégées.
pub const LEGACY_ARCHIVE_DIR: &str = ".chudcopy-archive";
/// Corbeille de Chud Copy sur les volumes qui n'en ont pas (partages réseau, NAS, clés USB) :
/// jamais copiée, comparée, analysée ni supprimée par un miroir.
pub const HOLDING_DIR: &str = "_Corbeille Chud Copy";

const SYSTEM_DIRS: &[&str] = &[
    "System Volume Information",
    "$RECYCLE.BIN",
    "$Recycle.Bin",
    ".Trash-*",
    ".Trashes",
    "lost+found",
    "@Recycle",
];

// Fichiers sans valeur à sauvegarder : régénérés par le système ou reste d'une opération inachevée.
// La casse varie selon les systèmes : ces motifs l'ignorent partout. Sont exclus de cette liste,
// volontairement, les types qui peuvent contenir de vraies données (.log, .old, node_modules)
// et les fichiers utilisés par les logiciels de synchronisation (.megaignore).
const JUNK_NAMES: &[&str] = &[
    // Profil Windows, verrouillé pendant l'utilisation et inutile hors de sa session.
    r"ntuser\.dat.*",
    r"ntuser\.ini",
    r"ntuser\.pol",
    r"usrclass\.dat.*",
    // Caches et mémoire virtuelle de Windows.
    r"desktop\.ini",
    r"(eh)?thumbs(_vista)?\.db",
    r"iconcache.*\.db",
    r"thumbcache.*\.db",
    r"pagefile\.sys",
    r"hiberfil\.sys",
    r"swapfile\.sys",
    // macOS et Linux.
    r"\.ds_store",
    r"\._.*",
    r"\.directory",
    r"\.fuse_hidden.*",
    // Verrous et copies de travail des éditeurs.
    r"\.~lock\..*#",
    r".*~",
    // Journaux de Nextcloud et ownCloud.
    r"\.(nextcloud|owncloud)sync\.log",
    r"\._?sync_.*\.db.*",
];
const JUNK_EXTENSIONS: &[&str] = &[
    "tmp",
    "temp",
    "bak",
    "lnk",
    "lock",
    "swp",
    "swo",
    "dmp",
    "crdownload",
    "part",
    "partial",
    "download",
    "opdownload",
    "!qB",
    "aria2",
];
const JUNK_DIRS: &[&str] = &[
    r"\.thumbnails",
    r"\.debris",
    r"\.dropbox\.cache",
    r"\.tmp\.driveupload",
    r"\.tmp\.drivedownload",
    r"\.spotlight-v100",
    r"\.fseventsd",
    r"\.temporaryitems",
    r"\.appledouble",
];

/// La liste des fichiers inutiles telle qu'on la montre à l'utilisateur.
pub fn junk_list() -> Vec<String> {
    let readable = |pattern: &str| {
        pattern
            .replace("(eh)?", "")
            .replace("(_vista)?", "")
            .replace("(nextcloud|owncloud)", "nextcloud")
            .replace(".*", "*")
            .replace(r"\.", ".")
    };
    let mut out: Vec<String> = JUNK_NAMES.iter().map(|name| readable(name)).collect();
    out.push("~$*".into());
    out.extend(JUNK_EXTENSIONS.iter().map(|ext| format!("*.{ext}")));
    out.extend(JUNK_DIRS.iter().map(|dir| format!("{}/", readable(dir))));
    out
}

pub fn rules(filters: &Filters, options: &Options, source: &Path) -> Vec<String> {
    let mut out = vec![format!("- {ARCHIVE_DIR}/**"), format!("- {LEGACY_ARCHIVE_DIR}/**"), format!("- {HOLDING_DIR}/**")];

    if filters.exclude_system {
        for dir in SYSTEM_DIRS {
            out.push(format!("- {dir}/**"));
        }
    }

    if filters.exclude_junk {
        out.extend(JUNK_NAMES.iter().map(|name| format!("- {{{{(?i){name}}}}}")));
        out.push("- ~$*".to_string()); // Verrous d'Office.
        out.extend(JUNK_EXTENSIONS.iter().map(|ext| format!("- {}", extension_glob(ext))));
        out.extend(JUNK_DIRS.iter().map(|dir| format!("- {{{{(?i){dir}}}}}/**")));
    }

    if !options.include_hidden {
        out.push("- .*".to_string());
        out.push("- .*/**".to_string());
        out.extend(hidden_attribute_rules(source));
    }

    for dir in clean(&filters.exclude_dirs) {
        out.push(format!("- {}/**", dir.trim_end_matches(['/', '\\'])));
    }
    for name in clean(&filters.exclude_files) {
        out.push(format!("- {name}"));
    }
    for ext in clean(&filters.exclude_extensions) {
        out.push(format!("- {}", extension_glob(&ext)));
    }

    let mut includes = Vec::new();
    for dir in clean(&filters.include_dirs) {
        includes.push(format!("+ {}/**", dir.trim_end_matches(['/', '\\'])));
    }
    for name in clean(&filters.include_files) {
        includes.push(format!("+ {name}"));
    }
    for ext in clean(&filters.include_extensions) {
        includes.push(format!("+ {}", extension_glob(&ext)));
    }
    if !includes.is_empty() {
        out.append(&mut includes);
        out.push("- **".to_string());
    }
    out
}

/// Dossiers que les outils ne parcourent jamais : ceux du système et ceux de Chud Copy lui-même.
pub(crate) fn is_skipped_dir(name: &str) -> bool {
    name == ARCHIVE_DIR || name == LEGACY_ARCHIVE_DIR || name == HOLDING_DIR || is_system_dir(name)
}

pub(crate) fn is_system_dir(name: &str) -> bool {
    let name = name.to_lowercase();
    SYSTEM_DIRS.iter().any(|dir| {
        let dir = dir.to_lowercase();
        match dir.strip_suffix('*') {
            Some(prefix) => name.starts_with(prefix),
            None => name == dir,
        }
    })
}

/// Compte les fichiers de la source sans passer par le moteur, pour vérifier qu'il les voit tous.
/// Les liens ne sont ni suivis ni comptés : le moteur peut les ignorer légitimement.
/// `None` si `stop` l'a interrompu : le compte serait partiel, donc faux.
pub fn count_source_files(source: &Path, filters: &Filters, stop: &dyn Fn() -> bool) -> Option<u64> {
    let walker = walkdir::WalkDir::new(source).min_depth(1).follow_links(false).into_iter();
    let mut count = 0u64;
    for entry in walker
        .filter_entry(|entry| {
            let name = entry.file_name().to_string_lossy();
            let archive = name == ARCHIVE_DIR || name == LEGACY_ARCHIVE_DIR || name == HOLDING_DIR;
            !archive && !(filters.exclude_system && entry.file_type().is_dir() && is_system_dir(&name))
        })
        .flatten()
    {
        if entry.file_type().is_dir() && stop() {
            return None;
        }
        if entry.file_type().is_file() {
            count += 1;
        }
    }
    (!stop()).then_some(count)
}

fn clean(items: &[String]) -> Vec<String> {
    items
        .iter()
        // Un retour à la ligne pourrait forger une règle supplémentaire.
        .map(|item| item.replace(['\r', '\n'], "").trim().to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

/// Les extensions ne dépendent pas de la casse : `JPG` et `.jpg` désignent les mêmes fichiers.
fn extension_glob(ext: &str) -> String {
    let bare = ext.trim_start_matches("*.").trim_start_matches('.');
    format!("*.{{{{(?i){}}}}}", escape_regex(bare))
}

fn escape_regex(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if r"\.+*?()|[]{}^$".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

#[cfg(windows)]
fn escape_glob(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        if r"\*?[]{}".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// Sous Windows, « caché » est un attribut : aucune règle de nom ne suffit.
#[cfg(windows)]
fn hidden_attribute_rules(source: &Path) -> Vec<String> {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;

    let mut out = Vec::new();
    let mut walker = walkdir::WalkDir::new(source).min_depth(1).follow_links(false).into_iter();
    // Un dossier refusé (« System Volume Information », partage restreint) ne doit pas arrêter
    // la recherche : les fichiers cachés suivants resteraient copiés.
    while let Some(entry) = walker.next() {
        let Ok(entry) = entry else { continue };
        let Ok(meta) = entry.metadata() else { continue };
        if meta.file_attributes() & FILE_ATTRIBUTE_HIDDEN == 0 {
            continue;
        }
        let Ok(rel) = entry.path().strip_prefix(source) else { continue };
        let rel = escape_glob(&rel.to_string_lossy().replace('\\', "/"));
        if entry.file_type().is_dir() {
            out.push(format!("- /{rel}/**"));
            walker.skip_current_dir();
        } else {
            out.push(format!("- /{rel}"));
        }
    }
    out
}

#[cfg(not(windows))]
fn hidden_attribute_rules(_source: &Path) -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exclusions_precede_inclusions_and_catch_all_comes_last() {
        let filters = Filters {
            include_extensions: vec!["JPG".into()],
            exclude_files: vec!["brouillon*".into()],
            ..Filters::default()
        };
        let rules = rules(&filters, &Options::default(), Path::new("."));
        let exclude = rules.iter().position(|r| r == "- brouillon*").unwrap();
        let include = rules.iter().position(|r| r.starts_with("+ ")).unwrap();
        assert!(exclude < include);
        assert_eq!(rules.last().unwrap(), "- **");
    }

    #[test]
    fn no_include_means_no_catch_all() {
        let rules = rules(&Filters::default(), &Options::default(), Path::new("."));
        assert!(!rules.iter().any(|r| r == "- **"));
    }

    #[test]
    fn newlines_cannot_forge_rules() {
        let filters = Filters {
            exclude_files: vec!["a\n+ **".into()],
            ..Filters::default()
        };
        let rules = rules(&filters, &Options::default(), Path::new("."));
        assert!(rules.iter().all(|r| !r.contains('\n')));
    }

    #[test]
    fn the_census_skips_what_the_engine_is_told_to_skip() {
        let dir = tempfile::tempdir().unwrap();
        for rel in [
            "a.txt",
            "sous/b.txt",
            "$RECYCLE.BIN/c.txt",
            ".Trash-1000/d.txt",
            ".chudcopy-archive/e.txt",
            "sous/.chudcopy-archive/2026/f.txt",
            "_Archives Chud Copy/g.txt",
            "sous/_Archives Chud Copy/h.txt",
        ] {
            let path = dir.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "x").unwrap();
        }
        assert_eq!(count_source_files(dir.path(), &Filters::default(), &|| false), Some(2));
        assert_eq!(count_source_files(dir.path(), &Filters::default(), &|| true), None);
        let keep_system = Filters {
            exclude_system: false,
            ..Filters::default()
        };
        assert_eq!(count_source_files(dir.path(), &keep_system, &|| false), Some(4));
    }

    #[test]
    fn extensions_ignore_case_and_dot() {
        assert_eq!(extension_glob(".JPG"), "*.{{(?i)JPG}}");
        assert_eq!(extension_glob("*.tar.gz"), r"*.{{(?i)tar\.gz}}");
    }
}
