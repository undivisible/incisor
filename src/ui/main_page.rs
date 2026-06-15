use gpui::*;
use crepuscularity_gpui::prelude::*;
use crate::state::{AppModel, Page};
use crate::drive::{Drive, ImageInfo, SourceType};
use crate::scanner::scan_drives;
use crate::image_writer::{self, WriterEvent, FlashHandle};
use std::collections::HashSet;

pub struct ArtisanApp {
    pub model: AppModel,
    flash_handle: Option<FlashHandle>,
}

impl ArtisanApp {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let weak = cx.weak_entity();
        cx.spawn(async move |_this, cx| {
            let drives = tokio::task::spawn_blocking(|| scan_drives()).await.unwrap_or_default();
            let _ = weak.update(cx, |app, cx| {
                app.model.available_drives = drives;
                app.model.drives_loaded = true;
                app.sort_and_autoselect();
                cx.notify();
            });
        }).detach();

        Self { model: AppModel::new(), flash_handle: None }
    }

    fn sort_and_autoselect(&mut self) {
        // Sort: system drives last, then by device path
        self.model.available_drives.sort_by(|a, b| {
            if a.is_system != b.is_system {
                return a.is_system.cmp(&b.is_system);
            }
            a.device.cmp(&b.device)
        });

        // Auto-select single valid non-system non-readonly drive
        let valid: Vec<String> = self.model.available_drives
            .iter()
            .filter(|d| {
                d.is_valid(self.model.image.as_ref(), true)
                    && !d.is_read_only
                    && d.size.unwrap_or(0) > 0
            })
            .map(|d| d.device.clone())
            .collect();

        if valid.len() == 1 {
            self.model.select_drive(&valid[0]);
        }
    }

    // ── Event handlers ──────────────────────────────────────────────

    pub fn open_settings(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.model.settings_open = true;
        cx.notify();
    }

    pub fn close_settings(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.model.settings_open = false;
        cx.notify();
    }

    pub fn select_file(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let file = rfd::FileDialog::new()
            .add_filter("Disk Images", &["img", "iso", "dmg", "raw", "bin", "gz", "bz2", "xz", "zip", "rpi-sdimg", "wic"])
            .add_filter("All files", &["*"])
            .pick_file();

        if let Some(path) = file {
            let size = std::fs::metadata(&path).ok().map(|m| m.len()).unwrap_or(0);
            let ext = path.extension().and_then(|e| e.to_str()).map(|s| s.to_lowercase());
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();

            self.model.set_image(ImageInfo {
                path: Some(path.to_string_lossy().to_string()),
                name: Some(name),
                size,
                extension: ext,
                source_type: SourceType::File,
                ..Default::default()
            });
            self.sort_and_autoselect();
            cx.notify();
        }
    }

    pub fn show_url_modal(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.model.url_input_open = true;
        self.model.url_text = String::new();
        cx.notify();
    }

    pub fn close_url_modal(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.model.url_input_open = false;
        cx.notify();
    }

    pub fn submit_url(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let url = self.model.url_text.trim().to_string();
        if url.is_empty() {
            self.model.url_input_open = false;
            cx.notify();
            return;
        }

        self.model.set_image(ImageInfo {
            url: Some(url.clone()),
            name: Some(url.rsplit('/').next().unwrap_or(&url).to_string()),
            size: 0, // unknown until download
            extension: None,
            source_type: SourceType::Http,
            is_size_estimated: true,
            ..Default::default()
        });
        self.model.url_input_open = false;
        self.sort_and_autoselect();
        cx.notify();
    }

    pub fn paste_url(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        // Read clipboard via osascript on macOS
        #[cfg(target_os = "macos")]
        {
            if let Ok(out) = std::process::Command::new("osascript")
                .args(["-e", "the clipboard"])
                .output()
            {
                if out.status.success() {
                    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if !text.is_empty() {
                        self.model.url_text = text;
                        cx.notify();
                    }
                }
            }
        }
        #[cfg(target_os = "linux")]
        {
            if let Ok(out) = std::process::Command::new("xclip").args(["-o", "-selection", "clipboard"]).output() {
                if out.status.success() {
                    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if !text.is_empty() {
                        self.model.url_text = text;
                        cx.notify();
                    }
                }
            }
        }
        #[cfg(target_os = "windows")]
        {
            if let Ok(out) = std::process::Command::new("powershell").args(["-Command", "Get-Clipboard"]).output() {
                if out.status.success() {
                    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if !text.is_empty() {
                        self.model.url_text = text;
                        cx.notify();
                    }
                }
            }
        }
    }

    pub fn clear(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.model.clear_selection();
        cx.notify();
    }

    pub fn change_source(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.model.clear_image();
        cx.notify();
    }

    pub fn go_to_main(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.model.page = Page::Main;
        self.model.flash_results = None;
        self.model.is_flashing = false;
        self.flash_handle = None;
        cx.notify();
    }

    pub fn cancel_flash(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ref h) = self.flash_handle {
            h.cancel();
        }
        cx.notify();
    }

    pub fn flash(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let image = match self.model.image.clone() {
            Some(img) => img,
            None => return,
        };

        let drives = self.model.selected_drives();
        if drives.is_empty() { return; }

        // Check for warnings on selected drives
        let has_warnings = drives.iter().any(|d| {
            !d.compatibility_statuses(Some(&image), true).is_empty()
        });

        if has_warnings {
            let warnings: Vec<_> = drives.iter()
                .filter(|d| !d.compatibility_statuses(Some(&image), true).is_empty())
                .map(|d| (d.clone(), d.compatibility_statuses(Some(&image), true)))
                .collect();
            let is_system = drives.iter().any(|d| d.is_system);
            self.model.warning_message = Some(if is_system {
                "You are about to flash to a system drive. This may render your system unbootable.".into()
            } else {
                "You are about to flash to a large drive. Make sure it doesn't contain important data.".into()
            });
            self.model.drives_with_warnings = warnings;
            cx.notify();
            return;
        }

        self.begin_flash(cx);
    }

    pub fn close_warning_continue(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.model.warning_message = None;
        self.begin_flash(cx);
    }

    pub fn close_warning_cancel(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.model.warning_message = None;
        cx.notify();
    }

    pub fn close_error(&mut self, _: &ClickEvent, _window: &mut Window, cx: &mut Context<Self>) {
        self.model.error_message = None;
        cx.notify();
    }

    fn begin_flash(&mut self, cx: &mut Context<Self>) {
        let image = match self.model.image.clone() {
            Some(img) => img,
            None => return,
        };
        let drives = self.model.selected_drives();
        if drives.is_empty() { return; }

        self.model.is_flashing = true;
        self.model.flash_progress = Default::default();
        let (tx, mut rx) = tokio::sync::mpsc::channel::<WriterEvent>(64);

        let weak = cx.weak_entity();
        let handle = image_writer::start_flash(image, drives.clone(), tx);
        self.flash_handle = Some(handle);

        // Flash event loop runs on GPUI's executor. Tokio runtime is entered at startup.
        cx.spawn(async move |_this, cx| {
            use crate::state::FlashStep;
            while let Some(evt) = rx.recv().await {
                let weak = weak.clone();
                match evt {
                    WriterEvent::Progress(p) => {
                        let _ = weak.update(cx, |app, cx| {
                            app.model.flash_progress = p;
                            cx.notify();
                        });
                    }
                    WriterEvent::Fail(msg) => {
                        let _ = weak.update(cx, |app, cx| {
                            app.model.is_flashing = false;
                            app.model.flash_progress.step = FlashStep::Failed;
                            app.model.error_message = Some(msg);
                            cx.notify();
                        });
                    }
                    WriterEvent::Done(results) => {
                        let _ = weak.update(cx, |app, cx| {
                            app.model.is_flashing = false;
                            app.model.flash_results = Some(results.clone());
                            app.model.page = Page::Success;
                            app.flash_handle = None;

                            let title = if results.cancelled {
                                "Flash cancelled"
                            } else if results.successful > 0 {
                                "Flash complete!"
                            } else {
                                "Flash failed"
                            };
                            let body = format!("{} successful, {} failed", results.successful, results.failed);
                            notify_os(title, &body);

                            cx.notify();
                        });
                    }
                }
            }
        }).detach();
    }

    // ── Drive list rendering ────────────────────────────────────────

    fn render_drive_list(
        drives: &[Drive],
        selected: &HashSet<String>,
        weak: &gpui::WeakEntity<Self>,
        is_flashing: bool,
    ) -> AnyElement {
        if drives.is_empty() {
            return div()
                .px_2()
                .py_1()
                .text_xs()
                .text_color(rgb(0x757575))
                .child("No external drives detected. Plug one in.")
                .into_any_element();
        }

        div()
            .flex()
            .flex_col()
            .gap_1()
            .children(drives.iter().map(|drive| {
                let checked = selected.contains(&drive.device);
                let display = drive.display_name.clone();
                let is_ro = drive.is_read_only || is_flashing;
                let device = drive.device.clone();
                let w = weak.clone();
                let size_label = drive.size.map(|s| pretty_size(s)).unwrap_or_default();

                let mut item = div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1()
                    .text_color(if is_ro { rgb(0x757575) } else { rgb(0xe0e0e0) })
                    .id(SharedString::from(format!("drive-{}", device)));

                if !is_ro {
                    item = item.cursor_pointer();
                    item = item.on_click(move |_event, _window, app| {
                        let _ = w.update(app, |entity, cx| {
                            entity.model.toggle_drive(&device);
                            cx.notify();
                        });
                    });
                }

                item
                    .child(
                        div()
                            .w(px(14.)).h(px(14.))
                            .border_1()
                            .border_color(if checked { rgb(0x00aeef) } else { rgb(0x3a3a3a) })
                            .bg(if checked { rgb(0x00aeef) } else { rgb(0x1a1a1a) })
                            .flex().items_center().justify_center()
                            .child(if checked { div().text_color(white()).text_xs().child("✓") } else { div() }),
                    )
                    .child(div().text_xs().child(display))
                    .child(div().flex_1())
                    .child(div().text_xs().text_color(rgb(0x757575)).child(size_label))
            }))
            .into_any_element()
    }
}

impl Render for ArtisanApp {
    #[allow(refining_impl_trait)]
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let m = &self.model;
        if m.page == Page::Success { return self.render_success(cx); }

        let weak = cx.weak_entity();
        let has_image = m.has_image();
        let has_drive = m.has_drive();
        let can_flash = has_image && has_drive && !m.is_flashing;
        let image_name: SharedString = m.image_basename().into();
        let drive_title: SharedString = m.drive_title().into();
        let err_msg = m.error_message.clone();
        let warn_msg = m.warning_message.clone();
        let settings_open = m.settings_open;
        let is_flashing = m.is_flashing;
        let url_open = m.url_input_open;
        let url_text = m.url_text.clone();
        let p = &m.flash_progress;
        let speed_txt: SharedString = p.speed.map(|s| format!("{:.2} MB/s", s)).unwrap_or_default().into();
        let eta_txt: SharedString = p.eta.map(|e| format!("{:.0}s", e)).unwrap_or_default().into();

        let pct = p.percentage.unwrap_or(0.0) as f32;
        let bar_fill = div()
            .h(px(12.))
            .w(px(pct / 100.0 * 200.0))
            .bg(rgb(0xda60ff))
            .rounded_full();

        let drive_list = Self::render_drive_list(&m.available_drives, &m.selected_devices, &weak, is_flashing);

        let main_view: gpui::Div = view! {r#"
            div w-full h-full bg-zinc-950 text-white flex flex-col items-center

                # Header — centered title
                div w-full flex items-center justify-center px-4 py-3 border-b border-zinc-800
                    div text-lg font-bold tracking-wide "incisor"
                    div absolute right-4
                        button bg-transparent text-zinc-400 border-none cursor-pointer @click=open_settings
                            "⚙"

                # Main content — vertically + horizontally centered
                div flex-1 flex flex-col items-center justify-center w-full max-w-[700px] mx-auto px-8

                    # Three-step row
                    div flex items-center justify-center gap-3 w-full

                        # Source step
                        div flex flex-col items-center gap-3 min-w-[160px]
                            div w-12 h-12 bg-zinc-800 rounded-xl flex items-center justify-center
                                if {has_image}
                                    div text-xl "📁"
                                else
                                    div text-xl text-zinc-500 "📁"
                            if {is_flashing}
                                div text-xs text-zinc-400 text-center truncate max-w-[140px] "{image_name}"
                            else if {has_image}
                                div flex flex-col items-center gap-1
                                    div text-xs text-zinc-300 text-center truncate max-w-[140px] "{image_name}"
                                    button bg-zinc-800 text-zinc-400 text-2xs rounded @click=change_source
                                        "change"
                            else
                                div flex flex-col items-center gap-1
                                    div text-xs text-zinc-500 "Select source"
                                    button bg-blue-600 text-white text-xs px-4 py-2 rounded-md @click=select_file
                                        "Flash from file"
                                    button bg-zinc-800 text-zinc-400 text-xs px-4 py-2 rounded-md @click=show_url_modal
                                        "Flash from URL"

                        # Step divider
                        div w-16 h-0.5 bg-zinc-700

                        # Target step
                        div flex flex-col items-center gap-3 min-w-[160px]
                            div w-12 h-12 bg-zinc-800 rounded-xl flex items-center justify-center
                                div text-xl "💾"
                            div text-xs text-center
                                if {has_drive}
                                    span text-zinc-300 "{drive_title}"
                                else
                                    span text-zinc-500 "Select target"
                            # Drive list (inline dropdown)
                            if {!is_flashing && !m.available_drives.is_empty()}
                                div w-full mt-1 bg-zinc-900 rounded-lg border border-zinc-800
                                    {drive_list}
                            else
                                if {m.drives_loaded}
                                    div text-2xs text-zinc-600 "No external drives found"
                                else
                                    div text-2xs text-zinc-500 "Scanning..."

                        # Step divider
                        div w-16 h-0.5 bg-zinc-700

                        # Flash step
                        div flex flex-col items-center gap-3 min-w-[160px]
                            div w-12 h-12 bg-zinc-800 rounded-xl flex items-center justify-center
                                div text-xl "⚡"
                            if {is_flashing}
                                div text-xs text-zinc-400 "Flashing..."
                                div w-full h-2 bg-zinc-800 rounded-full overflow-hidden
                                    {bar_fill}
                                div flex justify-between w-full text-2xs text-zinc-600
                                    div "{speed_txt}"
                                    div "{eta_txt}"
                                button bg-red-700 text-white text-xs w-full px-3 py-1.5 rounded-md @click=cancel_flash
                                    "Cancel"
                            else
                                div text-xs text-zinc-500 "Flash!"
                                if {can_flash}
                                    button bg-blue-600 text-white text-sm px-8 py-2.5 rounded-md @click=flash
                                        "Flash!"
                                else
                                    if {has_image && !has_drive}
                                        button bg-zinc-700 text-zinc-500 text-sm px-8 py-2.5 rounded-md
                                            "Select targets"
                                    else
                                        button bg-zinc-700 text-zinc-500 text-sm px-8 py-2.5 rounded-md
                                            "Flash!"

                    # Error banner
                    if {err_msg.is_some()}
                        div mt-6 px-4 py-2 bg-red-900/30 border border-red-800 rounded-md
                            div flex items-center justify-between gap-4
                                div text-xs text-red-400 break-all "{err_msg.as_ref().unwrap()}"
                                button bg-transparent text-red-400 border-none cursor-pointer @click=close_error
                                    "✕"

                # Warning overlay
                if {warn_msg.is_some()}
                    div absolute top-0 left-0 w-full h-full bg-black/60 flex items-center justify-center
                        div bg-zinc-800 rounded-xl p-6 min-w-[380px] shadow-xl
                            div text-base text-zinc-100 font-bold "Warning"
                            div h-3
                            div text-sm text-zinc-300 leading-relaxed break-all "{warn_msg.as_ref().unwrap()}"
                            div h-6
                            div flex gap-3 justify-end
                                button bg-amber-600 text-white text-sm px-5 py-2 rounded-md @click=close_warning_continue
                                    "Continue"
                                button bg-zinc-700 text-zinc-300 text-sm px-5 py-2 rounded-md @click=close_warning_cancel
                                    "Cancel"

                # URL input modal
                if {url_open}
                    div absolute top-0 left-0 w-full h-full bg-black/60 flex items-center justify-center
                        div bg-zinc-800 rounded-xl p-6 min-w-[420px] shadow-xl
                            div flex justify-between items-center mb-4
                                div text-base font-bold text-zinc-100 "Flash from URL"
                                button bg-transparent text-zinc-400 border-none cursor-pointer @click=close_url_modal
                                    "✕"
                            div text-sm text-zinc-300 "Enter a URL or paste from clipboard"
                            div h-2
                            if {url_text.is_empty()}
                                div text-xs text-zinc-500 "(paste a URL with the button below)"
                            else
                                div text-xs text-zinc-300 break-all "{url_text}"
                            div h-3
                            button bg-zinc-700 text-zinc-200 text-sm px-4 py-2 rounded-md w-full @click=paste_url
                                "Paste from clipboard"
                            div h-4
                            div flex gap-2 justify-end
                                button bg-zinc-700 text-zinc-300 text-sm px-4 py-2 rounded-md @click=close_url_modal
                                    "Cancel"
                                button bg-blue-600 text-white text-sm px-4 py-2 rounded-md @click=submit_url
                                    "Fetch"

                # Settings modal
                if {settings_open}
                    div absolute top-0 left-0 w-full h-full bg-black/60 flex items-center justify-center
                        div bg-zinc-800 rounded-xl p-6 min-w-[400px] shadow-xl
                            div flex justify-between items-center mb-5
                                div text-base font-bold text-zinc-100 "Settings"
                                button bg-transparent text-zinc-400 border-none cursor-pointer @click=close_settings
                                    "✕"
                            div flex flex-col gap-4
                                div flex items-center justify-between
                                    div text-sm text-zinc-100 "Safe write (verify after flash)"
                                    div w-9 h-5 bg-blue-600 rounded-full p-0.5
                                        div w-4 h-4 bg-white rounded-full ml-auto
                                div flex items-center justify-between
                                    div text-sm text-zinc-100 "Auto-select single drive"
                                    div w-9 h-5 bg-zinc-700 rounded-full p-0.5
                                        div w-4 h-4 bg-white rounded-full
                                div flex items-center justify-between
                                    div text-sm text-zinc-100 "OS notifications"
                                    div w-9 h-5 bg-blue-600 rounded-full p-0.5
                                        div w-4 h-4 bg-white rounded-full ml-auto
        "#};
        main_view.into_any_element()
    }
}

// ── Success page ──────────────────────────────────────────────────

impl ArtisanApp {
    fn render_success(&self, cx: &mut Context<Self>) -> AnyElement {
        let r = &self.model.flash_results;
        let is_ok = r.as_ref().map_or(true, |r| !r.cancelled && r.successful > 0);
        let icon = if is_ok { "✓" } else { "✗" };
        let title = if let Some(ref r) = r {
            if r.cancelled { "Cancelled" } else if r.failed > 0 && r.successful == 0 { "Flash failed" } else { "Flash complete!" }
        } else { "Flash complete!" };
        let detail: SharedString = r.as_ref().map(|r| format!("{} successful, {} failed", r.successful, r.failed)).unwrap_or_default().into();

        if is_ok {
            let v: gpui::Div = view! {r#"
                div w-full h-full bg-zinc-950 text-white flex flex-col
                    div flex items-center justify-between px-4 py-2 border-b border-zinc-800
                        div flex-1
                        div text-xl font-bold "incisor"
                        div flex-1
                    div flex-1 flex flex-col items-center justify-center gap-4
                        div w-16 h-16 rounded-full bg-green-500 flex items-center justify-center text-2xl font-bold text-white
                            "{icon}"
                        div text-2xl font-bold text-green-400 "{title}"
                        if {!detail.is_empty()}
                            div text-sm text-zinc-400 "{detail}"
                        button bg-blue-600 text-white px-6 py-3 rounded-md text-sm @click=go_to_main "Flash another"
            "#};
            v.into_any_element()
        } else {
            let v: gpui::Div = view! {r#"
                div w-full h-full bg-zinc-950 text-white flex flex-col
                    div flex items-center justify-between px-4 py-2 border-b border-zinc-800
                        div flex-1
                        div text-xl font-bold "incisor"
                        div flex-1
                    div flex-1 flex flex-col items-center justify-center gap-4
                        div w-16 h-16 rounded-full bg-red-500 flex items-center justify-center text-2xl font-bold text-white
                            "{icon}"
                        div text-2xl font-bold text-red-400 "{title}"
                        if {!detail.is_empty()}
                            div text-sm text-zinc-400 "{detail}"
                        button bg-blue-600 text-white px-6 py-3 rounded-md text-sm @click=go_to_main "Try again"
            "#};
            v.into_any_element()
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────

fn pretty_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1000.0 && unit < UNITS.len() - 1 { size /= 1000.0; unit += 1; }
    format!("{:.1} {}", size, UNITS[unit])
}

fn notify_os(title: &str, body: &str) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("osascript")
            .args(["-e", &format!("display notification \"{}\" with title \"Artisan\" subtitle \"{}\"", body, title)])
            .output();
    }
    #[cfg(target_os = "linux")]
    {
        let _ = notify_rust::Notification::new()
            .summary(title)
            .body(body)
            .appname("Artisan")
            .timeout(notify_rust::Timeout::Milliseconds(5000))
            .show();
    }
    #[cfg(target_os = "windows")]
    {
        let _ = notify_rust::Notification::new()
            .summary(title)
            .body(body)
            .appname("Artisan")
            .show();
    }
}
