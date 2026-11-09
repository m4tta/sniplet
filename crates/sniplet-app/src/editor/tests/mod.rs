//! Headless tests that drive the real editor view through Rune's `Runtime`: pointer presses
//! and drags on elements by key, keys through the same `on_key` the window uses, and
//! background work through the editor's `Inbox`.
use rune::{Env, Key, KeyEvent, Mods, PhysicalInput, PointerButton, Runtime};
use sniplet_core::{Annotation, AnnotationKind, Document, Point as ImagePoint};

use super::Editor;

mod drawing;
mod panels;
mod sizes;

#[test]
fn upload_keys_have_a_random_part() {
    // Without one, two keys made in the same millisecond are equal.
    let keys = [(); 2].map(|_| super::commands::upload_key().unwrap());
    assert_ne!(keys[0], keys[1]);
}

#[test]
fn a_capture_too_large_to_append_leaves_the_document_unchanged() {
    // A 100000 x 1500 canvas is over the image size limit.
    let mut h = Harness::new(
        Document::new(image::RgbaImage::new(100_000, 1)),
        WINDOW,
        true,
    );
    h.update(|editor| editor.add_image(image::RgbaImage::new(1, 1_500), "Capture", true));
    assert!(h.annotations().is_empty());
    assert_eq!((h.document().width(), h.document().height()), (100_000, 1));
    assert!(!h.document().can_undo());
    assert!(
        h.editor().status.contains("too large"),
        "{}",
        h.editor().status
    );
}

/// One editor window: the runtime, and the temp folder its settings write to.
struct Harness {
    rt: Runtime<Editor>,
    _settings: tempfile::TempDir,
}

/// The default window, in points.
const WINDOW: (f32, f32) = (1280.0, 796.0);

/// A dark editor window showing a 400 × 300 demo image.
fn editor() -> Harness {
    Harness::new(
        Document::new(sniplet_core::demo_image(400, 300)),
        WINDOW,
        true,
    )
}

impl Harness {
    fn new(document: Document, size: (f32, f32), dark: bool) -> Self {
        let settings = tempfile::tempdir().unwrap();
        let mut editor = Editor::new(Some(document), Default::default(), 1.0);
        editor.settings_path = Some(settings.path().join("settings.json"));
        let mut rt = Runtime::new(
            editor,
            super::view,
            size,
            Env {
                system_dark: dark,
                scale: 2.0,
            },
        );
        // As `main.rs` wires the window: unclaimed keys (modifier keys too) reach the editor,
        // and every frame drains its inbox.
        rt.on_key = Some(super::on_key);
        rt.set_modifier_keys(true);
        // Elements land where layout puts them at once, so a test reads final positions.
        rt.reduced_motion = true;
        rt.poll = Some(|editor: &mut Editor, _: f64, tx: &mut rune::Tx| {
            crate::ui::Inbox::drain(editor, |e| &e.inbox, tx)
        });
        let mut harness = Self {
            rt,
            _settings: settings,
        };
        harness.settle();
        harness
    }

    fn editor(&self) -> &Editor {
        &self.rt.state
    }

    /// Change the editor directly, as a capture or another window would, and draw it.
    fn update(&mut self, change: impl FnOnce(&mut Editor)) {
        change(&mut self.rt.state);
        self.rt.invalidate();
        self.settle();
    }

    fn document(&self) -> &Document {
        self.editor().document.as_ref().unwrap()
    }

    fn annotations(&self) -> &[Annotation] {
        self.document().annotations()
    }

    /// The only annotation, which must exist.
    fn only(&self) -> Annotation {
        assert_eq!(self.annotations().len(), 1, "expected one annotation");
        self.annotations()[0].clone()
    }

    fn frame(&mut self) {
        self.rt.frame(1.0 / 60.0);
    }

    /// Run frames until queued input, background jobs and preview renders are done.
    fn settle(&mut self) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
        loop {
            self.frame();
            let editor = self.editor();
            if editor.inbox.is_idle() && !editor.preview_running {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "background work did not finish"
            );
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        // One more frame draws what the last job changed, with every animation landed.
        self.rt.settle_all();
        self.frame();
    }

    /// The window point, in points, of an image point.
    fn at(&self, x: f32, y: f32) -> (f32, f32) {
        let (canvas, _) = self.rt.presented("editor-canvas").unwrap();
        let p = self.editor().canvas_point(ImagePoint::new(x, y));
        (canvas[0] + p.x, canvas[1] + p.y)
    }

    /// The center of the element keyed `key`, which must be on screen.
    fn find(&self, key: &str) -> (f32, f32) {
        self.rt
            .find(key)
            .unwrap_or_else(|| panic!("{key} is not on screen"))
    }

    fn shows(&self, key: &str) -> bool {
        self.rt.find(key).is_some()
    }

    /// The drawn rect `[x, y, w, h]` of the element keyed `key`.
    fn rect(&self, key: &str) -> [f32; 4] {
        self.rt
            .presented(key)
            .unwrap_or_else(|| panic!("{key} is not on screen"))
            .0
    }

    /// Move to (x, y) and press there, a frame later as a hand would.
    fn down(&mut self, (x, y): (f32, f32)) {
        self.rt.pointer_move(x, y);
        self.frame();
        self.rt.pointer_down(x, y);
        self.frame();
    }

    fn move_to(&mut self, (x, y): (f32, f32)) {
        self.rt.pointer_move(x, y);
        self.frame();
    }

    fn up(&mut self, (x, y): (f32, f32)) {
        self.rt.pointer_up(x, y);
        self.settle();
    }

    /// Press at `from`, move there in steps to `to`, and release.
    fn drag(&mut self, from: (f32, f32), to: (f32, f32)) {
        self.drag_with(PointerButton::Primary, from, to);
    }

    /// Press `button` at `from`, move there in steps to `to`, and release it, as the window
    /// passes each mouse button to the runtime.
    fn drag_with(&mut self, button: PointerButton, from: (f32, f32), to: (f32, f32)) {
        self.move_to(from);
        self.rt.button_down(from.0, from.1, button);
        self.frame();
        for step in 1..=8 {
            let t = step as f32 / 8.0;
            self.move_to((from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t));
        }
        self.rt.button_up(to.0, to.1, button);
        self.settle();
        // A later press at the same place must not pair into a double click.
        self.rt.frame(1.0);
    }

    /// Drag between two image points.
    fn drag_image(&mut self, from: (f32, f32), to: (f32, f32)) {
        let (from, to) = (self.at(from.0, from.1), self.at(to.0, to.1));
        self.drag(from, to);
    }

    /// Press and release at a window point.
    fn click_at(&mut self, point: (f32, f32)) {
        self.drag(point, point);
    }

    /// Press and release at an image point.
    fn click_image(&mut self, x: f32, y: f32) {
        self.click_at(self.at(x, y));
    }

    /// Press and release the element keyed `key`.
    fn click(&mut self, key: &str) {
        self.click_at(self.find(key));
    }

    /// Hold or release Shift, Ctrl and Alt for the pointer input that follows.
    fn hold(&mut self, mods: Mods) {
        self.rt.physical_input(PhysicalInput::Modifiers(mods));
        self.frame();
    }

    /// Press a key such as `"a"`, `"escape"`, `"shift-right"` or `"ctrl-shift-z"`.
    fn press(&mut self, keys: &str) {
        self.rt.key(key(keys));
        self.settle();
    }

    /// The OS auto-repeat of a held key.
    fn repeat(&mut self, keys: &str) {
        self.rt.repeat_key(key(keys));
        self.settle();
    }

    fn release(&mut self, keys: &str) {
        self.rt.key_up(key(keys));
        self.settle();
    }

    /// Type text into the focused field.
    fn type_text(&mut self, text: &str) {
        self.rt.key(KeyEvent::from(Key::Text(text.into())));
        self.settle();
    }

    fn undo(&mut self) {
        self.press("ctrl-z");
    }

    fn redo(&mut self) {
        self.press("ctrl-shift-z");
    }

    fn export(&self) -> image::RgbaImage {
        self.editor().export_pixels().unwrap()
    }
}

/// A key event from a name such as `"ctrl-shift-z"`: modifiers, then a key name or character
/// (`"minus"` for `-`). `ctrl` is the platform's shortcut key, as a real keyboard sends it:
/// Ctrl, or Cmd on macOS, where Ctrl+A in a text field moves to the line start instead.
fn key(keys: &str) -> KeyEvent {
    let mut parts: Vec<&str> = keys.split('-').collect();
    let name = parts.pop().unwrap();
    let mut mods = Mods::default();
    for part in parts {
        match part {
            "shift" => mods.shift = true,
            "alt" => mods.alt = true,
            "ctrl" => {
                mods.cmd = true;
                if cfg!(target_os = "macos") {
                    mods.logo = true;
                } else {
                    mods.ctrl = true;
                }
            }
            _ => panic!("unknown modifier {part}"),
        }
    }
    let key = match name {
        "escape" => Key::Escape,
        "enter" => Key::Enter,
        "tab" => Key::Tab,
        "backspace" => Key::Backspace,
        "delete" => Key::Delete,
        "left" => Key::Left,
        "right" => Key::Right,
        "up" => Key::Up,
        "down" => Key::Down,
        "space" => Key::Text(" ".into()),
        "f1" => Key::Function(1),
        "minus" if mods.cmd => Key::Command('-'),
        _ if mods.cmd => Key::Command(name.chars().next().unwrap()),
        _ => Key::Text(name.into()),
    };
    KeyEvent { key, mods }
}

fn shift() -> Mods {
    Mods {
        shift: true,
        ..Default::default()
    }
}

/// The points of an arrow annotation.
fn arrow_points(annotation: &Annotation) -> [ImagePoint; 3] {
    annotation
        .kind
        .arrow_points()
        .expect("expected an arrow annotation")
}

/// The measurement of the last annotation, which must be a ruler.
fn placed_ruler(h: &Harness) -> sniplet_core::Measurement {
    match h.annotations().last().map(|a| &a.kind) {
        Some(AnnotationKind::Measurement { measurement }) => *measurement,
        _ => panic!("expected a measurement"),
    }
}
