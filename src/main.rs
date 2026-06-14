use gpui::*;
use crepuscularity_gpui::prelude::*;

mod drive;
mod error;
mod state;
mod image_writer;
mod scanner;
mod ui;

use crate::ui::main_page::ArtisanApp;

fn main() {
    // Tokio runtime for async I/O (downloads, block writes, decompression)
    let rt = tokio::runtime::Runtime::new().expect("Failed to start Tokio runtime");
    let _guard = rt.enter();

    Application::new().run(|cx: &mut App| {
        let window_options = gpui_window_options(
            "incisor.app",
            "Artisan",
            Some(gpui::WindowBounds::Windowed(bounds(
                point(px(0.), px(0.)),
                size(px(960.), px(680.)),
            ))),
            Some(Size {
                width: px(800.),
                height: px(600.),
            }),
        );

        cx.open_window(window_options, |_window, cx| cx.new(ArtisanApp::new))
            .unwrap();
    });
}
