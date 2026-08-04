use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

const BRACKETED_PASTE_START: &[u8] = b"\x1b[200~";
const BRACKETED_PASTE_END: &[u8] = b"\x1b[201~";

pub fn key_event_to_bytes(event: KeyEvent) -> Option<Vec<u8>> {
    let modifiers = event.modifiers;
    match event.code {
        KeyCode::Char(ch) if modifiers.contains(KeyModifiers::CONTROL) => {
            ctrl_char(ch).map(|byte| vec![byte])
        }
        KeyCode::Char(ch) if modifiers.contains(KeyModifiers::ALT) => {
            let mut bytes = vec![0x1b];
            let mut encoded = [0; 4];
            bytes.extend_from_slice(ch.encode_utf8(&mut encoded).as_bytes());
            Some(bytes)
        }
        KeyCode::Char(ch) => {
            let mut encoded = [0; 4];
            Some(ch.encode_utf8(&mut encoded).as_bytes().to_vec())
        }
        KeyCode::Enter => Some(vec![b'\r']),
        KeyCode::Backspace => Some(vec![0x7f]),
        KeyCode::Tab => Some(vec![b'\t']),
        KeyCode::Esc => Some(vec![0x1b]),
        KeyCode::Up => Some(b"\x1b[A".to_vec()),
        KeyCode::Down => Some(b"\x1b[B".to_vec()),
        KeyCode::Right => Some(b"\x1b[C".to_vec()),
        KeyCode::Left => Some(b"\x1b[D".to_vec()),
        KeyCode::Home => Some(b"\x1b[H".to_vec()),
        KeyCode::End => Some(b"\x1b[F".to_vec()),
        KeyCode::PageUp => Some(b"\x1b[5~".to_vec()),
        KeyCode::PageDown => Some(b"\x1b[6~".to_vec()),
        KeyCode::Delete => Some(b"\x1b[3~".to_vec()),
        KeyCode::Insert => Some(b"\x1b[2~".to_vec()),
        KeyCode::F(number) => function_key(number),
        _ => None,
    }
}

pub fn paste_text_to_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let text = normalize_paste_newlines(text);
    if bracketed {
        let mut bytes = Vec::with_capacity(
            BRACKETED_PASTE_START.len() + text.len() + BRACKETED_PASTE_END.len(),
        );
        bytes.extend_from_slice(BRACKETED_PASTE_START);
        bytes.extend_from_slice(text.as_bytes());
        bytes.extend_from_slice(BRACKETED_PASTE_END);
        return bytes;
    }

    let mut bytes = Vec::with_capacity(text.len());
    for ch in text.chars() {
        if ch == '\n' {
            bytes.push(b'\r');
        } else {
            let mut encoded = [0; 4];
            bytes.extend_from_slice(ch.encode_utf8(&mut encoded).as_bytes());
        }
    }
    bytes
}

pub fn normalize_paste_newlines(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn ctrl_char(ch: char) -> Option<u8> {
    match ch {
        'a'..='z' => Some(ch as u8 - b'a' + 1),
        'A'..='Z' => Some(ch as u8 - b'A' + 1),
        '@' | ' ' => Some(0),
        '[' => Some(27),
        '\\' => Some(28),
        ']' => Some(29),
        '^' => Some(30),
        '_' => Some(31),
        '?' => Some(127),
        _ => None,
    }
}

fn function_key(number: u8) -> Option<Vec<u8>> {
    let bytes = match number {
        1 => b"\x1bOP".as_slice(),
        2 => b"\x1bOQ".as_slice(),
        3 => b"\x1bOR".as_slice(),
        4 => b"\x1bOS".as_slice(),
        5 => b"\x1b[15~".as_slice(),
        6 => b"\x1b[17~".as_slice(),
        7 => b"\x1b[18~".as_slice(),
        8 => b"\x1b[19~".as_slice(),
        9 => b"\x1b[20~".as_slice(),
        10 => b"\x1b[21~".as_slice(),
        11 => b"\x1b[23~".as_slice(),
        12 => b"\x1b[24~".as_slice(),
        _ => return None,
    };
    Some(bytes.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    #[test]
    fn translates_printable_and_enter() {
        assert_eq!(
            key_event_to_bytes(key(KeyCode::Char('a'), KeyModifiers::NONE)),
            Some(b"a".to_vec())
        );
        assert_eq!(
            key_event_to_bytes(key(KeyCode::Enter, KeyModifiers::NONE)),
            Some(vec![b'\r'])
        );
    }

    #[test]
    fn translates_control_characters() {
        assert_eq!(
            key_event_to_bytes(key(KeyCode::Char('c'), KeyModifiers::CONTROL)),
            Some(vec![3])
        );
        assert_eq!(
            key_event_to_bytes(key(KeyCode::Char('l'), KeyModifiers::CONTROL)),
            Some(vec![12])
        );
        assert_eq!(
            key_event_to_bytes(key(KeyCode::Char('r'), KeyModifiers::CONTROL)),
            Some(vec![18])
        );
    }

    #[test]
    fn translates_navigation_sequences() {
        assert_eq!(
            key_event_to_bytes(key(KeyCode::Up, KeyModifiers::NONE)),
            Some(b"\x1b[A".to_vec())
        );
        assert_eq!(
            key_event_to_bytes(key(KeyCode::PageDown, KeyModifiers::NONE)),
            Some(b"\x1b[6~".to_vec())
        );
    }

    #[test]
    fn bracketed_paste_wraps_text_and_preserves_line_feeds() {
        assert_eq!(
            paste_text_to_bytes("first\r\nsecond\rthird", true),
            b"\x1b[200~first\nsecond\nthird\x1b[201~".to_vec()
        );
    }

    #[test]
    fn plain_paste_preserves_legacy_enter_behavior() {
        assert_eq!(
            paste_text_to_bytes("first\r\nsecond", false),
            b"first\rsecond".to_vec()
        );
    }
}
