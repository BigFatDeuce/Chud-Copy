//! Santé des disques d'après leurs données SMART.
//!
//! Lecture seule : aucun autotest n'est lancé, rien n'est écrit sur les disques. smartctl
//! (smartmontools) est utilisé s'il est installé ; sous Windows, à défaut, les compteurs de
//! fiabilité du système. L'accès à ces données exige les droits d'administrateur : la demande
//! passe par la fenêtre du système (UAC sous Windows, polkit sous Linux).

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{fail, new_id, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Verdict {
    Healthy,
    Warning,
    Failing,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct DiskHealth {
    pub name: String,
    pub model: String,
    pub serial: String,
    /// « HDD », « SSD » ou « NVMe ».
    pub kind: String,
    pub size: u64,
    pub verdict: Option<Verdict>,
    pub temperature: Option<f64>,
    pub power_on_hours: Option<u64>,
    pub power_cycles: Option<u64>,
    pub reallocated: Option<u64>,
    pub pending: Option<u64>,
    pub uncorrectable: Option<u64>,
    pub crc_errors: Option<u64>,
    pub wear_used: Option<u64>,
    pub media_errors: Option<u64>,
    /// Délais de commande dépassés (attribut 188) : un problème de liaison, pas de surface.
    pub command_timeouts: Option<u64>,
    /// Entrées du journal d'erreurs du disque (ATA ou NVMe).
    pub error_log: Option<u64>,
    /// Erreurs du journal ATA survenues dans les 720 dernières heures de fonctionnement.
    pub errors_recent: Option<u64>,
    /// Date du relevé précédent de ce disque, s'il y en a un.
    pub previous_at: Option<i64>,
    /// Compteurs qui ont changé depuis le relevé précédent.
    pub changes: Vec<Change>,
    /// Codes des constats, traduits par l'interface.
    pub notes: Vec<String>,
    /// Relevé brut, transmis tel quel dans le compte rendu pour une IA.
    pub raw: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Change {
    pub field: String,
    pub before: u64,
    pub after: u64,
}

/// Les compteurs suivis d'un relevé à l'autre. Les erreurs de surface comptent plus que celles de liaison.
const SURFACE: [&str; 4] = ["reallocated", "pending", "uncorrectable", "mediaErrors"];
const LINK: [&str; 2] = ["crcErrors", "commandTimeouts"];
/// Au-delà, une erreur du journal est ancienne : elle date d'avant ce mois de fonctionnement.
const RECENT_HOURS: u64 = 720;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct HealthReport {
    pub tool: String,
    pub disks: Vec<DiskHealth>,
    /// smartctl absent : l'installer donne des relevés plus complets.
    pub missing_tool: bool,
    /// L'autorisation d'administrateur a été refusée : relevé partiel.
    pub limited: bool,
}

fn number(value: &Value, pointer: &str) -> Option<u64> {
    value.pointer(pointer).and_then(Value::as_u64)
}

fn ata_raw(value: &Value, id: u64) -> Option<u64> {
    value
        .pointer("/ata_smart_attributes/table")?
        .as_array()?
        .iter()
        .find(|attribute| attribute.get("id").and_then(Value::as_u64) == Some(id))
        .and_then(|attribute| attribute.pointer("/raw/value").and_then(Value::as_u64))
}

/// Interprète la sortie JSON de `smartctl -a -j`.
pub fn from_smartctl(value: &Value) -> DiskHealth {
    let nvme = value.pointer("/device/protocol").and_then(Value::as_str) == Some("NVMe");
    let rotation = number(value, "/rotation_rate");
    let kind = if nvme {
        "NVMe"
    } else if rotation == Some(0) {
        "SSD"
    } else if rotation.is_some() {
        "HDD"
    } else {
        ""
    };
    let mut disk = DiskHealth {
        name: value.pointer("/device/name").and_then(Value::as_str).unwrap_or_default().to_string(),
        model: value.get("model_name").and_then(Value::as_str).unwrap_or_default().to_string(),
        serial: value.get("serial_number").and_then(Value::as_str).unwrap_or_default().to_string(),
        kind: kind.to_string(),
        size: number(value, "/user_capacity/bytes").unwrap_or(0),
        temperature: value.pointer("/temperature/current").and_then(Value::as_f64),
        power_on_hours: number(value, "/power_on_time/hours"),
        power_cycles: number(value, "/power_cycle_count"),
        raw: Some(value.clone()),
        ..DiskHealth::default()
    };
    if nvme {
        disk.wear_used = number(value, "/nvme_smart_health_information_log/percentage_used");
        disk.media_errors = number(value, "/nvme_smart_health_information_log/media_errors");
        disk.error_log = number(value, "/nvme_smart_health_information_log/num_err_log_entries");
        if disk.error_log.unwrap_or(0) > 0 && disk.media_errors.unwrap_or(0) == 0 {
            // Le journal NVMe enregistre aussi les commandes refusées (champ invalide, commande non prise
            // en charge) envoyées par le système ou un logiciel : rien à voir avec la mémoire du disque.
            let entries = value
                .pointer("/nvme_error_information_log/table")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let hardware = entries.iter().any(|entry| {
                let status = entry.pointer("/status_field/string").and_then(Value::as_str).unwrap_or_default().to_lowercase();
                !status.is_empty()
                    && !["invalid", "not supported", "unsupported", "abort"]
                        .iter()
                        .any(|benign| status.contains(benign))
            });
            disk.notes.push(if hardware { "nvme-log".into() } else { "nvme-log-benign".into() });
        }
        if number(value, "/nvme_smart_health_information_log/critical_warning").unwrap_or(0) != 0 {
            disk.notes.push("nvme-critical".into());
        }
        let spare = number(value, "/nvme_smart_health_information_log/available_spare");
        let threshold = number(value, "/nvme_smart_health_information_log/available_spare_threshold");
        if let (Some(spare), Some(threshold)) = (spare, threshold) {
            if spare <= threshold {
                disk.notes.push("spare-low".into());
            }
        }
    } else {
        disk.reallocated = ata_raw(value, 5);
        disk.pending = ata_raw(value, 197);
        disk.uncorrectable = ata_raw(value, 198);
        disk.crc_errors = ata_raw(value, 199);
        disk.command_timeouts = ata_raw(value, 188).map(|raw| raw & 0xffff);
        disk.error_log = number(value, "/ata_smart_error_log/summary/count").or(number(value, "/ata_smart_error_log/extended/count"));
        if let (Some(count), Some(hours)) = (disk.error_log.filter(|c| *c > 0), disk.power_on_hours) {
            let table = value
                .pointer("/ata_smart_error_log/summary/table")
                .or(value.pointer("/ata_smart_error_log/extended/table"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let recent = table
                .iter()
                .filter_map(|entry| entry.get("lifetime_hours").and_then(Value::as_u64))
                .filter(|at| hours.saturating_sub(*at) <= RECENT_HOURS)
                .count() as u64;
            disk.errors_recent = Some(recent);
            if recent > 0 {
                disk.notes.push("errors-recent".into());
            }
            if count > recent {
                disk.notes.push("errors-old".into());
            }
        }
        if disk.crc_errors.unwrap_or(0) + disk.command_timeouts.unwrap_or(0) > 0 {
            disk.notes.push("link-errors".into());
        }
        // Usure des SSD SATA : selon les fabricants, durée de vie restante en attribut 231 ou 233.
        if let Some(left) = value
            .pointer("/ata_smart_attributes/table")
            .and_then(Value::as_array)
            .and_then(|table| table.iter().find(|a| matches!(a.get("id").and_then(Value::as_u64), Some(231 | 233 | 177))))
            .and_then(|a| a.get("value").and_then(Value::as_u64))
        {
            disk.wear_used = Some(100u64.saturating_sub(left.min(100)));
        }
    }
    let passed = value.pointer("/smart_status/passed").and_then(Value::as_bool);
    if passed == Some(false) {
        disk.notes.push("smart-failed".into());
    }
    disk.verdict = Some(judge(&disk, passed));
    disk
}

/// Interprète une entrée de `Get-PhysicalDisk` complétée par `Get-StorageReliabilityCounter`.
pub fn from_windows(value: &Value) -> DiskHealth {
    let text = |key: &str| {
        value
            .get(key)
            .map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()))
            .unwrap_or_default()
    };
    let media = text("MediaType");
    let bus = text("BusType");
    let kind = if bus.eq_ignore_ascii_case("NVMe") || bus == "17" {
        "NVMe"
    } else if media.eq_ignore_ascii_case("SSD") || media == "4" {
        "SSD"
    } else if media.eq_ignore_ascii_case("HDD") || media == "3" {
        "HDD"
    } else {
        ""
    };
    let mut disk = DiskHealth {
        name: text("DeviceId"),
        model: text("FriendlyName").trim().to_string(),
        serial: text("SerialNumber").trim().to_string(),
        kind: kind.to_string(),
        size: value.get("Size").and_then(Value::as_u64).unwrap_or(0),
        temperature: value.get("Temperature").and_then(Value::as_f64).filter(|t| *t > 0.0),
        power_on_hours: value.get("PowerOnHours").and_then(Value::as_u64),
        power_cycles: value.get("StartStopCycleCount").and_then(Value::as_u64),
        wear_used: value.get("Wear").and_then(Value::as_u64),
        uncorrectable: match (
            value.get("ReadErrorsUncorrected").and_then(Value::as_u64),
            value.get("WriteErrorsUncorrected").and_then(Value::as_u64),
        ) {
            (None, None) => None,
            (r, w) => Some(r.unwrap_or(0) + w.unwrap_or(0)),
        },
        raw: Some(value.clone()),
        ..DiskHealth::default()
    };
    let health = text("HealthStatus");
    let failing = health.eq_ignore_ascii_case("Unhealthy") || health == "2";
    let warning = health.eq_ignore_ascii_case("Warning") || health == "1";
    if failing {
        disk.notes.push("smart-failed".into());
    } else if warning {
        disk.notes.push("windows-warning".into());
    }
    let passed = if failing {
        Some(false)
    } else if health.is_empty() {
        None
    } else {
        Some(true)
    };
    disk.verdict = Some(if warning {
        Verdict::Warning.max_with(judge(&disk, passed))
    } else {
        judge(&disk, passed)
    });
    disk
}

impl Verdict {
    fn rank(self) -> u8 {
        match self {
            Verdict::Healthy => 0,
            Verdict::Unknown => 1,
            Verdict::Warning => 2,
            Verdict::Failing => 3,
        }
    }

    fn max_with(self, other: Verdict) -> Verdict {
        if self.rank() >= other.rank() {
            self
        } else {
            other
        }
    }
}

/// Seuils prudents : un seul secteur réalloué ou en attente mérite déjà une sauvegarde.
fn judge(disk: &DiskHealth, passed: Option<bool>) -> Verdict {
    if passed == Some(false) || disk.notes.iter().any(|n| n == "nvme-critical") {
        return Verdict::Failing;
    }
    let pending = disk.pending.unwrap_or(0) + disk.uncorrectable.unwrap_or(0);
    if pending > 0 || disk.reallocated.unwrap_or(0) > 0 || disk.media_errors.unwrap_or(0) > 0 || disk.notes.iter().any(|n| n == "spare-low") {
        return Verdict::Warning;
    }
    if disk.wear_used.unwrap_or(0) >= 90 || disk.temperature.unwrap_or(0.0) >= 60.0 || disk.errors_recent.unwrap_or(0) > 0 {
        return Verdict::Warning;
    }
    if disk.notes.iter().any(|n| n == "nvme-log") {
        return Verdict::Warning;
    }
    if passed.is_none() && disk.reallocated.is_none() && disk.wear_used.is_none() {
        return Verdict::Unknown;
    }
    Verdict::Healthy
}

fn counter(disk: &DiskHealth, field: &str) -> Option<u64> {
    match field {
        "reallocated" => disk.reallocated,
        "pending" => disk.pending,
        "uncorrectable" => disk.uncorrectable,
        "mediaErrors" => disk.media_errors,
        "crcErrors" => disk.crc_errors,
        "commandTimeouts" => disk.command_timeouts,
        "errorLog" => disk.error_log,
        _ => None,
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
struct Snapshot {
    at: i64,
    counters: std::collections::BTreeMap<String, u64>,
}

fn snapshot_key(disk: &DiskHealth) -> String {
    let id = if disk.serial.is_empty() {
        format!("{}-{}", disk.model, disk.name)
    } else {
        disk.serial.clone()
    };
    id.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' }).collect()
}

/// Compare le relevé au précédent, l'enregistre, et signale ce qui augmente.
/// Une valeur ancienne et stable est un fait passé ; une valeur qui monte est un problème en cours.
pub fn apply_history(history: &Path, disk: &mut DiskHealth, now: i64) {
    let path = history.join(format!("{}.json", snapshot_key(disk)));
    let previous: Option<Snapshot> = std::fs::read(&path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok());
    let fields = SURFACE.iter().chain(LINK.iter()).chain(std::iter::once(&"errorLog"));
    let current: std::collections::BTreeMap<String, u64> = fields.filter_map(|f| counter(disk, f).map(|v| (f.to_string(), v))).collect();
    if let Some(previous) = previous {
        disk.previous_at = Some(previous.at);
        for (field, after) in &current {
            if let Some(before) = previous.counters.get(field).filter(|before| *before != after) {
                disk.changes.push(Change {
                    field: field.clone(),
                    before: *before,
                    after: *after,
                });
            }
        }
        let rising = |group: &[&str]| disk.changes.iter().any(|c| c.after > c.before && group.contains(&c.field.as_str()));
        let surface = rising(&SURFACE);
        let link = rising(&LINK);
        if surface {
            disk.notes.push("rising-surface".into());
        }
        if link {
            disk.notes.push("rising-link".into());
        }
        if (surface || link) && disk.verdict != Some(Verdict::Failing) {
            disk.verdict = Some(Verdict::Warning);
        }
    }
    let _ = std::fs::create_dir_all(history);
    let snapshot = Snapshot { at: now, counters: current };
    if let Ok(bytes) = serde_json::to_vec_pretty(&snapshot) {
        let _ = crate::store::atomic_write(&path, &bytes);
    }
}

fn smartctl() -> Option<PathBuf> {
    let name = if cfg!(windows) { "smartctl.exe" } else { "smartctl" };
    let mut candidates: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).map(|d| d.join(name)).collect())
        .unwrap_or_default();
    if cfg!(windows) {
        candidates.push(PathBuf::from(r"C:\Program Files\smartmontools\bin\smartctl.exe"));
    } else {
        candidates.extend(["/usr/sbin/smartctl", "/usr/bin/smartctl", "/sbin/smartctl"].map(PathBuf::from));
    }
    candidates.into_iter().find(|p| p.is_file())
}

fn temp_file(work: &Path, extension: &str) -> PathBuf {
    work.join(format!("sante-{}.{extension}", new_id()))
}

/// Exécute un script avec les droits d'administrateur. Faux si l'utilisateur a refusé.
fn elevated(script: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let inner = format!("-NoProfile -ExecutionPolicy Bypass -File \"{}\"", script.display());
        let outer = format!("try {{ Start-Process powershell -Verb RunAs -Wait -WindowStyle Hidden -ArgumentList '{inner}'; exit 0 }} catch {{ exit 1 }}");
        Command::new("powershell")
            .args(["-NoProfile", "-Command", &outer])
            .creation_flags(0x0800_0000)
            .status()
            .is_ok_and(|s| s.success())
    }
    #[cfg(not(windows))]
    {
        Command::new("pkexec").arg("sh").arg(script).status().is_ok_and(|s| s.success())
    }
}

fn read_json(path: &Path) -> Option<Value> {
    let bytes = std::fs::read(path).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    serde_json::from_str(text.trim_start_matches('\u{feff}')).ok()
}

fn as_list(value: Value) -> Vec<Value> {
    match value {
        Value::Array(items) => items,
        Value::Null => Vec::new(),
        other => vec![other],
    }
}

/// Relève l'état de tous les disques. `work` est un dossier temporaire de l'application.
pub fn scan(work: &Path, history: &Path) -> Result<HealthReport> {
    std::fs::create_dir_all(work)?;
    let output = temp_file(work, "json");
    let tool = smartctl();
    let mut report = HealthReport {
        missing_tool: tool.is_none(),
        ..HealthReport::default()
    };

    if let Some(tool) = &tool {
        report.tool = "smartctl".into();
        let script = temp_file(work, if cfg!(windows) { "ps1" } else { "sh" });
        let body = if cfg!(windows) {
            format!(
                "$t = '{tool}'\n$all = @()\nforeach ($d in (& $t --scan -j | ConvertFrom-Json).devices) {{ $all += (& $t -a -j $d.name | Out-String) }}\n'[' + ($all -join ',') + ']' | Set-Content -Encoding UTF8 -LiteralPath '{out}'\n",
                tool = tool.display(),
                out = output.display()
            )
        } else {
            format!(
                "t='{tool}'\nfirst=1\n{{ printf '['\nfor d in $(\"$t\" --scan | cut -d' ' -f1); do [ $first -eq 1 ] || printf ','; first=0; \"$t\" -a -j \"$d\"; done\nprintf ']'; }} > '{out}'\nchmod 644 '{out}'\n",
                tool = tool.display(),
                out = output.display()
            )
        };
        std::fs::write(&script, body)?;
        let granted = elevated(&script);
        let _ = std::fs::remove_file(&script);
        if let Some(value) = read_json(&output).filter(|_| granted) {
            report.disks = as_list(value).iter().map(from_smartctl).collect();
        } else {
            report.limited = true;
        }
    }

    #[cfg(windows)]
    if report.disks.is_empty() {
        report.tool = "windows".into();
        let script = temp_file(work, "ps1");
        let query = |out: &Path| {
            format!(
                "Get-PhysicalDisk | ForEach-Object {{ $r = $_ | Get-StorageReliabilityCounter -ErrorAction SilentlyContinue; [pscustomobject]@{{ DeviceId = $_.DeviceId; FriendlyName = $_.FriendlyName; SerialNumber = $_.SerialNumber; MediaType = [string]$_.MediaType; BusType = [string]$_.BusType; Size = $_.Size; HealthStatus = [string]$_.HealthStatus; Temperature = $r.Temperature; Wear = $r.Wear; PowerOnHours = $r.PowerOnHours; StartStopCycleCount = $r.StartStopCycleCount; ReadErrorsUncorrected = $r.ReadErrorsUncorrected; WriteErrorsUncorrected = $r.WriteErrorsUncorrected }} }} | ConvertTo-Json -Depth 3 | Set-Content -Encoding UTF8 -LiteralPath '{}'\n",
                out.display()
            )
        };
        std::fs::write(&script, query(&output))?;
        let granted = report.limited || tool.is_some() || elevated(&script);
        if !granted || read_json(&output).is_none() {
            // Sans les droits d'administrateur, Windows donne au moins l'état général de chaque disque.
            report.limited = true;
            use std::os::windows::process::CommandExt;
            let _ = Command::new("powershell")
                .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"])
                .arg(&script)
                .creation_flags(0x0800_0000)
                .status();
        }
        let _ = std::fs::remove_file(&script);
        if let Some(value) = read_json(&output) {
            report.disks = as_list(value).iter().map(from_windows).collect();
        }
    }

    #[cfg(not(windows))]
    if report.disks.is_empty() {
        report.tool = "lsblk".into();
        let listed = Command::new("lsblk")
            .args(["-J", "-d", "-b", "-o", "NAME,MODEL,SERIAL,SIZE,ROTA,TRAN,TYPE"])
            .output();
        if let Ok(listed) = listed {
            if let Ok(value) = serde_json::from_slice::<Value>(&listed.stdout) {
                for device in value.get("blockdevices").and_then(Value::as_array).cloned().unwrap_or_default() {
                    if device.get("type").and_then(Value::as_str) != Some("disk") {
                        continue;
                    }
                    let text = |key: &str| device.get(key).and_then(Value::as_str).unwrap_or_default().trim().to_string();
                    let kind = if text("tran") == "nvme" {
                        "NVMe"
                    } else if device.get("rota").and_then(Value::as_bool) == Some(false) {
                        "SSD"
                    } else {
                        "HDD"
                    };
                    report.disks.push(DiskHealth {
                        name: format!("/dev/{}", text("name")),
                        model: text("model"),
                        serial: text("serial"),
                        kind: kind.into(),
                        size: device.get("size").and_then(Value::as_u64).unwrap_or(0),
                        verdict: Some(Verdict::Unknown),
                        raw: Some(device.clone()),
                        ..DiskHealth::default()
                    });
                }
            }
        }
    }

    let _ = std::fs::remove_file(&output);
    if report.disks.is_empty() {
        return fail(crate::ui_text("err.health-none", &[]));
    }
    let now = crate::now_ms();
    for disk in &mut report.disks {
        apply_history(history, disk, now);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ata(reallocated: u64, pending: u64, passed: bool) -> Value {
        json!({
            "device": { "name": "/dev/sda", "protocol": "ATA" },
            "model_name": "WDC WD40EFRX", "serial_number": "WD-123", "rotation_rate": 5400,
            "user_capacity": { "bytes": 4000787030016u64 },
            "smart_status": { "passed": passed },
            "temperature": { "current": 34 },
            "power_on_time": { "hours": 41234 }, "power_cycle_count": 812,
            "ata_smart_attributes": { "table": [
                { "id": 5, "value": 200, "raw": { "value": reallocated } },
                { "id": 197, "value": 200, "raw": { "value": pending } },
                { "id": 198, "value": 200, "raw": { "value": 0 } },
                { "id": 199, "value": 200, "raw": { "value": 0 } }
            ] }
        })
    }

    #[test]
    fn a_clean_hard_drive_is_healthy() {
        let disk = from_smartctl(&ata(0, 0, true));
        assert_eq!(disk.kind, "HDD");
        assert_eq!(disk.verdict, Some(Verdict::Healthy));
        assert_eq!(disk.power_on_hours, Some(41234));
        assert_eq!(disk.temperature, Some(34.0));
    }

    #[test]
    fn a_single_bad_sector_already_warns() {
        assert_eq!(from_smartctl(&ata(8, 0, true)).verdict, Some(Verdict::Warning));
        assert_eq!(from_smartctl(&ata(0, 2, true)).verdict, Some(Verdict::Warning));
        let failing = from_smartctl(&ata(0, 0, false));
        assert_eq!(failing.verdict, Some(Verdict::Failing));
        assert!(failing.notes.contains(&"smart-failed".to_string()));
    }

    #[test]
    fn nvme_wear_and_critical_warnings_are_read() {
        let base = |used: u64, critical: u64| {
            json!({
                "device": { "name": "/dev/nvme0", "protocol": "NVMe" },
                "model_name": "Samsung SSD 980", "smart_status": { "passed": true },
                "nvme_smart_health_information_log": {
                    "percentage_used": used, "media_errors": 0, "critical_warning": critical,
                    "available_spare": 100, "available_spare_threshold": 10
                }
            })
        };
        let fresh = from_smartctl(&base(3, 0));
        assert_eq!((fresh.kind.as_str(), fresh.wear_used, fresh.verdict), ("NVMe", Some(3), Some(Verdict::Healthy)));
        assert_eq!(from_smartctl(&base(95, 0)).verdict, Some(Verdict::Warning));
        assert_eq!(from_smartctl(&base(10, 4)).verdict, Some(Verdict::Failing));
    }

    #[test]
    fn cable_errors_alone_never_condemn_a_disk() {
        let mut value = ata(0, 0, true);
        value["ata_smart_attributes"]["table"][3]["raw"]["value"] = json!(57);
        let disk = from_smartctl(&value);
        assert_eq!(disk.crc_errors, Some(57));
        assert_eq!(disk.verdict, Some(Verdict::Healthy), "une erreur de liaison n'est pas une erreur de surface");
        assert!(disk.notes.contains(&"link-errors".to_string()));
    }

    #[test]
    fn old_and_recent_error_log_entries_are_told_apart() {
        let mut value = ata(0, 0, true);
        value["ata_smart_error_log"] = json!({ "summary": { "count": 3, "table": [
            { "lifetime_hours": 41200 }, { "lifetime_hours": 12000 }, { "lifetime_hours": 11990 }
        ] } });
        let disk = from_smartctl(&value);
        assert_eq!(disk.errors_recent, Some(1));
        assert!(disk.notes.contains(&"errors-recent".to_string()) && disk.notes.contains(&"errors-old".to_string()));
        assert_eq!(disk.verdict, Some(Verdict::Warning));

        value["ata_smart_error_log"]["summary"]["table"] = json!([{ "lifetime_hours": 12000 }]);
        value["ata_smart_error_log"]["summary"]["count"] = json!(1);
        let old = from_smartctl(&value);
        assert_eq!(old.verdict, Some(Verdict::Healthy), "une erreur ancienne et isolée reste de l'histoire");
        assert!(old.notes.contains(&"errors-old".to_string()));
    }

    #[test]
    fn a_refused_nvme_command_is_not_a_memory_fault() {
        let value = json!({
            "device": { "name": "/dev/nvme0", "protocol": "NVMe" }, "model_name": "WD SN770", "smart_status": { "passed": true },
            "nvme_smart_health_information_log": { "percentage_used": 2, "media_errors": 0, "critical_warning": 0, "num_err_log_entries": 1460 },
            "nvme_error_information_log": { "table": [ { "error_count": 1460, "status_field": { "string": "Invalid Field in Command" } } ] }
        });
        let disk = from_smartctl(&value);
        assert_eq!(disk.error_log, Some(1460));
        assert_eq!(disk.verdict, Some(Verdict::Healthy));
        assert!(disk.notes.contains(&"nvme-log-benign".to_string()));
    }

    #[test]
    fn a_counter_that_rises_between_two_readings_is_flagged() {
        let dir = tempfile::tempdir().unwrap();
        let mut first = from_smartctl(&ata(8, 0, true));
        apply_history(dir.path(), &mut first, 1000);
        assert!(first.changes.is_empty() && first.previous_at.is_none());

        let mut same = from_smartctl(&ata(8, 0, true));
        apply_history(dir.path(), &mut same, 2000);
        assert_eq!(same.previous_at, Some(1000));
        assert!(same.changes.is_empty(), "un compteur stable est de l'histoire ancienne");
        assert!(!same.notes.contains(&"rising-surface".to_string()));

        let mut worse = from_smartctl(&ata(12, 3, true));
        apply_history(dir.path(), &mut worse, 3000);
        assert!(worse.notes.contains(&"rising-surface".to_string()));
        assert_eq!(
            worse.changes.iter().find(|c| c.field == "reallocated").map(|c| (c.before, c.after)),
            Some((8, 12))
        );
        assert_eq!(worse.verdict, Some(Verdict::Warning));
    }

    #[test]
    fn windows_reliability_counters_are_read() {
        let healthy = json!({ "DeviceId": "0", "FriendlyName": "CT1000MX500SSD1", "MediaType": "SSD", "BusType": "SATA",
            "Size": 1000204886016u64, "HealthStatus": "Healthy", "Temperature": 38, "Wear": 7, "PowerOnHours": 9000 });
        let disk = from_windows(&healthy);
        assert_eq!((disk.kind.as_str(), disk.wear_used, disk.verdict), ("SSD", Some(7), Some(Verdict::Healthy)));

        let warning = json!({ "FriendlyName": "ST2000DM008", "MediaType": "HDD", "HealthStatus": "Warning" });
        assert_eq!(from_windows(&warning).verdict, Some(Verdict::Warning));
        let failing = json!({ "FriendlyName": "ST2000DM008", "MediaType": "HDD", "HealthStatus": "Unhealthy" });
        assert_eq!(from_windows(&failing).verdict, Some(Verdict::Failing));
        let unreadable = json!({ "FriendlyName": "Clé USB", "BusType": "USB" });
        assert_eq!(from_windows(&unreadable).verdict, Some(Verdict::Unknown));
    }
}
