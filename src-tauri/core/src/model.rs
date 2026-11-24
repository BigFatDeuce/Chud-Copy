//! Modèle persistant des tâches.
//!
//! Chaque champ a une valeur par défaut : un fichier écrit par une ancienne version se relit
//! sans perte après une mise à jour de l'application.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const TASK_SCHEMA: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Mode {
    /// Copie les fichiers manquants ou modifiés, ne supprime rien.
    #[default]
    Update,
    /// Recopie tous les fichiers, même ceux qui semblent identiques.
    Full,
    /// Ajoute uniquement les fichiers absents, ne remplace jamais rien.
    AddOnly,
    /// La destination devient l'image exacte de la source, suppressions comprises.
    Mirror,
    /// Comme le miroir, mais ce qui disparaîtrait est déplacé dans une archive datée.
    MirrorArchive,
}

impl Mode {
    /// Vrai quand l'opération peut retirer ou remplacer des fichiers absents de la source.
    pub fn removes_from_destination(self) -> bool {
        matches!(self, Mode::Mirror | Mode::MirrorArchive)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Symlinks {
    #[default]
    Skip,
    Follow,
    Preserve,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum Compare {
    #[default]
    SizeAndTime,
    Checksum,
    SizeOnly,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Options {
    pub empty_dirs: bool,
    pub include_hidden: bool,
    pub symlinks: Symlinks,
    pub preserve_metadata: bool,
    pub compare: Compare,
    pub skip_newer: bool,
    pub verify_after: bool,
    pub retries: u32,
    pub transfers: u32,
    pub multi_thread: bool,
    pub parallel_volumes: bool,
    /// Accord explicite pour réunir plusieurs sources dans un même dossier.
    pub allow_merge: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            empty_dirs: true,
            include_hidden: true,
            symlinks: Symlinks::Skip,
            preserve_metadata: true,
            compare: Compare::SizeAndTime,
            skip_newer: false,
            verify_after: false,
            retries: 3,
            transfers: 4,
            multi_thread: false,
            parallel_volumes: true,
            allow_merge: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Filters {
    pub include_files: Vec<String>,
    pub exclude_files: Vec<String>,
    pub include_dirs: Vec<String>,
    pub exclude_dirs: Vec<String>,
    pub include_extensions: Vec<String>,
    pub exclude_extensions: Vec<String>,
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    /// Date au format AAAA-MM-JJ.
    pub newer_than: Option<String>,
    pub older_than: Option<String>,
    pub exclude_system: bool,
    /// Écarte les fichiers sans valeur à sauvegarder : profils Windows, caches, temporaires.
    pub exclude_junk: bool,
}

impl Default for Filters {
    fn default() -> Self {
        Self {
            include_files: Vec::new(),
            exclude_files: Vec::new(),
            include_dirs: Vec::new(),
            exclude_dirs: Vec::new(),
            include_extensions: Vec::new(),
            exclude_extensions: Vec::new(),
            min_size: None,
            max_size: None,
            newer_than: None,
            older_than: None,
            exclude_system: true,
            exclude_junk: false,
        }
    }
}

impl Filters {
    /// Vrai quand l'utilisateur a réellement restreint les fichiers concernés.
    pub fn restricts_files(&self) -> bool {
        self.exclude_junk
            || !self.include_files.is_empty()
            || !self.exclude_files.is_empty()
            || !self.include_dirs.is_empty()
            || !self.exclude_dirs.is_empty()
            || !self.include_extensions.is_empty()
            || !self.exclude_extensions.is_empty()
            || self.min_size.is_some()
            || self.max_size.is_some()
            || self.newer_than.is_some()
            || self.older_than.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Endpoint {
    pub id: String,
    pub path: String,
    /// Le disque reconnu à cet endroit, pour refuser une opération sur un autre disque.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity: Option<crate::drives::DriveIdentity>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Link {
    pub id: String,
    pub source_id: String,
    pub destination_id: String,
    /// Sous-dossier relatif créé dans la destination, vide pour y copier directement.
    pub subfolder: String,
    pub enabled: bool,
}

impl Default for Link {
    fn default() -> Self {
        Self {
            id: String::new(),
            source_id: String::new(),
            destination_id: String::new(),
            subfolder: String::new(),
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Task {
    pub schema: u32,
    pub id: String,
    pub name: String,
    pub description: String,
    pub sources: Vec<Endpoint>,
    pub destinations: Vec<Endpoint>,
    pub links: Vec<Link>,
    pub mode: Mode,
    pub options: Options,
    pub filters: Filters,
    /// Couleur du cadre de la tâche, au format #rrggbb. Aucune : couleur du thème.
    pub color: Option<String>,
    /// Rappel affiché quand la tâche n'a pas été lancée depuis ce nombre de jours. Ne lance jamais rien.
    pub reminder_days: Option<u32>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Default for Task {
    fn default() -> Self {
        Self {
            schema: TASK_SCHEMA,
            id: String::new(),
            name: String::new(),
            description: String::new(),
            sources: Vec::new(),
            destinations: Vec::new(),
            links: Vec::new(),
            mode: Mode::Update,
            options: Options::default(),
            filters: Filters::default(),
            color: None,
            reminder_days: None,
            created_at: 0,
            updated_at: 0,
        }
    }
}

/// Une liaison dont les chemins ont été résolus.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedLink {
    pub id: String,
    pub source: PathBuf,
    pub destination_root: PathBuf,
    pub target: PathBuf,
    pub enabled: bool,
}

impl Task {
    pub fn resolve(&self, link: &Link) -> Option<ResolvedLink> {
        let source = self.sources.iter().find(|s| s.id == link.source_id)?;
        let destination = self.destinations.iter().find(|d| d.id == link.destination_id)?;
        let root = PathBuf::from(&destination.path);
        let target = if link.subfolder.trim().is_empty() {
            root.clone()
        } else {
            root.join(Path::new(link.subfolder.trim()))
        };
        Some(ResolvedLink {
            id: link.id.clone(),
            source: PathBuf::from(&source.path),
            destination_root: root,
            target,
            enabled: link.enabled,
        })
    }

    pub fn resolved_links(&self) -> Vec<ResolvedLink> {
        self.links.iter().filter_map(|l| self.resolve(l)).collect()
    }
}
