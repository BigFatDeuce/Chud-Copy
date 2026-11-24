//! Mise à l'écart récupérable des fichiers choisis par l'utilisateur.
//!
//! La corbeille du système n'existe que sur les disques locaux. Sur un partage réseau, un NAS ou
//! une clé USB, Windows supprime définitivement après une confirmation qui peut rester cachée
//! derrière la fenêtre, et Linux recopie le fichier entier dans la corbeille de l'utilisateur, à
//! travers le réseau. Dans ces deux cas l'opération semblait figée. Ces emplacements reçoivent donc
//! leur propre corbeille, un dossier sur le même volume : le déplacement y est un simple
//! renommage, instantané, et le fichier reste récupérable.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::filters::HOLDING_DIR;
use crate::{fail, Result};

#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Recycled {
    /// Chemins effectivement retirés de leur place.
    pub moved: Vec<String>,
    /// Corbeilles Chud Copy utilisées, là où le système n'en a pas.
    pub holding: Vec<String>,
}

/// Où part un fichier retiré.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Bin {
    System,
    /// Racine du volume qui reçoit la corbeille Chud Copy.
    Holding(PathBuf),
}

#[cfg(windows)]
fn bin_of(path: &Path) -> Bin {
    use crate::drives::{drive_type, mapped_share, volume_root};
    const DRIVE_FIXED: u32 = 3;
    let parent = path.parent().unwrap_or(path);
    let Some(root) = volume_root(parent) else {
        return Bin::Holding(parent.to_path_buf());
    };
    let network = root.starts_with(r"\\") || mapped_share(&root).is_some();
    if !network && drive_type(&root) == DRIVE_FIXED {
        Bin::System
    } else {
        Bin::Holding(PathBuf::from(root))
    }
}

#[cfg(unix)]
fn bin_of(path: &Path) -> Bin {
    use crate::drives::{is_network, mount_of, mounts};
    let parent = path.parent().unwrap_or(path);
    match mount_of(parent, &mounts()) {
        Some(mount) if is_network(&mount.file_system) => Bin::Holding(PathBuf::from(mount.mount)),
        _ => Bin::System,
    }
}

#[cfg(not(any(windows, unix)))]
fn bin_of(_path: &Path) -> Bin {
    Bin::System
}

fn stamp() -> String {
    chrono::Local::now().format("%Y-%m-%d %Hh%M %S").to_string()
}

/// Chemin relatif d'un fichier sous une racine, sans préfixe de lecteur.
fn relative(path: &Path, root: &Path) -> PathBuf {
    match path.strip_prefix(root) {
        Ok(rel) => rel.to_path_buf(),
        Err(_) => path.components().filter(|c| matches!(c, std::path::Component::Normal(_))).collect(),
    }
}

/// Un nom libre : « nom (2).ext » si le premier est pris.
fn free_name(target: PathBuf) -> PathBuf {
    if fs::symlink_metadata(&target).is_err() {
        return target;
    }
    let stem = target.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = target.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (2..)
        .map(|n| target.with_file_name(format!("{stem} ({n}){ext}")))
        .find(|candidate| fs::symlink_metadata(candidate).is_err())
        .expect("un nom libre finit toujours par exister")
}

/// Déplace un fichier dans la corbeille Chud Copy de son volume, en gardant son arborescence.
/// Si la racine du partage est en lecture seule, la corbeille est créée à côté du fichier.
fn hold(path: &Path, root: &Path, stamp: &str) -> std::io::Result<PathBuf> {
    let attempts = [
        (root.join(HOLDING_DIR).join(stamp), relative(path, root)),
        (
            path.parent().unwrap_or(root).join(HOLDING_DIR).join(stamp),
            PathBuf::from(path.file_name().unwrap_or_default()),
        ),
    ];
    let mut last = None;
    for (bin, rel) in attempts {
        let target = bin.join(&rel);
        let Some(dir) = target.parent() else { continue };
        if let Err(error) = fs::create_dir_all(dir) {
            last = Some(error);
            continue;
        }
        match fs::rename(path, free_name(target)) {
            Ok(()) => return Ok(bin),
            Err(error) => last = Some(error),
        }
    }
    Err(last.unwrap_or_else(|| std::io::Error::other("corbeille impossible")))
}

/// Retire les fichiers vers la corbeille adaptée à leur emplacement. `owner` est la fenêtre qui
/// porte un éventuel avertissement du système, pour qu'il ne reste jamais caché.
/// S'arrête à la première erreur ; ce qui a déjà été retiré est rapporté.
pub fn recycle(paths: &[PathBuf], owner: Option<isize>) -> Result<Recycled> {
    let mut out = Recycled::default();
    let mut system = Vec::new();
    let stamp = stamp();
    for path in paths {
        match bin_of(path) {
            Bin::System => system.push(path.clone()),
            Bin::Holding(root) => match hold(path, &root, &stamp) {
                Ok(bin) => {
                    let bin = bin.display().to_string();
                    if !out.holding.contains(&bin) {
                        out.holding.push(bin);
                    }
                    out.moved.push(path.display().to_string());
                }
                Err(error) => return fail(format!("{} : {error}", path.display())),
            },
        }
    }
    if !system.is_empty() {
        system_bin(&system, owner)?;
        out.moved.extend(system.iter().map(|p| p.display().to_string()));
    }
    Ok(out)
}

#[cfg(windows)]
fn system_bin(paths: &[PathBuf], owner: Option<isize>) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE};
    use windows::Win32::UI::Shell::{FileOperation, IFileOperation, IShellItem, SHCreateItemFromParsingName, FOF_ALLOWUNDO, FOF_NO_UI, FOF_WANTNUKEWARNING};

    let failed = |e: windows::core::Error| crate::CoreError::Message(e.message().to_string());
    // SAFETY : COM est initialisé pour ce fil et libéré à la fin ; chaque chaîne est terminée par zéro.
    unsafe {
        let initialized = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE).is_ok();
        let result = (|| -> Result<()> {
            let operation: IFileOperation = CoCreateInstance(&FileOperation, None, CLSCTX_ALL).map_err(failed)?;
            // Un fichier trop gros pour la corbeille déclenche une demande de suppression définitive :
            // elle s'affiche devant Chud Copy, et un refus laisse le fichier en place.
            operation.SetOperationFlags(FOF_NO_UI | FOF_ALLOWUNDO | FOF_WANTNUKEWARNING).map_err(failed)?;
            if let Some(owner) = owner {
                operation.SetOwnerWindow(HWND(owner as *mut _)).map_err(failed)?;
            }
            for path in paths {
                let text = path.to_string_lossy();
                let plain = text.strip_prefix(r"\\?\").unwrap_or(&text);
                let wide: Vec<u16> = std::ffi::OsStr::new(plain).encode_wide().chain(Some(0)).collect();
                let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None).map_err(failed)?;
                operation.DeleteItem(&item, None).map_err(failed)?;
            }
            operation.PerformOperations().map_err(failed)?;
            if operation.GetAnyOperationsAborted().map_err(failed)?.as_bool() {
                return crate::fail_text("err.trash-aborted");
            }
            Ok(())
        })();
        if initialized {
            CoUninitialize();
        }
        result
    }
}

#[cfg(not(windows))]
fn system_bin(paths: &[PathBuf], _owner: Option<isize>) -> Result<()> {
    trash::delete_all(paths).map_err(|e| crate::CoreError::Message(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holding_keeps_the_tree_and_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::create_dir_all(root.join("a/b")).unwrap();
        fs::write(root.join("a/b/f.txt"), "1").unwrap();
        let bin = hold(&root.join("a/b/f.txt"), root, "s").unwrap();
        assert_eq!(bin, root.join(HOLDING_DIR).join("s"));
        assert_eq!(fs::read_to_string(bin.join("a/b/f.txt")).unwrap(), "1");
        assert!(!root.join("a/b/f.txt").exists());

        fs::write(root.join("a/b/f.txt"), "2").unwrap();
        hold(&root.join("a/b/f.txt"), root, "s").unwrap();
        assert_eq!(fs::read_to_string(bin.join("a/b/f.txt")).unwrap(), "1");
        assert_eq!(fs::read_to_string(bin.join("a/b/f (2).txt")).unwrap(), "2");
    }

    #[test]
    fn a_file_outside_the_root_still_has_a_place() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("x.bin"), "x").unwrap();
        let bin = hold(&dir.path().join("x.bin"), other.path(), "s").unwrap();
        assert!(bin.starts_with(other.path()) || bin.starts_with(dir.path()));
        assert!(!dir.path().join("x.bin").exists());
    }

    /// Sur un vrai partage réseau de test : CHUD_TEST_SHARE=<dossier sur le partage>, vide et jetable.
    #[test]
    #[ignore]
    fn a_network_share_gets_an_instant_holding_bin() {
        let Some(share) = std::env::var_os("CHUD_TEST_SHARE").map(PathBuf::from) else { return };
        let dir = share.join("recycle-test");
        fs::create_dir_all(dir.join("sous")).unwrap();
        let big = dir.join("sous/gros.bin");
        fs::write(&big, vec![5u8; 64 << 20]).unwrap();
        assert!(crate::drives::is_network_path(&big));
        assert!(matches!(bin_of(&big), Bin::Holding(_)));
        let started = std::time::Instant::now();
        let out = recycle(std::slice::from_ref(&big), None).unwrap();
        assert!(started.elapsed() < std::time::Duration::from_secs(3), "{:?}", started.elapsed());
        assert_eq!(out.moved.len(), 1);
        assert!(!big.exists());
        let held = PathBuf::from(&out.holding[0]);
        assert!(held.to_string_lossy().contains(HOLDING_DIR));
        let kept = walkdir::WalkDir::new(&held).into_iter().flatten().find(|e| e.file_name() == "gros.bin").unwrap();
        assert_eq!(fs::metadata(kept.path()).unwrap().len(), 64 << 20);
    }

    #[test]
    fn a_local_temporary_folder_uses_the_system_bin() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("f");
        fs::write(&file, "x").unwrap();
        assert_eq!(bin_of(&file), Bin::System);
    }
}
