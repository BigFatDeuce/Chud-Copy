//! Extinction de l'ordinateur à la fin des copies, sur demande expresse de l'utilisateur.
//!
//! L'interface arme l'extinction par une case à cocher, jamais enregistrée, et laisse une minute
//! pour l'annuler. Ce module ne fait que la demander au système, sans forcer : une application qui
//! a des modifications non enregistrées peut encore la retenir, rien n'est perdu.

use std::process::Command;

use crate::{fail_text, proc, Result};

/// Demande au système d'éteindre l'ordinateur maintenant.
///
/// Sous `CHUD_COPY_FAKE_POWER_OFF`, l'extinction est simulée : un fichier témoin est écrit à ce
/// chemin. Les tests de bout en bout s'en servent, pour qu'aucun test ne puisse éteindre la machine.
pub fn power_off() -> Result<()> {
    if let Some(witness) = std::env::var_os("CHUD_COPY_FAKE_POWER_OFF") {
        std::fs::write(witness, crate::now_ms().to_string())?;
        return Ok(());
    }
    for mut command in commands() {
        proc::prepare(&mut command);
        if let Ok(status) = command.status() {
            if status.success() {
                return Ok(());
            }
        }
    }
    fail_text("err.power-off")
}

#[cfg(windows)]
fn commands() -> Vec<Command> {
    // Sans /f : Windows laisse les applications enregistrer ou refuser, comme une extinction normale.
    let mut command = Command::new("shutdown");
    command.args(["/s", "/t", "0"]);
    vec![command]
}

#[cfg(not(windows))]
fn commands() -> Vec<Command> {
    // systemd passe par logind : l'utilisateur de la session locale peut éteindre sans être root.
    let mut systemd = Command::new("systemctl");
    systemd.arg("poweroff");
    let mut logind = Command::new("gdbus");
    logind.args(["call", "--system", "--dest", "org.freedesktop.login1", "--object-path", "/org/freedesktop/login1", "--method", "org.freedesktop.login1.Manager.PowerOff", "true"]);
    vec![systemd, logind]
}
