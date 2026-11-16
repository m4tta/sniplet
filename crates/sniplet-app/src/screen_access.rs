//! Guides the user to the macOS Screen Recording permission when Sniplet starts without it,
//! or when a capture starts without it. With no image open, a setup card takes the welcome
//! screen's place. A capture shows the card over an open image too. Otherwise, with an image
//! open, a line at the top of the canvas says what to do.
use rune::{El, Tx, col, el, icon, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize, Kit};

use crate::{
    editor::Editor,
    ui::{self, icons},
};

/// Sniplet does not have Screen Recording. macOS applies a new permission only after a
/// restart, so this stays until Sniplet starts again.
#[derive(Default)]
pub struct ScreenAccess {
    /// The user opened System Settings from Sniplet, so the next step is to reopen.
    settings_opened: bool,
    /// A capture did not start, so the card shows even over an open image.
    blocked: bool,
}

impl ScreenAccess {
    pub fn blocked(&self) -> bool {
        self.blocked
    }
}

/// Call before a capture starts. Without Screen Recording on macOS, a capture gets no
/// windows or only the desktop, so this shows the setup steps instead and returns false.
pub fn allows_capture(s: &mut Editor) -> bool {
    #[cfg(target_os = "macos")]
    if !sniplet_platform::screen_recording_allowed() {
        s.screen_access
            .get_or_insert_with(ScreenAccess::default)
            .blocked = true;
        crate::shell::show_editor();
        return false;
    }
    let _ = s;
    true
}

pub fn not_now(s: &mut Editor) {
    if let Some(access) = &mut s.screen_access {
        access.blocked = false;
    }
}

const SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.preference.security?Privacy_ScreenCapture";

const MESSAGE: &str = "To capture, allow Sniplet in System Settings > Privacy & Security > Screen & System Audio Recording, then quit and reopen Sniplet.";

fn open_settings(s: &mut Editor) {
    if let Some(access) = &mut s.screen_access {
        access.settings_opened = true;
    }
    if let Err(error) = std::process::Command::new("open").arg(SETTINGS_URL).spawn() {
        s.status = format!("Could not open System Settings: {error}");
    }
}

/// Quit, and start Sniplet again after this copy exits, so that macOS applies the permission.
fn reopen(s: &mut Editor, tx: &mut Tx) {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(error) => {
            s.status = format!("Could not reopen Sniplet: {error}");
            return;
        }
    };
    // Open the app bundle, so that macOS starts Sniplet as an app. A bare binary, as from
    // `cargo run`, starts directly.
    let target = exe
        .ancestors()
        .find(|path| path.extension().is_some_and(|ext| ext == "app"))
        .unwrap_or(&exe)
        .to_owned();
    // A second copy would hand off to this one and exit, so wait for this one to end.
    let script = r#"while kill -0 "$0" 2>/dev/null; do sleep 0.1; done
case "$1" in *.app) exec open "$1" ;; *) exec "$1" ;; esac"#;
    let spawned = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(script)
        .arg(std::process::id().to_string())
        .arg(&target)
        .spawn();
    match spawned {
        Ok(_) => s.quit(tx),
        Err(error) => s.status = format!("Could not reopen Sniplet: {error}"),
    }
}

/// The setup card. With no image, it takes the welcome screen's place. Over an image, it
/// gets a backdrop and a way to close it, and it does not quit, which would lose the image.
pub fn setup_card(access: &ScreenAccess, has_image: bool, k: &Kit) -> El {
    let opened = access.settings_opened;
    let open_button = if opened {
        Button::new("Open it again")
            .variant(ButtonVariant::Ghost)
            .control_size(ControlSize::Regular)
            .on_press(open_settings)
            .build(k)
    } else {
        Button::new("Open System Settings")
            .variant(ButtonVariant::Contrast)
            .control_size(ControlSize::Regular)
            .icon(&icons::SETTINGS)
            .on_press(open_settings)
            .build(&ui::accent(k))
    };
    let reopen_button = Button::new("Reopen Sniplet")
        .variant(if opened {
            ButtonVariant::Contrast
        } else {
            ButtonVariant::Neutral
        })
        .control_size(ControlSize::Regular)
        .on_press_tx(reopen)
        .build(&if opened { ui::accent(k) } else { k.clone() });
    let card = col()
        .w(440.0)
        .px(24.0)
        .py(24.0)
        .gap(18.0)
        .child(
            col()
                .gap(4.0)
                .child(
                    text("Allow Sniplet to capture your screen")
                        .font(k.ty.body_medium)
                        .color(k.c.text_base)
                        .wrap(),
                )
                .child(note(
                    k,
                    "macOS asks for this one time. It takes three steps.",
                )),
        )
        .child(step(
            k,
            1,
            if opened { Mark::Done } else { Mark::Current },
            "Open System Settings",
            (!opened).then_some("Sniplet opens the Screen & System Audio Recording list."),
            Some(open_button.key("screen-access-open")),
        ))
        .child(step(
            k,
            2,
            if opened { Mark::Current } else { Mark::Next },
            "Turn on Sniplet",
            Some("Find Sniplet in the list and turn on its switch."),
            None,
        ))
        .child(if has_image {
            step(
                k,
                3,
                Mark::Next,
                "Save your image, then quit and reopen Sniplet",
                Some("macOS applies the change after Sniplet starts again."),
                None,
            )
        } else {
            step(
                k,
                3,
                Mark::Next,
                "Reopen Sniplet",
                Some("macOS applies the change after Sniplet starts again."),
                Some(reopen_button.key("screen-access-reopen")),
            )
        });
    let card = if has_image {
        card.child(
            row().justify_end().child(
                Button::new("Not now")
                    .variant(ButtonVariant::Ghost)
                    .control_size(ControlSize::Regular)
                    .on_press(not_now)
                    .build(k)
                    .key("screen-access-not-now")
                    .focusable(false),
            ),
        )
    } else {
        card
    };
    let layer = el()
        .key("screen-access")
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .center();
    let layer = if has_image {
        // Over an image, a backdrop takes presses, so the image cannot be edited behind it.
        layer
            .fill(rune::Color::BLACK.alpha(0.55))
            .on_press(|_: &mut Editor| {})
    } else {
        layer
    };
    layer.child(ui::surface::<Editor>(k, card))
}

/// The fallback while an image is open: the message, with a button to System Settings.
pub fn notice(k: &Kit) -> El {
    ui::chip(
        k,
        row()
            .gap(10.0)
            .center_y()
            .child(
                text(MESSAGE)
                    .font(k.ty.text_xsmall)
                    .color(k.c.text_base)
                    .wrap()
                    .shrink()
                    .min_w(0.0),
            )
            .child(
                Button::new("Open System Settings")
                    .variant(ButtonVariant::Contrast)
                    .control_size(ControlSize::Small)
                    .on_press(open_settings)
                    .build(&ui::accent(k))
                    .key("screen-access-notice-open")
                    .focusable(false),
            ),
    )
    .key("screen-access-notice")
}

#[derive(Clone, Copy)]
enum Mark {
    Done,
    Current,
    Next,
}

fn step(
    k: &Kit,
    number: u8,
    mark: Mark,
    title: &str,
    detail: Option<&str>,
    action: Option<El>,
) -> El {
    let (fill, color) = match mark {
        Mark::Done => (k.c.success.bg, k.c.success.icon),
        Mark::Current => (k.c.bg_accent, rune::Color::WHITE),
        Mark::Next => (k.c.layer_02, k.c.text_muted),
    };
    let badge = el()
        .size(24.0, 24.0)
        .center()
        .radius(12.0)
        .fill(fill)
        .child(match mark {
            Mark::Done => icon(&icons::CIRCLE_CHECK, 14.0).color(color),
            _ => text(number.to_string())
                .font(k.ty.text_xsmall_medium)
                .color(color),
        });
    let mut body = col()
        .grow(1.0)
        .shrink()
        .min_w(0.0)
        .gap(4.0)
        .child(text(title).font(k.ty.ui_medium).color(k.c.text_base));
    if let Some(detail) = detail {
        body = body.child(note(k, detail));
    }
    if let Some(action) = action {
        body = body.child(row().pt(4.0).child(action.focusable(false)));
    }
    row().gap(12.0).child(badge).child(body)
}

fn note(k: &Kit, line: &str) -> El {
    text(line)
        .font(k.ty.text_xsmall)
        .color(k.c.text_muted)
        .wrap()
}
