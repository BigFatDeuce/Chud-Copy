//! Restauration des attributs après copie, au mieux de ce que permet le disque.
//!
//! `--metadata` de rclone fait échouer la copie d'un fichier quand son propriétaire ne peut pas
//! être rétabli, ce qui arrive dès qu'un utilisateur normal copie le fichier d'un autre. Un attribut
//! ne doit jamais coûter un fichier : la restauration se fait donc ici, une fois le fichier en
//! sécurité, et un échec est seulement compté.
//! L'attribut lecture seule de Windows n'est pas repris : il empêcherait de remplacer ce fichier
//! à la prochaine mise à jour.

use std::fs::{self, Metadata};
use std::path::{Path, PathBuf};

fn apply(from: &Metadata, to_path: &Path, to: &Metadata) -> bool {
    imp::apply(from, to_path, to)
}

#[cfg(unix)]
mod imp {
    use std::fs::{self, Metadata, Permissions};
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    use std::path::Path;

    pub fn apply(from: &Metadata, to_path: &Path, to: &Metadata) -> bool {
        let mut ok = true;
        // Les bits setuid, setgid et sticky ne voyagent pas avec une simple copie.
        let mode = from.mode() & 0o777;
        if to.mode() & 0o777 != mode {
            ok &= fs::set_permissions(to_path, Permissions::from_mode(mode)).is_ok();
        }
        if unsafe { libc::geteuid() } == 0 && (from.uid(), from.gid()) != (to.uid(), to.gid()) {
            ok &= std::os::unix::fs::lchown(to_path, Some(from.uid()), Some(from.gid())).is_ok();
        }
        ok
    }
}

#[cfg(windows)]
mod imp {
    use std::fs::Metadata;
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::MetadataExt;
    use std::path::Path;

    use windows_sys::Win32::Storage::FileSystem::{SetFileAttributesW, FILE_ATTRIBUTE_ARCHIVE, FILE_ATTRIBUTE_HIDDEN};

    pub fn apply(from: &Metadata, to_path: &Path, to: &Metadata) -> bool {
        const KEPT: u32 = FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_ARCHIVE;
        let wanted = (to.file_attributes() & !KEPT) | (from.file_attributes() & KEPT);
        if wanted == to.file_attributes() {
            return true;
        }
        let mut wide: Vec<u16> = extended(to_path).encode_wide().chain(Some(0)).collect();
        wide.shrink_to_fit();
        unsafe { SetFileAttributesW(wide.as_ptr(), wanted) != 0 }
    }

    /// Le préfixe `\\?\` lève la limite de 260 caractères de cette API.
    fn extended(path: &Path) -> std::ffi::OsString {
        let text = path.as_os_str();
        if text.to_string_lossy().starts_with(r"\\") {
            return text.to_os_string();
        }
        let mut out = std::ffi::OsString::from(r"\\?\");
        out.push(text);
        out
    }
}

/// Applique les attributs de la source aux fichiers écrits. Renvoie le nombre d'échecs.
pub fn restore(source: &Path, target: &Path, written: &[String]) -> usize {
    let mut failures = 0;
    for rel in written {
        let relative: PathBuf = rel.split('/').collect();
        let from = source.join(&relative);
        let to = target.join(&relative);
        let (Ok(from_meta), Ok(to_meta)) = (fs::symlink_metadata(&from), fs::symlink_metadata(&to)) else {
            continue;
        };
        if !from_meta.is_file() || !to_meta.is_file() {
            continue;
        }
        if !apply(&from_meta, &to, &to_meta) {
            failures += 1;
        }
    }
    failures
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn permission_bits_follow_the_source() {
        let dir = tempfile::tempdir().unwrap();
        let (a, b) = (dir.path().join("a"), dir.path().join("b"));
        fs::create_dir_all(&a).unwrap();
        fs::create_dir_all(&b).unwrap();
        fs::write(a.join("f"), "x").unwrap();
        fs::write(b.join("f"), "x").unwrap();
        fs::set_permissions(a.join("f"), fs::Permissions::from_mode(0o640)).unwrap();
        assert_eq!(restore(&a, &b, &["f".to_string()]), 0);
        assert_eq!(fs::metadata(b.join("f")).unwrap().permissions().mode() & 0o777, 0o640);
    }

    #[test]
    fn a_missing_file_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(restore(dir.path(), dir.path(), &["absent".to_string()]), 0);
    }
}
