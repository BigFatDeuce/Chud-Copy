//! Seul dans son binaire : modifier l'environnement toucherait tous les tests d'un même processus.

use std::fs;
use std::path::Path;

use chud_core::engine::rclone::Rclone;
use chud_core::engine::{Control, Engine, JobSpec, JobStatus};
use chud_core::model::{Filters, Mode, Options};

#[test]
fn engine_environment_cannot_change_an_operation() {
    let name = if cfg!(windows) {
        "rclone-x86_64-pc-windows-msvc.exe"
    } else {
        "rclone-x86_64-unknown-linux-gnu"
    };
    let binary = Path::new(env!("CARGO_MANIFEST_DIR")).join("../binaries").join(name);
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::write(dir.path().join("src/fichier.txt"), "contenu").unwrap();

    std::env::set_var("RCLONE_DRY_RUN", "true");
    let engine = Rclone::new(binary, dir.path().join("tmp"));
    let job = JobSpec {
        id: "l".into(),
        source: dir.path().join("src"),
        target: dir.path().join("dst"),
        mode: Mode::Update,
        options: Options::default(),
        filters: Filters::default(),
        max_delete: None,
        archive_stamp: "essai".into(),
    };
    let outcome = engine.transfer(&job, &Control::default(), &mut |_| {}).unwrap();

    assert_eq!(outcome.status, JobStatus::Completed);
    assert!(dir.path().join("dst/fichier.txt").exists());
}
