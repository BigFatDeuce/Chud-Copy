//! Disques disponibles et espace libre.

use std::path::Path;

use serde::Serialize;
use sysinfo::Disks;

use crate::paths;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Disk {
    pub name: String,
    pub mount: String,
    pub total: u64,
    pub available: u64,
    pub removable: bool,
    pub file_system: String,
}

const PSEUDO_FS: &[&str] = &[
    "overlay",
    "tmpfs",
    "devtmpfs",
    "squashfs",
    "proc",
    "sysfs",
    "cgroup",
    "cgroup2",
    "autofs",
    "fuse.portal",
    "efivarfs",
    "ramfs",
    "9p",
];
const HIDDEN_MOUNTS: &[&str] = &["/boot", "/snap", "/var/lib", "/var/snap", "/run/user", "/etc", "/proc", "/sys", "/dev"];

fn raw() -> Vec<Disk> {
    let Ok(disks) = Disks::new_with_refreshed_list() else {
        return Vec::new();
    };
    disks
        .iter()
        .map(|d| Disk {
            name: d.name().to_string_lossy().into_owned(),
            mount: d.mount_point().to_string_lossy().into_owned(),
            total: d.total_space(),
            available: d.available_space(),
            removable: d.is_removable(),
            file_system: d.file_system().to_string_lossy().into_owned(),
        })
        .collect()
}

/// Les disques à proposer à l'utilisateur, sans pseudo-systèmes ni sous-volumes en double.
pub fn list() -> Vec<Disk> {
    let mut out: Vec<Disk> = Vec::new();
    let mut disks = raw();
    disks.sort_by_key(|d| d.mount.len());
    for disk in disks {
        if disk.total == 0 || PSEUDO_FS.contains(&disk.file_system.as_str()) {
            continue;
        }
        if cfg!(unix) && HIDDEN_MOUNTS.iter().any(|m| disk.mount == *m || disk.mount.starts_with(&format!("{m}/"))) {
            continue;
        }
        // Un sous-volume btrfs (Fedora, Nobara) apparaît une fois par point de montage.
        if cfg!(unix) && out.iter().any(|d| d.name == disk.name) {
            continue;
        }
        out.push(disk);
    }
    out.sort_by(|a, b| a.mount.cmp(&b.mount));
    out
}

/// Le disque qui porte un chemin : le point de montage le plus long qui le contient.
pub fn containing(path: &Path, disks: &[Disk]) -> Option<Disk> {
    let key = paths::key(path);
    disks
        .iter()
        .filter(|d| {
            let mount = paths::key_of_text(&d.mount);
            paths::same(&key, &mount) || paths::inside(&key, &mount)
        })
        .max_by_key(|d| paths::key_of_text(&d.mount).len())
        .cloned()
}

pub fn all() -> Vec<Disk> {
    raw()
}

/// Identifiant stable du disque physique ou logique d'un chemin, pour répartir les opérations.
pub fn volume_key(path: &Path, disks: &[Disk]) -> String {
    match containing(path, disks) {
        Some(disk) => disk.mount,
        None => paths::key_of_text(&path.to_string_lossy()).first().cloned().unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disk(mount: &str) -> Disk {
        Disk {
            name: mount.into(),
            mount: mount.into(),
            total: 10,
            available: 5,
            removable: false,
            file_system: "ext4".into(),
        }
    }

    #[cfg(unix)]
    #[test]
    fn longest_mount_wins() {
        let disks = vec![disk("/"), disk("/mnt/data")];
        assert_eq!(volume_key(Path::new("/mnt/data/backup"), &disks), "/mnt/data");
        assert_eq!(volume_key(Path::new("/mnt/database"), &disks), "/");
    }

    #[cfg(windows)]
    #[test]
    fn drive_letter_is_the_volume() {
        let disks = vec![disk("C:\\"), disk("Z:\\")];
        assert_eq!(volume_key(Path::new(r"Z:\Backup\Photos"), &disks), "Z:\\");
    }
}
