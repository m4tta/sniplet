//! Sniplet shortcuts that macOS also uses. macOS lets an app register such keys, and then its
//! own shortcut runs too, so Sniplet finds them and tells the user.
use global_hotkey::hotkey::{Code, Modifiers};
use sniplet_platform::SystemShortcut;

/// macOS virtual key codes on an ANSI keyboard, for the keys a shortcut can end in.
const KEY_CODES: [(Code, u16); 73] = [
    (Code::KeyA, 0),
    (Code::KeyS, 1),
    (Code::KeyD, 2),
    (Code::KeyF, 3),
    (Code::KeyH, 4),
    (Code::KeyG, 5),
    (Code::KeyZ, 6),
    (Code::KeyX, 7),
    (Code::KeyC, 8),
    (Code::KeyV, 9),
    (Code::KeyB, 11),
    (Code::KeyQ, 12),
    (Code::KeyW, 13),
    (Code::KeyE, 14),
    (Code::KeyR, 15),
    (Code::KeyY, 16),
    (Code::KeyT, 17),
    (Code::Digit1, 18),
    (Code::Digit2, 19),
    (Code::Digit3, 20),
    (Code::Digit4, 21),
    (Code::Digit6, 22),
    (Code::Digit5, 23),
    (Code::Equal, 24),
    (Code::Digit9, 25),
    (Code::Digit7, 26),
    (Code::Minus, 27),
    (Code::Digit8, 28),
    (Code::Digit0, 29),
    (Code::BracketRight, 30),
    (Code::KeyO, 31),
    (Code::KeyU, 32),
    (Code::BracketLeft, 33),
    (Code::KeyI, 34),
    (Code::KeyP, 35),
    (Code::KeyL, 37),
    (Code::KeyJ, 38),
    (Code::Quote, 39),
    (Code::KeyK, 40),
    (Code::Semicolon, 41),
    (Code::Backslash, 42),
    (Code::Comma, 43),
    (Code::Slash, 44),
    (Code::KeyN, 45),
    (Code::KeyM, 46),
    (Code::Period, 47),
    (Code::Enter, 36),
    (Code::Tab, 48),
    (Code::Space, 49),
    (Code::Backquote, 50),
    (Code::Backspace, 51),
    (Code::Escape, 53),
    (Code::F5, 96),
    (Code::F6, 97),
    (Code::F7, 98),
    (Code::F3, 99),
    (Code::F8, 100),
    (Code::F9, 101),
    (Code::F11, 103),
    (Code::F10, 109),
    (Code::F12, 111),
    (Code::F4, 118),
    (Code::F2, 120),
    (Code::F1, 122),
    (Code::Home, 115),
    (Code::PageUp, 116),
    (Code::Delete, 117),
    (Code::End, 119),
    (Code::PageDown, 121),
    (Code::ArrowLeft, 123),
    (Code::ArrowRight, 124),
    (Code::ArrowDown, 125),
    (Code::ArrowUp, 126),
];

/// A saved shortcut in the form that macOS lists its own. None when it is off, unreadable, or
/// ends in a key outside the table.
pub(crate) fn system_form(text: &str) -> Option<SystemShortcut> {
    let hotkey = crate::shell::parse_hotkey(text).ok()?;
    let key_code = KEY_CODES.iter().find(|(code, _)| *code == hotkey.key)?.1;
    let modifiers = [
        (Modifiers::SUPER | Modifiers::META, SystemShortcut::COMMAND),
        (Modifiers::SHIFT, SystemShortcut::SHIFT),
        (Modifiers::ALT, SystemShortcut::OPTION),
        (Modifiers::CONTROL, SystemShortcut::CONTROL),
    ]
    .into_iter()
    .filter(|(held, _)| hotkey.mods.intersects(*held))
    .fold(0, |bits, (_, bit)| bits | bit);
    Some(SystemShortcut {
        key_code,
        modifiers,
    })
}

/// The indexes in `slots` of the shortcuts that `system` also has.
pub(crate) fn conflicts(slots: &[&str], system: &[SystemShortcut]) -> Vec<usize> {
    (0..slots.len())
        .filter(|&index| system_form(slots[index]).is_some_and(|form| system.contains(&form)))
        .collect()
}

/// New keys for each shortcut in conflict: the same key with modifiers that neither `system`
/// nor another Sniplet shortcut uses. A shortcut with no free keys is left out.
pub(crate) fn replacements(slots: &[&str], system: &[SystemShortcut]) -> Vec<(usize, String)> {
    let mut taken: Vec<_> = slots.iter().filter_map(|text| system_form(text)).collect();
    taken.extend_from_slice(system);
    let mut moves = Vec::new();
    for index in conflicts(slots, system) {
        let key = slots[index].rsplit('+').next().unwrap_or_default();
        let free = ["Control+Shift", "Control+Alt", "Control+Alt+Shift"]
            .into_iter()
            .map(|modifiers| format!("{modifiers}+{key}"))
            .find(|text| system_form(text).is_some_and(|form| !taken.contains(&form)));
        if let Some(text) = free {
            taken.extend(system_form(&text));
            moves.push((index, text));
        }
    }
    moves
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conflicts_move_to_keys_that_nothing_else_uses() {
        let slots = [
            "CommandOrControl+Shift+2",
            "CommandOrControl+Shift+3",
            "CommandOrControl+Shift+4",
            "Control+Alt+4",
            "",
        ];
        // The system has two of Sniplet's shortcuts and the first replacement for one of them.
        let system = [slots[1], slots[2], "Control+Shift+4"].map(|text| system_form(text).unwrap());
        assert_eq!(conflicts(&slots, &system), [1, 2]);
        // Mission Control's keys.
        assert!(system_form("Control+Left").is_some());

        let moves = replacements(&slots, &system);
        assert_eq!(
            moves.iter().map(|(index, _)| *index).collect::<Vec<_>>(),
            [1, 2]
        );
        let mut moved = slots.map(str::to_owned);
        for (index, text) in moves {
            moved[index] = text;
        }
        let moved = moved.each_ref().map(String::as_str);
        assert!(conflicts(&moved, &system).is_empty());
        for index in 0..4 {
            assert_eq!(crate::shell::conflicting_hotkey(&moved, index), Ok(None));
        }
        assert_eq!(moved[2], "Control+Alt+Shift+4");
    }
}
