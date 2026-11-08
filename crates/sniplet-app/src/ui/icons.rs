//! Lucide icons (ISC, see `assets/icons/LICENSE-LUCIDE`), vendored as SVG files in
//! `assets/icons/lucide`. Each file is a 24 × 24 outline drawn in `currentColor`, so an icon
//! takes the color of the element that draws it. To add one, copy its SVG from Lucide into that
//! folder and list it here.
use rune::Icon;

macro_rules! lucide {
    ($($(#[$meta:meta])* $name:ident = $file:literal,)*) => {
        $(
            $(#[$meta])*
            pub static $name: Icon = Icon {
                name: concat!("lucide-", $file),
                view: 24.0,
                body: include_str!(concat!("../../../../assets/icons/lucide/", $file, ".svg")),
            };
        )*
    };
}

lucide! {
    APP_WINDOW = "app-window",
    ARROW_UP_RIGHT = "arrow-up-right",
    CHECK = "check",
    CHEVRONS_DOWN = "chevrons-down",
    CHEVRONS_UP = "chevrons-up",
    CIRCLE_ALERT = "circle-alert",
    CIRCLE_CHECK = "circle-check",
    CIRCLE = "circle",
    CLIPBOARD_PASTE = "clipboard-paste",
    // The Linux tray menu uses these two.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    CLIPBOARD = "clipboard",
    CLOUD_UPLOAD = "cloud-upload",
    COPY = "copy",
    CROP = "crop",
    DOWNLOAD = "download",
    DROPLETS = "droplets",
    ERASER = "eraser",
    EYE_OFF = "eye-off",
    FILE = "file",
    FOCUS = "focus",
    FOLDER_OPEN = "folder-open",
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    GITHUB = "github",
    GLOBE = "globe",
    GRID_2X2 = "grid-2x2",
    HIGHLIGHTER = "highlighter",
    IMAGE_PLUS = "image-plus",
    KEYBOARD = "keyboard",
    LAYERS = "layers",
    LIST_ORDERED = "list-ordered",
    MINUS = "minus",
    MONITOR = "monitor",
    MOUSE_POINTER_2 = "mouse-pointer-2",
    MOUSE = "mouse",
    PAINT_BUCKET = "paint-bucket",
    PENCIL = "pencil",
    PIN = "pin",
    PLUS = "plus",
    POWER = "power",
    QR_CODE = "qr-code",
    REFRESH_CW = "refresh-cw",
    REPEAT = "repeat",
    ROTATE_CCW = "rotate-ccw",
    RULER = "ruler",
    SAVE = "save",
    SCAN_TEXT = "scan-text",
    SCAN = "scan",
    SCISSORS = "scissors",
    SETTINGS = "settings",
    SLASH = "slash",
    SQUARE_DASHED_BOTTOM = "square-dashed-bottom",
    SQUARE_DASHED_TEXT = "square-dashed-text",
    SQUARE_MOUSE_POINTER = "square-mouse-pointer",
    SQUARE = "square",
    TIMER = "timer",
    TYPE = "type",
    WAND_SPARKLES = "wand-sparkles",
    X = "x",
    ZOOM_IN = "zoom-in",
}
