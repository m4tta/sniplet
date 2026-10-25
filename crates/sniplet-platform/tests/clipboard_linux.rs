#![cfg(target_os = "linux")]

use std::process::Command;

use image::{Rgba, RgbaImage};
use sniplet_platform::Clipboard;

const PROBE: &str = "SNIPLET_CLIPBOARD_PROBE";
const TEXT: &str = "Sniplet clipboard · 日本語\nsecond line";

fn fixture() -> RgbaImage {
    RgbaImage::from_fn(3, 2, |x, y| {
        Rgba([(x * 71) as u8, (y * 113) as u8, 153, (x * 100) as u8])
    })
}

/// Run on an isolated display without a clipboard manager. The writer must
/// remain the clipboard owner after its worker and public handle are gone.
#[test]
#[ignore = "requires a Linux desktop; run under xvfb-run without a clipboard manager"]
fn copied_data_outlives_worker_and_handle() {
    if let Ok(kind) = std::env::var(PROBE) {
        // A fresh process and a raw backend prevent an in-process cache from
        // satisfying the read or accidentally keeping the writer alive.
        let mut reader = arboard::Clipboard::new().unwrap();
        match kind.as_str() {
            "image" => {
                let actual = reader.get_image().unwrap();
                let expected = fixture();
                assert_eq!((actual.width, actual.height), (3, 2));
                assert_eq!(actual.bytes.as_ref(), expected.as_raw());
            }
            "text" => assert_eq!(reader.get_text().unwrap(), TEXT),
            _ => panic!("unknown clipboard probe: {kind}"),
        }
        return;
    }

    // Replace both types repeatedly, as capture, Copy, OCR, and link copying
    // do. Each write uses a new worker and drops its handle before any paste.
    for kind in ["image", "text", "image", "text", "image"] {
        std::thread::spawn(move || {
            let mut writer = Clipboard::new().unwrap();
            match kind {
                "image" => writer.set_image(&fixture()).unwrap(),
                "text" => writer.set_text(TEXT).unwrap(),
                _ => unreachable!(),
            }
        })
        .join()
        .unwrap();

        // Multiple independent pastes must work without recreating a writer.
        for _ in 0..2 {
            let output = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--ignored",
                    "--exact",
                    "copied_data_outlives_worker_and_handle",
                    "--nocapture",
                ])
                .env(PROBE, kind)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "independent {kind} paste failed:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
        }
    }
}
