//! Garde d'identité des disques : reconnaît le disque qui porte une source ou une destination.
//!
//! Une lettre de lecteur (E:, F:) ou un point de montage (/media/moi/SAUVEGARDE) peut désigner un
//! autre disque d'un jour à l'autre : on branche une autre clé, Windows réattribue les lettres, ou
//! le disque de sauvegarde n'est pas monté et le dossier retombe sur le disque système. Un miroir
//! lancé dans ces conditions effacerait ou remplirait le mauvais disque. Chaque source et chaque
//! destination mémorise donc l'identité de son disque (numéro de série du volume, UUID), et toute
//! opération est refusée si le disque trouvé n'est plus le même, jusqu'à confirmation explicite.
//!
//! Rien ici n'écrit sur les disques : l'identité est lue, jamais modifiée.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::{Endpoint, Task};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct DriveIdentity {
    /// Numéro de série du volume (Windows), UUID du système de fichiers ou partage réseau (Linux).
    pub id: String,
    pub label: String,
    pub file_system: String,
    /// Racine du volume : « E:\ », « /media/moi/SAUVEGARDE », « \\nas\partage\ ».
    pub mount: String,
    /// « local », « removable », « network » ou « unknown ».
    pub kind: String,
    pub total: u64,
    pub learned_at: i64,
}

impl DriveIdentity {
    /// Nom lisible : l'étiquette du volume, sinon sa racine.
    pub fn name(&self) -> String {
        if self.label.trim().is_empty() {
            self.mount.clone()
        } else {
            format!("{} ({})", self.label.trim(), self.mount)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum DriveCheck {
    /// Aucune identité mémorisée pour l'instant.
    Unknown,
    /// Le disque attendu est bien là.
    Same,
    /// Le disque attendu est introuvable : débranché, réseau injoignable.
    Absent,
    /// Un autre disque se trouve à cet endroit.
    Different { found: DriveIdentity },
}

/// Le dossier existant le plus proche : une destination peut ne pas encore exister.
fn existing_ancestor(path: &Path) -> Option<PathBuf> {
    let mut current = Some(path);
    while let Some(candidate) = current {
        if candidate.as_os_str().is_empty() {
            return None;
        }
        if std::fs::metadata(candidate).is_ok() {
            return Some(candidate.to_path_buf());
        }
        current = candidate.parent();
    }
    None
}

/// L'identité du disque qui porte `path`, ou de son dossier existant le plus proche.
/// `None` si aucun disque ne répond à cet endroit.
pub fn identify(path: &Path) -> Option<DriveIdentity> {
    let anchor = existing_ancestor(path)?;
    let mut identity = platform::identify(&anchor)?;
    identity.learned_at = crate::now_ms();
    Some(identity)
}

/// Compare le disque actuel à celui mémorisé. Ne bloque pas sur un disque qui ne répond pas :
/// l'appelant ne vérifie que des chemins déjà sondés.
pub fn check(remembered: Option<&DriveIdentity>, path: &Path) -> DriveCheck {
    let Some(remembered) = remembered else {
        return DriveCheck::Unknown;
    };
    match identify(path) {
        None => DriveCheck::Absent,
        Some(found) if same_drive(remembered, &found) => DriveCheck::Same,
        Some(found) => DriveCheck::Different { found },
    }
}

/// Comme `check`, sans jamais attendre plus de quelques secondes un partage réseau injoignable.
pub fn check_in_time(remembered: Option<&DriveIdentity>, path: &Path) -> DriveCheck {
    let Some(remembered) = remembered.cloned() else {
        return DriveCheck::Unknown;
    };
    let owned = path.to_path_buf();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(check(Some(&remembered), &owned));
    });
    rx.recv_timeout(std::time::Duration::from_secs(4)).unwrap_or(DriveCheck::Absent)
}

/// Racine et espace libre du volume d'un chemin, ou de son dossier existant le plus proche,
/// en quatre secondes au plus. Sert aux partages réseau, absents de la liste des disques.
pub fn free_space_in_time(path: &Path) -> Option<(String, u64)> {
    let owned = path.to_path_buf();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(existing_ancestor(&owned).and_then(|anchor| platform::free_space(&anchor)));
    });
    rx.recv_timeout(std::time::Duration::from_secs(4)).ok().flatten()
}

/// Vrai pour un partage réseau (SMB, NFS, SSHFS…), où chaque accès coûte un aller-retour.
pub fn is_network_path(path: &Path) -> bool {
    #[cfg(windows)]
    {
        const DRIVE_REMOTE: u32 = 4;
        match volume_root(path) {
            Some(root) => root.starts_with(r"\\") || mapped_share(&root).is_some() || drive_type(&root) == DRIVE_REMOTE,
            None => path.to_string_lossy().starts_with(r"\\"),
        }
    }
    #[cfg(unix)]
    {
        mount_of(path, &mounts()).is_some_and(|m| is_network(&m.file_system))
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = path;
        false
    }
}

/// Deux identités désignent-elles le même disque ? Le numéro de série fait foi ; sans lui,
/// l'étiquette, le système de fichiers, la taille et la racine doivent tous correspondre.
pub fn same_drive(a: &DriveIdentity, b: &DriveIdentity) -> bool {
    if !a.id.is_empty() || !b.id.is_empty() {
        return a.id == b.id;
    }
    a.label == b.label && a.file_system == b.file_system && a.total == b.total && a.mount == b.mount
}

/// Mémorise l'identité des disques encore inconnus d'une tâche, seulement quand le chemin existe
/// vraiment : un disque débranché ne doit pas faire mémoriser le disque système à sa place.
/// Renvoie vrai si la tâche a changé.
/// Les disques sont interrogés en parallèle, quatre secondes au plus en tout : enregistrer ou
/// lancer une tâche dont un NAS est éteint n'attend pas le long délai du système. Un disque qui
/// n'a pas répondu sera reconnu une prochaine fois.
pub fn learn_missing(task: &mut Task) -> bool {
    let (tx, rx) = std::sync::mpsc::channel();
    let mut pending = 0;
    for (index, endpoint) in task.sources.iter().chain(task.destinations.iter()).enumerate() {
        if endpoint.identity.is_some() || endpoint.path.trim().is_empty() {
            continue;
        }
        let tx = tx.clone();
        let path = PathBuf::from(&endpoint.path);
        std::thread::spawn(move || {
            let identity = if path.is_dir() { identify(&path) } else { None };
            let _ = tx.send((index, identity));
        });
        pending += 1;
    }
    drop(tx);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
    let mut changed = false;
    while pending > 0 {
        let Ok((index, identity)) = rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())) else {
            break;
        };
        pending -= 1;
        let Some(identity) = identity else { continue };
        let sources = task.sources.len();
        let endpoint = if index < sources { &mut task.sources[index] } else { &mut task.destinations[index - sources] };
        endpoint.identity = Some(identity);
        changed = true;
    }
    changed
}

/// Garde l'identité déjà connue d'un point dont le chemin n'a pas changé, oublie les autres.
pub fn carry_over(task: &mut Task, previous: &Task) {
    let all_previous: Vec<&Endpoint> = previous.sources.iter().chain(previous.destinations.iter()).collect();
    for endpoint in task.sources.iter_mut().chain(task.destinations.iter_mut()) {
        let before = all_previous.iter().find(|p| p.id == endpoint.id);
        endpoint.identity = match before {
            Some(before) if before.path == endpoint.path => endpoint.identity.clone().or_else(|| before.identity.clone()),
            _ => None,
        };
    }
}

#[cfg(windows)]
mod platform {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows_sys::Win32::NetworkManagement::WNet::WNetGetConnectionW;
    use windows_sys::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW, GetVolumeInformationW, GetVolumePathNameW};

    use super::DriveIdentity;

    const DRIVE_REMOVABLE: u32 = 2;
    const DRIVE_REMOTE: u32 = 4;
    const DRIVE_CDROM: u32 = 5;

    fn wide(text: &str) -> Vec<u16> {
        std::ffi::OsStr::new(text).encode_wide().chain(Some(0)).collect()
    }

    fn text(buffer: &[u16]) -> String {
        let end = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
        String::from_utf16_lossy(&buffer[..end])
    }

    /// Racine du volume qui porte un chemin existant : « E:\ » ou « \\nas\partage\ ».
    pub fn volume_root(path: &Path) -> Option<String> {
        let path = wide(&path.to_string_lossy());
        let mut root = vec![0u16; 1024];
        // SAFETY : tampons valides et terminés par zéro, tailles exactes.
        let ok = unsafe { GetVolumePathNameW(path.as_ptr(), root.as_mut_ptr(), root.len() as u32) };
        (ok != 0).then(|| text(&root))
    }

    /// Le partage réseau derrière une lettre de lecteur connectée : « \\nas\partage ».
    pub fn mapped_share(root: &str) -> Option<String> {
        let letter = wide(root.trim_end_matches('\\'));
        let mut share = vec![0u16; 1024];
        let mut length = share.len() as u32;
        // SAFETY : tampons valides, longueur passée par pointeur.
        let status = unsafe { WNetGetConnectionW(letter.as_ptr(), share.as_mut_ptr(), &mut length) };
        (status == 0).then(|| text(&share)).filter(|s| !s.is_empty())
    }

    /// Racine et espace libre du volume qui porte un chemin existant.
    pub fn free_space(path: &Path) -> Option<(String, u64)> {
        let root = volume_root(path)?;
        let wide_root = wide(&root);
        let mut available = 0u64;
        // SAFETY : chaîne terminée par zéro, pointeur valide vers un u64.
        let ok = unsafe { GetDiskFreeSpaceExW(wide_root.as_ptr(), &mut available, std::ptr::null_mut(), std::ptr::null_mut()) };
        (ok != 0).then_some((root, available))
    }

    pub fn drive_type(root: &str) -> u32 {
        let root = wide(root);
        // SAFETY : chaîne terminée par zéro.
        unsafe { GetDriveTypeW(root.as_ptr()) }
    }

    pub fn identify(path: &Path) -> Option<DriveIdentity> {
        let root = volume_root(path)?;
        let wide_root = wide(&root);
        let mut label = vec![0u16; 261];
        let mut fs = vec![0u16; 261];
        let mut serial = 0u32;
        // SAFETY : tampons valides de la taille annoncée ; pointeurs nuls pour les champs ignorés.
        let ok = unsafe {
            GetVolumeInformationW(
                wide_root.as_ptr(),
                label.as_mut_ptr(),
                label.len() as u32,
                &mut serial,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                fs.as_mut_ptr(),
                fs.len() as u32,
            )
        };
        if ok == 0 {
            return None;
        }
        let mut total = 0u64;
        // SAFETY : pointeurs valides vers des u64, les autres sont facultatifs.
        unsafe { GetDiskFreeSpaceExW(wide_root.as_ptr(), std::ptr::null_mut(), &mut total, std::ptr::null_mut()) };
        let kind = match drive_type(&root) {
            DRIVE_REMOVABLE | DRIVE_CDROM => "removable",
            DRIVE_REMOTE => "network",
            _ if root.starts_with("\\\\") => "network",
            _ => "local",
        };
        // Un partage réseau est reconnu à son adresse autant qu'à son numéro de série.
        let id = if kind == "network" {
            let share = if root.starts_with("\\\\") {
                root.trim_end_matches('\\').to_string()
            } else {
                mapped_share(&root).unwrap_or_default()
            };
            format!("{serial:08X}@{}", share.to_lowercase())
        } else {
            format!("{serial:08X}")
        };
        Some(DriveIdentity {
            id,
            label: text(&label),
            file_system: text(&fs),
            mount: root,
            kind: kind.into(),
            total,
            learned_at: 0,
        })
    }
}

#[cfg(unix)]
mod platform {
    use std::path::{Path, PathBuf};

    use super::DriveIdentity;

    /// Une ligne de /proc/self/mountinfo.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Mount {
        pub device: String,
        pub mount: String,
        pub file_system: String,
        pub source: String,
    }

    /// Les espaces et caractères spéciaux sont écrits en octal : « \040 » pour une espace.
    pub fn unescape(text: &str) -> String {
        let bytes = text.as_bytes();
        let mut out = Vec::with_capacity(bytes.len());
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'\\' && i + 3 < bytes.len() && bytes[i + 1..i + 4].iter().all(|b| (b'0'..=b'7').contains(b)) {
                let value = (bytes[i + 1] - b'0') * 64 + (bytes[i + 2] - b'0') * 8 + (bytes[i + 3] - b'0');
                out.push(value);
                i += 4;
            } else {
                out.push(bytes[i]);
                i += 1;
            }
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    pub fn parse_mountinfo(text: &str) -> Vec<Mount> {
        text.lines()
            .filter_map(|line| {
                let (left, right) = line.split_once(" - ")?;
                let left: Vec<&str> = left.split(' ').collect();
                let right: Vec<&str> = right.split(' ').collect();
                Some(Mount {
                    device: left.get(2)?.to_string(),
                    mount: unescape(left.get(4)?),
                    file_system: right.first()?.to_string(),
                    source: unescape(right.get(1).copied().unwrap_or("")),
                })
            })
            .collect()
    }

    pub fn mounts() -> Vec<Mount> {
        std::fs::read_to_string("/proc/self/mountinfo").map(|t| parse_mountinfo(&t)).unwrap_or_default()
    }

    /// Le montage qui porte un chemin existant : même périphérique, point de montage le plus long.
    pub fn mount_of(path: &Path, mounts: &[Mount]) -> Option<Mount> {
        use std::os::unix::fs::MetadataExt;
        let dev = std::fs::metadata(path).ok()?.dev();
        let id = format!("{}:{}", libc_major(dev), libc_minor(dev));
        let canonical = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let inside = |m: &Mount| canonical.starts_with(&m.mount);
        mounts
            .iter()
            .filter(|m| m.device == id && inside(m))
            .max_by_key(|m| m.mount.len())
            .or_else(|| mounts.iter().filter(|m| inside(m)).max_by_key(|m| m.mount.len()))
            .cloned()
    }

    fn libc_major(dev: u64) -> u64 {
        ((dev >> 32) & 0xffff_f000) | ((dev >> 8) & 0x0000_0fff)
    }

    fn libc_minor(dev: u64) -> u64 {
        ((dev >> 12) & 0xffff_ff00) | (dev & 0x0000_00ff)
    }

    /// Le nom d'un lien de /dev/disk/by-uuid ou by-label qui pointe vers ce périphérique.
    fn link_to(dir: &str, device: &Path) -> Option<String> {
        let target = std::fs::canonicalize(device).ok()?;
        std::fs::read_dir(dir).ok()?.flatten().find_map(|entry| {
            let resolved: PathBuf = std::fs::canonicalize(entry.path()).ok()?;
            (resolved == target).then(|| unescape_udev(&entry.file_name().to_string_lossy()))
        })
    }

    /// udev écrit les caractères spéciaux en hexadécimal : « \x20 » pour une espace.
    fn unescape_udev(text: &str) -> String {
        let mut out = Vec::new();
        let bytes = text.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'\\' && i + 3 < bytes.len() && bytes[i + 1] == b'x' {
                if let Ok(value) = u8::from_str_radix(&text[i + 2..i + 4], 16) {
                    out.push(value);
                    i += 4;
                    continue;
                }
            }
            out.push(bytes[i]);
            i += 1;
        }
        String::from_utf8_lossy(&out).into_owned()
    }

    pub fn is_network(file_system: &str) -> bool {
        matches!(
            file_system,
            "cifs" | "smb3" | "smbfs" | "nfs" | "nfs4" | "fuse.sshfs" | "fuse.gvfsd-fuse" | "fuse.rclone" | "davfs" | "9p"
        )
    }

    /// Point de montage et espace libre du volume qui porte un chemin existant.
    pub fn free_space(path: &Path) -> Option<(String, u64)> {
        let mount = mount_of(path, &mounts())?;
        let c_path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).ok()?;
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY : chemin terminé par zéro, structure de sortie valide.
        if unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) } != 0 {
            return None;
        }
        Some((mount.mount, stat.f_bavail as u64 * stat.f_frsize as u64))
    }

    fn total(path: &Path) -> u64 {
        let Ok(c_path) = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) else {
            return 0;
        };
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        // SAFETY : chemin terminé par zéro, structure de sortie valide.
        if unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) } != 0 {
            return 0;
        }
        stat.f_blocks as u64 * stat.f_frsize as u64
    }

    pub fn identify(path: &Path) -> Option<DriveIdentity> {
        let mount = mount_of(path, &mounts())?;
        let network = is_network(&mount.file_system);
        let device = Path::new(&mount.source);
        let removable = device
            .file_name()
            .and_then(|name| {
                let name = name.to_string_lossy();
                let disk = name.trim_end_matches(|c: char| c.is_ascii_digit()).trim_end_matches('p').to_string();
                std::fs::read_to_string(format!("/sys/block/{disk}/removable"))
                    .ok()
                    .or_else(|| std::fs::read_to_string(format!("/sys/block/{name}/removable")).ok())
            })
            .is_some_and(|v| v.trim() == "1")
            || mount.mount.starts_with("/media/")
            || mount.mount.starts_with("/run/media/");
        let (id, label) = if network {
            (format!("{}:{}", mount.file_system, mount.source.to_lowercase()), String::new())
        } else if mount.source.starts_with("/dev/") {
            let uuid = link_to("/dev/disk/by-uuid", device).unwrap_or_default();
            let label = link_to("/dev/disk/by-label", device).unwrap_or_default();
            (if uuid.is_empty() { mount.source.clone() } else { uuid }, label)
        } else {
            (format!("{}:{}@{}", mount.file_system, mount.source, mount.mount), String::new())
        };
        let label = if label.is_empty() && !network {
            Path::new(&mount.mount)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .filter(|_| mount.mount != "/")
                .unwrap_or_default()
        } else {
            label
        };
        Some(DriveIdentity {
            id,
            label,
            file_system: mount.file_system.clone(),
            mount: mount.mount.clone(),
            kind: if network {
                "network"
            } else if removable {
                "removable"
            } else {
                "local"
            }
            .into(),
            total: total(Path::new(&mount.mount)),
            learned_at: 0,
        })
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn mountinfo_is_parsed_with_escapes() {
            let text = "36 35 98:0 / /mnt/My\\040Disk rw,noatime master:1 - ext4 /dev/sdb1 rw\n\
                        40 25 0:52 / /mnt/nas rw - cifs //nas/backup rw,vers=3.0\n";
            let mounts = parse_mountinfo(text);
            assert_eq!(mounts.len(), 2);
            assert_eq!(mounts[0].mount, "/mnt/My Disk");
            assert_eq!(mounts[0].device, "98:0");
            assert_eq!(mounts[0].file_system, "ext4");
            assert_eq!(mounts[1].source, "//nas/backup");
            assert!(is_network(&mounts[1].file_system));
        }

        #[test]
        fn udev_names_are_unescaped() {
            assert_eq!(unescape_udev("My\\x20Backup"), "My Backup");
        }
    }
}

#[cfg(windows)]
pub use platform::{drive_type, mapped_share, volume_root};
#[cfg(unix)]
pub use platform::{is_network, mount_of, mounts, Mount};

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(id: &str) -> DriveIdentity {
        DriveIdentity {
            id: id.into(),
            label: "SAUVEGARDE".into(),
            file_system: "NTFS".into(),
            mount: "E:\\".into(),
            kind: "removable".into(),
            total: 1000,
            learned_at: 0,
        }
    }

    #[test]
    fn serial_decides() {
        assert!(same_drive(&identity("1234ABCD"), &identity("1234ABCD")));
        assert!(!same_drive(&identity("1234ABCD"), &identity("9999FFFF")));
        // Même étiquette, même taille, autre numéro : un autre disque.
        let mut twin = identity("9999FFFF");
        twin.label = "SAUVEGARDE".into();
        assert!(!same_drive(&identity("1234ABCD"), &twin));
    }

    #[test]
    fn a_temporary_folder_has_a_stable_identity() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("a")).unwrap();
        let first = identify(&dir.path().join("a")).expect("identité");
        // Un dossier qui n'existe pas encore est rattaché au disque de son parent.
        let later = identify(&dir.path().join("a").join("pas-encore-cree")).expect("identité");
        assert!(same_drive(&first, &later));
        assert!(!first.mount.is_empty());
        assert_eq!(check(Some(&first), &dir.path().join("a")), DriveCheck::Same);
        assert_eq!(check(None, dir.path()), DriveCheck::Unknown);
    }

    #[test]
    fn another_drive_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let mut stranger = identify(dir.path()).unwrap();
        stranger.id = "AUTRE-DISQUE".into();
        assert!(matches!(check(Some(&stranger), dir.path()), DriveCheck::Different { .. }));
    }

    #[test]
    fn identities_are_learned_only_for_existing_paths_and_forgotten_when_paths_change() {
        let dir = tempfile::tempdir().unwrap();
        let present = dir.path().join("present");
        std::fs::create_dir_all(&present).unwrap();
        let mut task = Task::default();
        task.sources.push(Endpoint {
            id: "s".into(),
            path: present.display().to_string(),
            identity: None,
        });
        task.destinations.push(Endpoint {
            id: "d".into(),
            path: dir.path().join("absent").display().to_string(),
            identity: None,
        });
        assert!(learn_missing(&mut task));
        assert!(task.sources[0].identity.is_some());
        assert!(task.destinations[0].identity.is_none(), "un chemin absent ne fait rien mémoriser");
        assert!(!learn_missing(&mut task));

        let previous = task.clone();
        let mut edited = task.clone();
        edited.sources[0].identity = None;
        carry_over(&mut edited, &previous);
        assert!(edited.sources[0].identity.is_some(), "même chemin : identité conservée");
        edited.sources[0].path = dir.path().join("ailleurs").display().to_string();
        carry_over(&mut edited, &previous);
        assert!(edited.sources[0].identity.is_none(), "chemin changé : identité oubliée");
    }
}
