use rune::{El, Icon, TextStyle, Tx, icon, row, text};

use super::Palette;

/// Body text: 14 pt, as the toolbar and panels use.
pub const BODY: TextStyle = TextStyle::new(14.0, 400.0);
/// Small text: 12 pt, for hints and values.
pub const SMALL: TextStyle = TextStyle::new(12.0, 400.0);
pub const SEMIBOLD: TextStyle = TextStyle::new(14.0, 600.0);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Variant {
    /// No fill until hovered; the default for toolbar and dock buttons.
    Ghost,
    /// The neutral filled button.
    Primary,
    /// Filled with Sniplet's accent blue, for the main action.
    Accent,
}

/// Sniplet's button: an icon, a label or both, in one of three looks. It builds to an `El`
/// with a key, a button role and a tooltip, so tests and screen readers find it by key or name.
pub struct Button {
    key: String,
    icon: Option<&'static Icon>,
    label: Option<String>,
    help: Option<String>,
    variant: Variant,
    selected: bool,
    disabled: bool,
    compact: bool,
    size: Option<(f32, f32)>,
}

impl Button {
    pub fn new(key: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            icon: None,
            label: None,
            help: None,
            variant: Variant::Ghost,
            selected: false,
            disabled: false,
            compact: false,
            size: None,
        }
    }

    pub fn icon(mut self, icon: &'static Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    /// The tooltip, which also names an icon-only button for screen readers.
    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn primary(mut self) -> Self {
        self.variant = Variant::Primary;
        self
    }

    pub fn accent(mut self) -> Self {
        self.variant = Variant::Accent;
        self
    }

    /// A ghost button that shows it is the current choice.
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// A selected ghost button filled with the accent, for segmented choices.
    pub fn chosen(mut self, chosen: bool) -> Self {
        if chosen {
            self.variant = Variant::Accent;
        }
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Less padding, for buttons inside a segment or a dense row.
    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    pub fn size(mut self, width: f32, height: f32) -> Self {
        self.size = Some((width, height));
        self
    }

    pub fn build<S: 'static>(
        self,
        p: &Palette,
        on_press: impl Fn(&mut S, &mut Tx) + 'static,
    ) -> El {
        let (fill, ink) = match self.variant {
            Variant::Ghost if self.selected => (p.hover, p.foreground),
            Variant::Ghost => (rune::Color::TRANSPARENT, p.foreground),
            Variant::Primary => (p.primary, p.primary_foreground),
            Variant::Accent => (p.accent, rune::Color::WHITE),
        };
        let hover = match self.variant {
            Variant::Ghost => p.hover,
            _ => fill.mix(p.background, 0.15),
        };
        let height = if self.compact { 28.0 } else { 32.0 };
        let mut button = row()
            .key(self.key)
            .role("button")
            .h(height)
            .px(if self.label.is_some() {
                if self.compact { 8.0 } else { 12.0 }
            } else {
                6.0
            })
            .gap(8.0)
            .center()
            .radius(6.0)
            .fill(fill);
        if let Some((width, height)) = self.size {
            button = button.size(width, height).px(0.0);
        }
        if let Some(glyph) = self.icon {
            button = button.child(icon(glyph, 19.0).color(ink));
        }
        if let Some(label) = &self.label {
            button = button.child(text(label.clone()).font(BODY).color(ink).single_line());
        }
        match (&self.help, &self.label) {
            (Some(help), _) => button = button.help(help.clone()).label(help.clone()),
            (None, Some(label)) => button = button.label(label.clone()),
            (None, None) => {}
        }
        if self.disabled {
            return button.opacity(0.4);
        }
        button
            .hover(hover)
            .press(hover.mix(p.foreground, 0.08))
            .cursor(rune::Cursor::Pointer)
            .on_press_tx(on_press)
    }
}

/// A vertical hairline between groups of controls.
pub fn divider(p: &Palette, height: f32) -> El {
    rune::el().w(1.0).h(height).fill(p.border)
}
