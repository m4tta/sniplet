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
    use windows::{
        Win32::{
            Foundation::{
                DRAGDROP_S_CANCEL, DRAGDROP_S_DROP, DRAGDROP_S_USEDEFAULTCURSORS, LPARAM, POINT,
                WPARAM,
            },
            Graphics::Gdi::ScreenToClient,
            System::{
                Com::IDataObject,
                Ole::{
                    DROPEFFECT, DROPEFFECT_COPY, DoDragDrop, IDropSource, IDropSource_Impl,
                    OleInitialize,
                },
                SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS},
            },
            UI::{
                Input::KeyboardAndMouse::GetCapture,
                Shell::{BHID_DataObject, IShellItem, SHCreateItemFromParsingName},
                WindowsAndMessaging::{GetCursorPos, PostMessageW, WM_LBUTTONUP},
            },
        },
        core::{BOOL, HRESULT, HSTRING, implement},
    };

    /// The drag source OLE asks while the drag runs: drop on release, cancel on Escape.
    #[implement(IDropSource)]
    struct Source;

    impl IDropSource_Impl for Source_Impl {
        fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
            if escape.as_bool() {
                DRAGDROP_S_CANCEL
            } else if keys.0 & MK_LBUTTON.0 == 0 {
                DRAGDROP_S_DROP
            } else {
                HRESULT(0)
            }
        }

        fn GiveFeedback(&self, _: DROPEFFECT) -> HRESULT {
            DRAGDROP_S_USEDEFAULTCURSORS
        }
    }

    /// Run a system drag of the file at `path` until it drops or is cancelled. `DoDragDrop`
    /// runs its own message loop on this (the UI) thread and returns when the drag ends.
    pub(super) fn drag_file(path: &std::path::Path) -> Result<(), String> {
        let fail = |error: windows::core::Error| format!("Could not drag image: {error}");
        // SAFETY: COM calls on the UI thread with arguments that outlive them.
        unsafe {
            // winit already initializes OLE on its windows' thread; this only adds a reference.
            let _ = OleInitialize(None);
            // The shell's own data object for the file, as Explorer offers one: CF_HDROP and
            // the shell formats, so Explorer, chat apps and browsers all accept it.
            let item: IShellItem =
                SHCreateItemFromParsingName(&HSTRING::from(path.as_os_str()), None)
                    .map_err(fail)?;
            let data: IDataObject = item.BindToHandler(None, &BHID_DataObject).map_err(fail)?;
            // The window holding the pointer capture: it never sees the release, which
            // `DoDragDrop` takes.
            let window = GetCapture();
            let mut effect = DROPEFFECT::default();
            let result = DoDragDrop(
                &data,
                &IDropSource::from(Source),
                DROPEFFECT_COPY,
                &mut effect,
            );
            // Hand that release back so the press ends and nothing stays pressed.
            if !window.is_invalid() {
                let mut point = POINT::default();
                let _ = GetCursorPos(&mut point);
                let _ = ScreenToClient(window, &mut point);
                let position = (point.x as u16 as u32) | ((point.y as u16 as u32) << 16);
                let _ = PostMessageW(
                    Some(window),
                    WM_LBUTTONUP,
                    WPARAM(0),
                    LPARAM(position as isize),
                );
            }
            result.ok().map_err(fail)
        }
    }
}
