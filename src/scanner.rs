use crate::drive::Drive;
use std::process::Command;

pub fn scan_drives() -> Vec<Drive> {
    #[cfg(target_os = "macos")]
    { return scan_macos(); }
    #[cfg(target_os = "linux")]
    { return scan_linux(); }
    #[cfg(target_os = "windows")]
    { return scan_windows(); }
    Vec::new()
}

// ── macOS ─────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
fn scan_macos() -> Vec<Drive> {
    let output = match Command::new("diskutil").args(["list", "-plist", "external"]).output() {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    let dict: plist::Dictionary = match plist::from_bytes(&output.stdout) {
        Ok(d) => d,
        _ => return Vec::new(),
    };
    let ids: Vec<&str> = dict.get("AllDisks").and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(|v| v.as_string()).collect()).unwrap_or_default();
    ids.into_iter()
        // Only whole disks — partitions like disk2s1 contain a lowercase letter after digits
        .filter(|id| !id.contains(|c: char| c.is_ascii_lowercase() && c != 'd'))
        .filter_map(|id| build_macos(id))
        .collect()
}

#[cfg(target_os = "macos")]
fn build_macos(disk_id: &str) -> Option<Drive> {
    let device = format!("/dev/{}", disk_id);
    let info_output = Command::new("diskutil").args(["info", "-plist", &device]).output().ok()?;
    let info: plist::Dictionary = plist::from_bytes(&info_output.stdout).ok()?;

    let size = info.get("TotalSize").and_then(|v| v.as_unsigned_integer())
        .or_else(|| info.get("Size").and_then(|v| v.as_unsigned_integer()));
    let is_read_only = info.get("Writable").and_then(|v| v.as_boolean()).map(|w| !w).unwrap_or(true);
    let description = info.get("MediaName").and_then(|v| v.as_string())
        .or_else(|| info.get("DeviceName").and_then(|v| v.as_string())).unwrap_or(disk_id).to_string();
    let mountpoints: Vec<String> = info.get("MountPoint").and_then(|v| v.as_string()).map(|s| vec![s.to_string()]).unwrap_or_default();
    let is_internal = info.get("Internal").and_then(|v| v.as_boolean()).unwrap_or(true);
    let is_usb = info.get("Protocol").and_then(|v| v.as_string()).map(|p| p == "USB").unwrap_or(false);
    Some(Drive {
        device,
        device_path: Some(format!("/dev/r{}", disk_id)),
        display_name: format!("{} ({})", &description, disk_id),
        description,
        size,
        is_system: is_internal,
        is_read_only,
        mountpoints,
        is_usb,
    })
}

// ── Linux ─────────────────────────────────────────────────────────

#[cfg(target_os = "linux")]
fn scan_linux() -> Vec<Drive> {
    let output = match Command::new("lsblk").args(["-J", "-o", "NAME,SIZE,TYPE,MOUNTPOINT,LABEL,RO,RM"]).output() {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    let root: serde_json::Value = match serde_json::from_slice(&output.stdout) { Ok(v) => v, _ => return Vec::new() };
    let devices = match root.get("blockdevices").and_then(|v| v.as_array()) { Some(arr) => arr, None => return Vec::new() };

    let mut drives = Vec::new();
    for dev in devices {
        if dev.get("rm").and_then(|v| v.as_str()) != Some("1") { continue; }
        let name = dev["name"].as_str().unwrap_or("");
        if name.is_empty() { continue; }
        let mountpoint = dev["mountpoint"].as_str().unwrap_or("");
        let label = dev["label"].as_str().unwrap_or("");
        let is_ro = dev["ro"].as_str().unwrap_or("0") == "1";
        let description = if label.is_empty() { format!("Removable Device ({})", name) } else { label.to_string() };
        drives.push(Drive {
            device: format!("/dev/{}", name),
            device_path: Some(format!("/dev/{}", name)),
            display_name: format!("{} ({})", if label.is_empty() { name } else { &description }, name),
            description,
            size: Some(parse_lsblk_size(dev["size"].as_str().unwrap_or("0"))),
            is_system: false,
            is_read_only: is_ro,
            mountpoints: if mountpoint.is_empty() { vec![] } else { vec![mountpoint.to_string()] },
            is_usb: true,
        });
    }
    drives
}

#[cfg(target_os = "linux")]
fn parse_lsblk_size(s: &str) -> u64 {
    let s = s.trim();
    if s.is_empty() { return 0; }
    let (num, unit) = if s.ends_with('G') { (s[..s.len()-1].parse::<f64>().unwrap_or(0.0), 1_000_000_000.0) }
    else if s.ends_with('M') { (s[..s.len()-1].parse::<f64>().unwrap_or(0.0), 1_000_000.0) }
    else if s.ends_with('T') { (s[..s.len()-1].parse::<f64>().unwrap_or(0.0), 1_000_000_000_000.0) }
    else if s.ends_with('K') { (s[..s.len()-1].parse::<f64>().unwrap_or(0.0), 1_000.0) }
    else { (s.parse::<f64>().unwrap_or(0.0), 1.0) };
    (num * unit) as u64
}

// ── Windows ───────────────────────────────────────────────────────

#[cfg(target_os = "windows")]
fn scan_windows() -> Vec<Drive> {
    // Use wmic to get logical disk info
    let output = match Command::new("wmic")
        .args(["logicaldisk", "where", "DriveType=2", "get", "DeviceID,VolumeName,Size,Description", "/format:csv"])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return Vec::new(),
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let mut drives = Vec::new();
    for line in text.lines().skip(1) {
        let cols: Vec<&str> = line.split(',').collect();
        if cols.len() < 4 { continue; }
        let device = cols.get(1).unwrap_or(&"").trim().to_string();
        if device.is_empty() { continue; }
        let label = cols.get(3).unwrap_or(&"").trim().to_string();
        let size_str = cols.get(2).unwrap_or(&"0").trim();
        let size = size_str.parse::<u64>().ok();

        drives.push(Drive {
            device: format!(r"\\.\{}:", device),
            device_path: Some(device.clone()),
            description: if label.is_empty() { format!("Removable Disk ({})", device) } else { label.clone() },
            display_name: format!("{} ({})", if label.is_empty() { &device } else { &label }, device),
            size,
            is_system: false,
            is_read_only: false,
            mountpoints: vec![format!("{}:", device)],
            is_usb: true,
        });
    }

    // Also try wmic diskdrive for physical device info
    let phys_output = match Command::new("wmic")
        .args(["diskdrive", "where", "MediaType='Removable media'", "get", "DeviceID,Model,Size,InterfaceType", "/format:csv"])
        .output()
    {
        Ok(o) if o.status.success() => o,
        _ => return drives,
    };
    let text = String::from_utf8_lossy(&phys_output.stdout);
    for line in text.lines().skip(1) {
        let cols: Vec<&str> = line.split(',').collect();
        if cols.len() < 4 { continue; }
        let device_id = cols.get(1).unwrap_or(&"").trim();
        if device_id.is_empty() { continue; }
        let model = cols.get(3).unwrap_or(&"").trim().to_string();
        // deduplicate by checking if device_id is already in drives
        if drives.iter().any(|d| d.device == device_id) { continue; }
        let size_str = cols.get(2).unwrap_or(&"0").trim();
        let size = size_str.parse::<u64>().ok();

        drives.push(Drive {
            device: device_id.to_string(),
            device_path: Some(device_id.to_string()),
            description: if model.is_empty() { "Removable Disk".into() } else { model },
            display_name: format!("{} ({})", model, device_id.trim_start_matches(r"\\.\")),
            size,
            is_system: false,
            is_read_only: false,
            mountpoints: Vec::new(),
            is_usb: true,
        });
    }
    drives
}
