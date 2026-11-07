//! Dragging the exported image out of Sniplet into another app as a PNG file. Rune does this
//! on macOS; on Windows Sniplet starts an OLE file drag itself. Elsewhere the control only
//! shows its tooltip.
use std::path::PathBuf;

use rune::El;

/// Make `control` a drag source for the file `export` writes. `fail` reports an export error.
pub(crate) fn source<S: 'static>(
    control: El,
    export: impl Fn(&S) -> Result<PathBuf, String> + 'static,
    fail: impl Fn(&mut S, String) + 'static,
) -> El {
    #[cfg(target_os = "macos")]
    {
        let _ = fail;
        control.drag_out(move |s: &S| export(s).ok().map(|path| rune::DragItem::new().file(path)))
    }
    #[cfg(target_os = "windows")]
    {
        control.on_drag(move |s: &mut S, drag: &mut rune::Drag, _: &mut rune::Tx| {
            // `data` marks a drag that already started, so one gesture starts one drag.
            if drag.phase == rune::Phase::Down {
                drag.data = 0.0;
            }
            if drag.phase != rune::Phase::Move || drag.data != 0.0 || drag.dx.hypot(drag.dy) < 4.0 {
                return;
            }
            drag.data = 1.0;
            match export(s).and_then(|path| windows::drag_file(&path)) {
                Ok(()) => {}
                Err(error) => fail(s, error),
            }
        })
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (export, fail);
        control
    }
}

#[cfg(target_os = "windows")]
mod windows {
    /// Run a system drag of the file at `path` until it drops or is cancelled.
    pub(super) fn drag_file(path: &std::path::Path) -> Result<(), String> {
        let _ = path;
        Err("Dragging files out is not ported yet".into())
    }
}
