use std::fmt;

/// Physical block device info.
#[derive(Clone, Debug)]
pub struct Drive {
    pub device: String,
    pub device_path: Option<String>,
    pub description: String,
    pub display_name: String,
    pub size: Option<u64>,
    pub is_system: bool,
    pub is_read_only: bool,
    pub mountpoints: Vec<String>,
    pub is_usb: bool,
}

impl Drive {
    pub fn is_valid(&self, image: Option<&ImageInfo>, write: bool) -> bool {
        if !write {
            return true;
        }
        if self.is_read_only {
            return false;
        }
        if let Some(img) = image {
            if !self.is_large_enough(img) {
                return false;
            }
            if self.is_source_drive(img) {
                return false;
            }
        }
        !self.disabled()
    }

    pub fn is_large_enough(&self, image: &ImageInfo) -> bool {
        let drive_size = self.size.unwrap_or(0);
        if image.is_size_estimated {
            let compressed = image.compressed_size.unwrap_or(image.size);
            if drive_size < compressed {
                return false;
            }
            return true;
        }
        drive_size >= image.size
    }

    pub fn is_source_drive(&self, image: &ImageInfo) -> bool {
        if let Some(ref d) = image.drive {
            return d.device == self.device;
        }
        if let Some(ref path) = image.path {
            return self.mountpoints.iter().any(|mp| path.starts_with(mp));
        }
        false
    }

    pub fn is_system_drive(&self) -> bool {
        self.is_system
    }

    pub fn is_size_recommended(&self, image: &ImageInfo) -> bool {
        let drive_size = self.size.unwrap_or(0);
        drive_size >= image.recommended_drive_size.unwrap_or(0)
    }

    pub fn disabled(&self) -> bool {
        self.is_read_only
    }

    pub fn is_large(&self) -> bool {
        self.size.map_or(false, |s| s > LARGE_DRIVE_SIZE)
    }

    pub fn compatibility_statuses(
        &self,
        image: Option<&ImageInfo>,
        write: bool,
    ) -> Vec<DriveStatus> {
        let mut list = Vec::new();
        if self.is_read_only && write {
            list.push(DriveStatus {
                type_: CompatibilityType::Error,
                message: "Locked".into(),
            });
        }
        if let Some(img) = image {
            if !self.is_large_enough(img) {
                list.push(DriveStatus {
                    type_: CompatibilityType::Error,
                    message: "Too small".into(),
                });
                return list;
            }
        }
        if self.is_system_drive() {
            list.push(DriveStatus {
                type_: CompatibilityType::Warning,
                message: "System drive".into(),
            });
        } else if self.is_large() {
            list.push(DriveStatus {
                type_: CompatibilityType::Warning,
                message: "Large drive".into(),
            });
        }
        if let Some(img) = image {
            if self.is_source_drive(img) {
                list.push(DriveStatus {
                    type_: CompatibilityType::Error,
                    message: "Source drive".into(),
                });
            }
            if !self.is_size_recommended(img) {
                list.push(DriveStatus {
                    type_: CompatibilityType::Warning,
                    message: "Not recommended".into(),
                });
            }
        }
        list
    }
}

pub const LARGE_DRIVE_SIZE: u64 = 128_000_000_000;

#[derive(Clone, Debug, Default)]
pub struct ImageInfo {
    pub path: Option<String>,
    pub url: Option<String>,
    pub name: Option<String>,
    pub size: u64,
    pub compressed_size: Option<u64>,
    pub recommended_drive_size: Option<u64>,
    pub is_size_estimated: bool,
    pub drive: Option<Drive>,
    pub extension: Option<String>,
    pub logo: Option<String>,
    pub source_type: SourceType,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum SourceType {
    #[default]
    File,
    BlockDevice,
    Http,
}

impl fmt::Display for SourceType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceType::File => write!(f, "File"),
            SourceType::BlockDevice => write!(f, "BlockDevice"),
            SourceType::Http => write!(f, "Http"),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum CompatibilityType {
    Warning,
    Error,
}

#[derive(Clone, Debug)]
pub struct DriveStatus {
    pub type_: CompatibilityType,
    pub message: String,
}

pub const SUPPORTED_EXTENSIONS: &[&str] = &[
    "bin", "bz2", "dmg", "dsk", "etch", "gz", "hddimg", "img", "iso", "raw",
    "rpi-sdimg", "sdcard", "vhd", "wic", "xz", "zip",
];

pub fn looks_like_windows_image(image_path: &str) -> bool {
    let re = regex::Regex::new(r"(?i)windows|win7|win8|win10|winxp").unwrap();
    if let Some(name) = std::path::Path::new(image_path).file_name() {
        re.is_match(&name.to_string_lossy())
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_drive() -> Drive {
        Drive {
            device: "/dev/disk2".into(),
            device_path: Some("/dev/rdisk2".into()),
            description: "USB Drive".into(),
            display_name: "USB Drive (disk2)".into(),
            size: Some(32_000_000_000), // 32GB
            is_system: false,
            is_read_only: false,
            mountpoints: vec!["/Volumes/USB".into()],
            is_usb: true,
        }
    }

    fn test_image(size: u64) -> ImageInfo {
        ImageInfo {
            path: Some("/path/to/image.img".into()),
            name: Some("image.img".into()),
            size,
            source_type: SourceType::File,
            ..Default::default()
        }
    }

    #[test]
    fn test_is_valid_writable() {
        let d = test_drive();
        assert!(d.is_valid(None, true));
    }

    #[test]
    fn test_is_valid_read_only() {
        let mut d = test_drive();
        d.is_read_only = true;
        assert!(!d.is_valid(None, true));
        assert!(d.is_valid(None, false)); // write=false skips RO check
    }

    #[test]
    fn test_is_valid_system_drive() {
        let mut d = test_drive();
        d.is_system = true;
        // System drives are valid (just warned about)
        assert!(d.is_valid(None, true));
    }

    #[test]
    fn test_is_large_enough() {
        let d = test_drive();
        let img = test_image(16_000_000_000); // 16GB
        assert!(d.is_large_enough(&img));

        let img_big = test_image(64_000_000_000); // 64GB > 32GB drive
        assert!(!d.is_large_enough(&img_big));
    }

    #[test]
    fn test_is_large_enough_estimated() {
        let d = test_drive();
        let mut img = test_image(64_000_000_000);
        img.is_size_estimated = true;
        img.compressed_size = Some(10_000_000_000); // compressed is smaller than drive
        assert!(d.is_large_enough(&img));

        let mut img2 = test_image(64_000_000_000);
        img2.is_size_estimated = true;
        img2.compressed_size = Some(40_000_000_000); // compressed larger than drive
        assert!(!d.is_large_enough(&img2)); // estimated but compressed too big
    }

    #[test]
    fn test_is_source_drive_by_mount() {
        let d = test_drive();
        let img = ImageInfo {
            path: Some("/Volumes/USB/image.img".into()),
            ..Default::default()
        };
        assert!(d.is_source_drive(&img));
    }

    #[test]
    fn test_is_source_drive_by_device() {
        let d = test_drive();
        let img = ImageInfo {
            drive: Some(d.clone()),
            ..Default::default()
        };
        assert!(d.is_source_drive(&img));
    }

    #[test]
    fn test_is_source_drive_no_match() {
        let d = test_drive();
        let img = ImageInfo {
            path: Some("/some/other/path.img".into()),
            ..Default::default()
        };
        assert!(!d.is_source_drive(&img));
    }

    #[test]
    fn test_is_large_128gb() {
        let mut d = test_drive();
        d.size = Some(129_000_000_000); // 129GB > 128GB
        assert!(d.is_large());

        d.size = Some(64_000_000_000); // 64GB
        assert!(!d.is_large());
    }

    #[test]
    fn test_is_size_recommended() {
        let d = test_drive();
        let mut img = test_image(4_000_000_000);
        img.recommended_drive_size = Some(16_000_000_000); // recommends 16GB
        assert!(d.is_size_recommended(&img)); // 32GB > 16GB

        img.recommended_drive_size = Some(64_000_000_000);
        assert!(!d.is_size_recommended(&img)); // 32GB < 64GB
    }

    #[test]
    fn test_compatibility_statuses_all_clear() {
        let d = test_drive();
        let img = test_image(4_000_000_000);
        let statuses = d.compatibility_statuses(Some(&img), true);
        assert!(statuses.is_empty());
    }

    #[test]
    fn test_compatibility_statuses_read_only() {
        let mut d = test_drive();
        d.is_read_only = true;
        let statuses = d.compatibility_statuses(None, true);
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].type_, CompatibilityType::Error);
        assert_eq!(statuses[0].message, "Locked");
    }

    #[test]
    fn test_compatibility_statuses_too_small() {
        let d = test_drive();
        let img = test_image(64_000_000_000);
        let statuses = d.compatibility_statuses(Some(&img), true);
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].type_, CompatibilityType::Error);
        assert_eq!(statuses[0].message, "Too small");
    }

    #[test]
    fn test_compatibility_statuses_system() {
        let mut d = test_drive();
        d.is_system = true;
        let statuses = d.compatibility_statuses(None, true);
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].type_, CompatibilityType::Warning);
        assert_eq!(statuses[0].message, "System drive");
    }

    #[test]
    fn test_compatibility_statuses_large() {
        let mut d = test_drive();
        d.size = Some(129_000_000_000);
        let statuses = d.compatibility_statuses(None, true);
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].type_, CompatibilityType::Warning);
        assert_eq!(statuses[0].message, "Large drive");
    }

    #[test]
    fn test_looks_like_windows_image() {
        assert!(looks_like_windows_image("/path/to/Win10_22H2.iso"));
        assert!(!looks_like_windows_image("/path/to/ubuntu-24.04.img"));
    }

    #[test]
    fn test_supported_extensions() {
        assert!(SUPPORTED_EXTENSIONS.contains(&"img"));
        assert!(SUPPORTED_EXTENSIONS.contains(&"iso"));
        assert!(SUPPORTED_EXTENSIONS.contains(&"gz"));
        assert!(SUPPORTED_EXTENSIONS.contains(&"zip"));
    }
}
