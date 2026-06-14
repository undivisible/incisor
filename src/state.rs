use crate::drive::{Drive, ImageInfo};
use std::collections::HashSet;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq)]
pub enum FlashStep {
    Starting,
    Decompressing,
    Flashing,
    Verifying,
    Finishing,
    Failed,
}

#[derive(Clone, Debug)]
pub struct FlashProgress {
    pub step: FlashStep,
    pub percentage: Option<f64>,
    pub speed: Option<f64>,
    pub eta: Option<f64>,
    pub active: u32,
    pub failed: u32,
    pub position: u64,
}

impl Default for FlashProgress {
    fn default() -> Self {
        Self {
            step: FlashStep::Starting,
            percentage: None,
            speed: None,
            eta: None,
            active: 0,
            failed: 0,
            position: 0,
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct FlashResults {
    pub cancelled: bool,
    pub source_checksum: Option<String>,
    pub error_code: Option<String>,
    pub bytes_written: u64,
    pub successful: u32,
    pub failed: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Page {
    Main,
    Success,
}

#[derive(Clone, Debug)]
pub struct AppModel {
    pub available_drives: Vec<Drive>,
    pub drives_loaded: bool,
    pub selected_devices: HashSet<String>,
    pub image: Option<ImageInfo>,
    pub is_flashing: bool,
    pub flash_progress: FlashProgress,
    pub flash_results: Option<FlashResults>,
    pub page: Page,
    pub settings_open: bool,
    pub error_message: Option<String>,
    pub warning_message: Option<String>,
    pub drives_with_warnings: Vec<(Drive, Vec<crate::drive::DriveStatus>)>,
    pub application_session_uuid: String,
    pub flash_uuid: Option<String>,
    pub url_input_open: bool,
    pub url_text: String,
}

impl AppModel {
    pub fn new() -> Self {
        Self {
            available_drives: Vec::new(),
            drives_loaded: false,
            selected_devices: HashSet::new(),
            image: None,
            is_flashing: false,
            flash_progress: FlashProgress::default(),
            flash_results: None,
            page: Page::Main,
            settings_open: false,
            error_message: None,
            warning_message: None,
            drives_with_warnings: Vec::new(),
            application_session_uuid: Uuid::new_v4().to_string(),
            flash_uuid: None,
            url_input_open: false,
            url_text: String::new(),
        }
    }

    pub fn has_image(&self) -> bool { self.image.is_some() }
    pub fn has_drive(&self) -> bool { !self.selected_devices.is_empty() }

    pub fn selected_drives(&self) -> Vec<Drive> {
        self.available_drives
            .iter()
            .filter(|d| self.selected_devices.contains(&d.device))
            .cloned()
            .collect()
    }

    pub fn select_drive(&mut self, device: &str) {
        if let Some(drive) = self.available_drives.iter().find(|d| d.device == device) {
            if drive.is_read_only {
                self.error_message = Some("The drive is write-protected".into());
                return;
            }
            if let Some(ref img) = self.image {
                if !drive.is_large_enough(img) {
                    self.error_message = Some("The drive is not large enough".into());
                    return;
                }
            }
            self.selected_devices.insert(device.to_string());
        }
    }

    pub fn deselect_drive(&mut self, device: &str) {
        self.selected_devices.remove(device);
    }

    pub fn toggle_drive(&mut self, device: &str) {
        if self.selected_devices.contains(device) { self.deselect_drive(device); }
        else { self.select_drive(device); }
    }

    pub fn set_image(&mut self, image: ImageInfo) {
        let incompatible: Vec<String> = self.selected_devices
            .iter()
            .filter_map(|dev| {
                self.available_drives.iter().find(|d| d.device == *dev).and_then(|d| {
                    if !d.is_valid(Some(&image), true) { Some(dev.clone()) } else { None }
                })
            })
            .collect();
        for dev in incompatible { self.selected_devices.remove(&dev); }
        self.image = Some(image);
    }

    pub fn clear_selection(&mut self) { self.selected_devices.clear(); self.image = None; }

    pub fn clear_image(&mut self) { self.image = None; }

    pub fn drive_title(&self) -> String {
        let drives = self.selected_drives();
        match drives.len() {
            0 => "No targets found".into(),
            1 => drives[0].description.clone(),
            n => format!("{} Targets", n),
        }
    }

    pub fn image_basename(&self) -> String {
        self.image.as_ref().and_then(|img| {
            if let Some(ref d) = img.drive { return Some(d.description.clone()); }
            img.path.as_ref().and_then(|p| std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().to_string()))
                .or_else(|| img.name.clone())
        }).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drive::{SourceType, DriveStatus, CompatibilityType};

    fn sample_drive(device: &str) -> Drive {
        Drive {
            device: device.into(),
            device_path: Some(format!("/dev/r{}", device.trim_start_matches("/dev/"))),
            description: format!("Drive {}", device),
            display_name: format!("Drive ({})", device),
            size: Some(32_000_000_000),
            is_system: false,
            is_read_only: false,
            mountpoints: vec!["/Volumes/DRIVE".into()],
            is_usb: true,
        }
    }

    #[test]
    fn test_initial_state() {
        let m = AppModel::new();
        assert!(!m.has_image());
        assert!(!m.has_drive());
        assert_eq!(m.page, Page::Main);
        assert!(!m.is_flashing);
        assert!(m.available_drives.is_empty());
    }

    #[test]
    fn test_select_drive() {
        let mut m = AppModel::new();
        let d = sample_drive("/dev/disk2");
        m.available_drives = vec![d.clone()];
        m.select_drive("/dev/disk2");
        assert!(m.has_drive());
        assert_eq!(m.selected_devices.len(), 1);
    }

    #[test]
    fn test_select_nonexistent_drive() {
        let mut m = AppModel::new();
        m.select_drive("/dev/disk99");
        assert!(!m.has_drive());
    }

    #[test]
    fn test_toggle_drive() {
        let mut m = AppModel::new();
        m.available_drives = vec![sample_drive("/dev/disk2")];
        m.toggle_drive("/dev/disk2");
        assert!(m.has_drive());
        m.toggle_drive("/dev/disk2");
        assert!(!m.has_drive());
    }

    #[test]
    fn test_select_read_only_drive() {
        let mut m = AppModel::new();
        let mut d = sample_drive("/dev/disk2");
        d.is_read_only = true;
        m.available_drives = vec![d];
        m.select_drive("/dev/disk2");
        assert!(!m.has_drive());
        assert!(m.error_message.is_some());
    }

    #[test]
    fn test_set_image_clears_incompatible_drives() {
        let mut m = AppModel::new();
        let small = Drive {
            device: "/dev/disk2".into(),
            device_path: Some("/dev/rdisk2".into()),
            description: "Small Drive".into(),
            display_name: "Small (disk2)".into(),
            size: Some(4_000_000_000), // 4GB
            is_system: false,
            is_read_only: false,
            mountpoints: vec![],
            is_usb: true,
        };
        let large = Drive {
            device: "/dev/disk3".into(),
            device_path: Some("/dev/rdisk3".into()),
            description: "Large Drive".into(),
            display_name: "Large (disk3)".into(),
            size: Some(64_000_000_000), // 64GB
            is_system: false,
            is_read_only: false,
            mountpoints: vec![],
            is_usb: true,
        };
        m.available_drives = vec![small, large];
        m.select_drive("/dev/disk2");
        m.select_drive("/dev/disk3");
        assert_eq!(m.selected_devices.len(), 2);

        // Set a large image — disk2 (4GB) should be deselected
        m.set_image(ImageInfo {
            path: Some("/big.img".into()),
            size: 16_000_000_000, // 16GB > 4GB
            ..Default::default()
        });
        assert!(m.has_image());
        assert_eq!(m.selected_devices.len(), 1);
        assert!(m.selected_devices.contains("/dev/disk3"));
    }

    #[test]
    fn test_clear_selection() {
        let mut m = AppModel::new();
        m.available_drives = vec![sample_drive("/dev/disk2")];
        m.select_drive("/dev/disk2");
        m.set_image(ImageInfo {
            path: Some("/img.img".into()),
            size: 1000,
            ..Default::default()
        });
        assert!(m.has_image());
        assert!(m.has_drive());

        m.clear_selection();
        assert!(!m.has_image());
        assert!(!m.has_drive());
    }

    #[test]
    fn test_selected_drives() {
        let mut m = AppModel::new();
        let d1 = sample_drive("/dev/disk2");
        let d2 = sample_drive("/dev/disk3");
        m.available_drives = vec![d1, d2];
        m.select_drive("/dev/disk2");
        let selected = m.selected_drives();
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].device, "/dev/disk2");
    }

    #[test]
    fn test_drive_title() {
        let mut m = AppModel::new();
        assert_eq!(m.drive_title(), "No targets found");

        let d = sample_drive("/dev/disk2");
        m.available_drives = vec![d];
        m.select_drive("/dev/disk2");
        assert_eq!(m.drive_title(), "Drive /dev/disk2");

        let d2 = sample_drive("/dev/disk3");
        m.available_drives.push(d2);
        m.select_drive("/dev/disk3");
        assert_eq!(m.drive_title(), "2 Targets");
    }

    #[test]
    fn test_image_basename_from_path() {
        let mut m = AppModel::new();
        m.set_image(ImageInfo {
            path: Some("/somewhere/ubuntu-24.04.img".into()),
            size: 4_000_000_000,
            ..Default::default()
        });
        assert_eq!(m.image_basename(), "ubuntu-24.04.img");
    }

    #[test]
    fn test_image_basename_from_name() {
        let mut m = AppModel::new();
        m.set_image(ImageInfo {
            name: Some("raspios.img".into()),
            size: 2_000_000_000,
            ..Default::default()
        });
        assert_eq!(m.image_basename(), "raspios.img");
    }

    #[test]
    fn test_image_basename_path_takes_priority() {
        let mut m = AppModel::new();
        m.set_image(ImageInfo {
            path: Some("/tmp/download".into()),
            name: Some("raspios.img".into()),
            size: 2_000_000_000,
            ..Default::default()
        });
        // path basename takes priority
        assert_eq!(m.image_basename(), "download");
    }

    #[test]
    fn test_image_basename_from_drive() {
        let mut m = AppModel::new();
        m.set_image(ImageInfo {
            drive: Some(Drive {
                device: "/dev/disk2".into(),
                description: "Clone Drive".into(),
                ..sample_drive("/dev/disk2")
            }),
            size: 16_000_000_000,
            ..Default::default()
        });
        assert_eq!(m.image_basename(), "Clone Drive");
    }

    #[test]
    fn test_flash_progress_default() {
        let p = FlashProgress::default();
        assert_eq!(p.step, FlashStep::Starting);
        assert!(p.percentage.is_none());
        assert!(p.speed.is_none());
        assert!(p.eta.is_none());
    }

    #[test]
    fn test_flash_results_default() {
        let r = FlashResults::default();
        assert!(!r.cancelled);
        assert_eq!(r.successful, 0);
        assert_eq!(r.failed, 0);
        assert!(r.source_checksum.is_none());
    }
}
