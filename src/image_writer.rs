use crate::drive::{Drive, ImageInfo};
use crate::state::{FlashProgress, FlashResults, FlashStep};
use sha2::{Sha256, Digest};
use tokio::sync::mpsc;
use std::io::{Read, Write};
use std::path::Path;

const BLOCK_SIZE: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug)]
pub enum WriterEvent {
    Progress(FlashProgress),
    Done(FlashResults),
    Fail(String),
}

#[derive(Clone, Debug)]
pub enum WriterCommand {
    Cancel,
}

pub struct FlashHandle {
    pub cmd_tx: mpsc::Sender<WriterCommand>,
}

impl FlashHandle {
    pub fn cancel(&self) { let _ = self.cmd_tx.try_send(WriterCommand::Cancel); }
}

pub fn start_flash(
    image: ImageInfo,
    drives: Vec<Drive>,
    event_tx: mpsc::Sender<WriterEvent>,
) -> FlashHandle {
    let (cmd_tx, cmd_rx) = mpsc::channel::<WriterCommand>(8);
    let handle = FlashHandle { cmd_tx: cmd_tx.clone() };

    tokio::spawn(async move {
        let mut cancelled = false;
        let mut cmd_rx = cmd_rx;

        macro_rules! send { ($evt:expr) => { let _ = event_tx.send($evt).await; }; }

        macro_rules! progress { ($p:expr) => { send!(WriterEvent::Progress($p)); }; }

        // Phase 1: Starting
        progress!(FlashProgress { step: FlashStep::Starting, ..Default::default() });

        if matches!(cmd_rx.try_recv(), Ok(WriterCommand::Cancel)) { cancelled = true; }

        // Resolve source: file path or download URL
        let source_path: String = match (&image.path, &image.url) {
            (Some(p), _) => p.clone(),
            (None, Some(url)) => {
                progress!(FlashProgress { step: FlashStep::Decompressing, percentage: Some(0.0), ..Default::default() });
                let ext = Path::new(url).extension().and_then(|e| e.to_str()).unwrap_or("img").to_string();
                match download_to_temp(url, &ext, &event_tx).await {
                    Some(p) => p,
                    None => return,
                }
            }
            (None, None) => {
                send!(WriterEvent::Fail("No source path or URL".into()));
                return;
            }
        };

        // Phase 2: Decompress (blocking I/O)
        progress!(FlashProgress { step: FlashStep::Decompressing, percentage: Some(0.0), ..Default::default() });

        let compressed = match tokio::task::spawn_blocking({
            let path = source_path.clone();
            move || std::fs::read(&path)
        }).await {
            Ok(Ok(data)) => data,
            Ok(Err(e)) => { send!(WriterEvent::Fail(format!("Cannot read {}: {}", source_path, e))); return; }
            Err(e) => { send!(WriterEvent::Fail(format!("Task error: {}", e))); return; }
        };

        let data = if cancelled { compressed } else {
            match tokio::task::spawn_blocking(move || decompress_sync(&compressed, &source_path)).await {
                Ok(Ok(d)) => d,
                Ok(Err(e)) => { send!(WriterEvent::Fail(e)); return; }
                Err(e) => { send!(WriterEvent::Fail(format!("Decompress task: {}", e))); return; }
            }
        };

        progress!(FlashProgress { step: FlashStep::Decompressing, percentage: Some(100.0), ..Default::default() });

        if matches!(cmd_rx.try_recv(), Ok(WriterCommand::Cancel)) { cancelled = true; }
        if cancelled {
            send!(WriterEvent::Done(FlashResults { cancelled: true, ..Default::default() }));
            return;
        }

        // Source checksum
        let source_hash = {
            let mut hasher = Sha256::new();
            hasher.update(&data);
            hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect::<String>()
        };

        let total_len = data.len() as u64;
        let start = std::time::Instant::now();
        let mut successful = 0u32;
        let mut failed = 0u32;

        for drive in &drives {
            if cancelled { break; }

            let dev_path = match &drive.device_path {
                Some(p) => p.clone(),
                None => { failed += 1; continue; }
            };

            progress!(FlashProgress { step: FlashStep::Flashing, percentage: Some(0.0), active: 1, ..Default::default() });

            let mut dest = match std::fs::OpenOptions::new().write(true).read(true).open(&dev_path) {
                Ok(f) => f,
                Err(e) => {
                    failed += 1;
                    let msg = match e.raw_os_error() {
                        Some(libc::EACCES) => format!(
                            "Permission denied: {}.\nWriting to block devices requires elevated privileges.\nRun with: sudo {} {}",
                            dev_path,
                            std::env::current_exe().unwrap_or_default().display(),
                            std::env::args().skip(1).collect::<Vec<_>>().join(" ")
                        ),
                        _ => format!("Cannot open {}: {}", dev_path, e),
                    };
                    send!(WriterEvent::Fail(msg));
                    continue;
                }
            };

            let mut written: u64 = 0;
            for chunk in data.chunks(BLOCK_SIZE) {
                if matches!(cmd_rx.try_recv(), Ok(WriterCommand::Cancel)) { cancelled = true; break; }

                match dest.write_all(chunk) {
                    Ok(()) => {
                        written += chunk.len() as u64;
                        let pct = (written as f64 / total_len as f64 * 100.0).min(100.0);
                        let elapsed = start.elapsed().as_secs_f64();
                        let speed = if elapsed > 0.0 { (written as f64 / 1_000_000.0) / elapsed } else { 0.0 };
                        let eta = if speed > 0.0 && pct < 100.0 { ((total_len - written) as f64 / 1_000_000.0) / speed } else { 0.0 };

                        progress!(FlashProgress {
                            step: FlashStep::Flashing, percentage: Some(pct),
                            speed: Some(speed), eta: Some(eta), active: 1,
                            ..Default::default()
                        });
                    }
                    Err(e) => {
                        failed += 1;
                        send!(WriterEvent::Fail(format!("Write error on {}: {}", dev_path, e)));
                        break;
                    }
                }
            }

            if cancelled { break; }
            let _ = dest.flush();
            drop(dest);

            // Verify
            progress!(FlashProgress { step: FlashStep::Verifying, percentage: Some(0.0), ..Default::default() });

            let ok = verify_sync(&dev_path, &data);

            if ok { successful += 1; } else { failed += 1; }
            progress!(FlashProgress { step: FlashStep::Verifying, percentage: Some(100.0), ..Default::default() });
        }

        if cancelled {
            send!(WriterEvent::Done(FlashResults { cancelled: true, ..Default::default() }));
        } else {
            send!(WriterEvent::Done(FlashResults {
                cancelled: false, successful, failed,
                source_checksum: Some(source_hash),
                ..Default::default()
            }));
        }
    });

    handle
}

/// Synchronous decompression (called from spawn_blocking)
fn decompress_sync(data: &[u8], path: &str) -> Result<Vec<u8>, String> {
    let ext = Path::new(path).extension().and_then(|e| e.to_str()).map(|s| s.to_lowercase());

    match ext.as_deref() {
        Some("gz") | Some("gzip") => {
            let mut decoder = flate2::read::GzDecoder::new(data);
            let mut out = Vec::with_capacity(data.len() * 4);
            decoder.read_to_end(&mut out).map_err(|e| format!("Gzip: {}", e))?;
            Ok(out)
        }
        Some("bz2") => {
            let mut decoder = bzip2::read::BzDecoder::new(data);
            let mut out = Vec::with_capacity(data.len() * 3);
            decoder.read_to_end(&mut out).map_err(|e| format!("Bzip2: {}", e))?;
            Ok(out)
        }
        Some("xz") => {
            let mut decoder = xz2::read::XzDecoder::new(data);
            let mut out = Vec::with_capacity(data.len() * 2);
            decoder.read_to_end(&mut out).map_err(|e| format!("XZ: {}", e))?;
            Ok(out)
        }
        Some("zip") => {
            let mut archive = zip::ZipArchive::new(std::io::Cursor::new(data))
                .map_err(|e| format!("ZIP: {}", e))?;
            if archive.len() == 0 { return Err("Empty ZIP".into()); }
            let mut file = archive.by_index(0).map_err(|e| format!("ZIP entry: {}", e))?;
            let mut out = Vec::with_capacity(file.size() as usize);
            file.read_to_end(&mut out).map_err(|e| format!("ZIP read: {}", e))?;
            Ok(out)
        }
        _ => Ok(data.to_vec()),
    }
}

/// Synchronous verify — reads from raw device on macOS to bypass block cache
fn verify_sync(path: &str, original: &[u8]) -> bool {
    #[cfg(target_os = "macos")]
    let path = path.replace("/dev/disk", "/dev/rdisk");
    let max_read = original.len().min(16_000_000);
    let mut buf = vec![0u8; max_read];
    let mut f = match std::fs::File::open(path) { Ok(f) => f, _ => return false };
    let n = match f.read(&mut buf) { Ok(n) => n, _ => return false };
    buf.truncate(n);

    let orig_hash = {
        let mut h = Sha256::new();
        h.update(&original[..buf.len()]);
        h.finalize().iter().map(|b| format!("{:02x}", b)).collect::<String>()
    };
    let read_hash = {
        let mut h = Sha256::new();
        h.update(&buf);
        h.finalize().iter().map(|b| format!("{:02x}", b)).collect::<String>()
    };
    orig_hash == read_hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decompress_sync_raw() {
        // Raw data (no compression) should pass through unchanged
        let data = b"hello world this is a test image";
        let result = decompress_sync(data, "/tmp/test.img").unwrap();
        assert_eq!(result, data);
    }

    #[test]
    fn test_decompress_sync_gzip() {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        use std::io::Write;

        let original = b"This is some image data that will be compressed";
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(original).unwrap();
        let compressed = encoder.finish().unwrap();

        let result = decompress_sync(&compressed, "/tmp/test.img.gz").unwrap();
        assert_eq!(result, original);
    }

    #[test]
    fn test_decompress_sync_bzip2() {
        use bzip2::write::BzEncoder;
        use bzip2::Compression;
        use std::io::Write;

        let original = b"Bzip2 compressed image data";
        let mut encoder = BzEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(original).unwrap();
        let compressed = encoder.finish().unwrap();

        let result = decompress_sync(&compressed, "/tmp/test.img.bz2").unwrap();
        assert_eq!(result, original);
    }

    #[test]
    fn test_verify_sync_match() {
        // Write a temp file and verify it matches
        let dir = std::env::temp_dir();
        let path = dir.join("incisor-test-verify.img");
        let data = b"verify me please!".repeat(1000);
        std::fs::write(&path, &data).unwrap();

        assert!(verify_sync(path.to_str().unwrap(), &data));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn test_verify_sync_mismatch() {
        let dir = std::env::temp_dir();
        let path = dir.join("incisor-test-verify-bad.img");
        let data = b"original data".repeat(100);
        std::fs::write(&path, &data).unwrap();

        let wrong = b"different data".repeat(100);
        assert!(!verify_sync(path.to_str().unwrap(), &wrong));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn test_verify_sync_nonexistent() {
        assert!(!verify_sync("/nonexistent/device", b"data"));
    }

    #[test]
    fn test_verify_sync_partial() {
        // Verify only reads first 16MB, so large data beyond that should still match
        let data = vec![0xABu8; 32_000_000]; // 32MB > 16MB verify limit
        let dir = std::env::temp_dir();
        let path = dir.join("incisor-test-verify-large.img");
        std::fs::write(&path, &data).unwrap();

        assert!(verify_sync(path.to_str().unwrap(), &data));
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    fn test_channel_send_recv() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let (tx, mut rx) = mpsc::channel::<WriterEvent>(8);
            tx.send(WriterEvent::Progress(FlashProgress {
                step: FlashStep::Starting,
                percentage: Some(50.0),
                ..Default::default()
            })).await.unwrap();

            let evt = rx.recv().await.unwrap();
            match evt {
                WriterEvent::Progress(p) => {
                    assert_eq!(p.step, FlashStep::Starting);
                    assert_eq!(p.percentage, Some(50.0));
                }
                _ => panic!("Expected Progress"),
            }
        });
    }

    #[test]
    fn test_cancel_command() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let (tx, mut rx) = mpsc::channel::<WriterCommand>(8);
            tx.send(WriterCommand::Cancel).await.unwrap();
            let cmd = rx.recv().await.unwrap();
            assert!(matches!(cmd, WriterCommand::Cancel));
        });
    }
}

/// Download a URL to a temp file and return the path.
async fn download_to_temp(
    url: &str,
    ext: &str,
    event_tx: &mpsc::Sender<WriterEvent>,
) -> Option<String> {
    let client = reqwest::Client::builder()
        .user_agent("Artisan/0.1")
        .build()
        .ok()?;

    let resp = client.get(url).send().await.ok()?;
    let total = resp.content_length().unwrap_or(0);

    let tmp_path = std::env::temp_dir().join(format!("incisor-dl-{}.{}", uuid::Uuid::new_v4(), ext));
    let mut file = std::fs::File::create(&tmp_path).ok()?;

    let mut downloaded: u64 = 0;
    let mut stream = resp.bytes_stream();

    use futures_util::StreamExt;
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.ok()?;
        file.write_all(&chunk).ok()?;
        downloaded += chunk.len() as u64;
        if total > 0 {
            let pct = (downloaded as f64 / total as f64 * 100.0).min(100.0);
            let _ = event_tx.send(WriterEvent::Progress(FlashProgress {
                step: FlashStep::Decompressing,
                percentage: Some(pct),
                ..Default::default()
            })).await;
        }
    }

    let _ = file.flush();
    Some(tmp_path.to_string_lossy().to_string())
}
