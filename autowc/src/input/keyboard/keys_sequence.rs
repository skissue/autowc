use super::key_to_code;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeysSequenceAction {
    Text(String),
    Chord(Vec<u32>),
    Wait { duration_ms: u64 },
}

pub fn parse_keys_sequence(input: &str) -> Result<Vec<KeysSequenceAction>, String> {
    let mut actions = Vec::new();
    let mut text = String::new();
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\\' if chars.peek() == Some(&'\\') => {
                chars.next();
                text.push('\\');
            }
            '\\' if chars.peek() == Some(&'<') => {
                chars.next();
                text.push('<');
            }
            '<' => {
                flush_text(&mut actions, &mut text);
                let mut token = String::new();
                loop {
                    match chars.next() {
                        Some('>') => break,
                        Some(ch) => token.push(ch),
                        None => return Err("unterminated keys token".into()),
                    }
                }
                actions.push(parse_angle_token(&token)?);
            }
            _ => text.push(ch),
        }
    }

    flush_text(&mut actions, &mut text);
    Ok(actions)
}

fn flush_text(actions: &mut Vec<KeysSequenceAction>, text: &mut String) {
    if !text.is_empty() {
        actions.push(KeysSequenceAction::Text(std::mem::take(text)));
    }
}

fn parse_angle_token(token: &str) -> Result<KeysSequenceAction, String> {
    if let Some(duration_ms) = token.strip_prefix("w:") {
        let duration_ms = duration_ms
            .parse::<u64>()
            .map_err(|_| format!("invalid wait duration: <{token}>"))?;
        return Ok(KeysSequenceAction::Wait { duration_ms });
    }

    Ok(KeysSequenceAction::Chord(parse_chord(token)?))
}

fn parse_chord(token: &str) -> Result<Vec<u32>, String> {
    if token.is_empty() {
        return Err("empty keys token".into());
    }

    let parts = chord_parts(token)?;
    let mut parts = parts.iter().peekable();
    let mut codes = Vec::new();
    while let Some(part) = parts.next() {
        if part.is_empty() {
            return Err(format!("invalid keys token: <{token}>"));
        }

        let code = if parts.peek().is_some() {
            parse_modifier(part).ok_or_else(|| format!("unknown modifier: {part}"))?
        } else {
            parse_key(part).ok_or_else(|| format!("unknown key: {part}"))?
        };
        codes.push(code);
    }

    Ok(codes)
}

fn chord_parts(token: &str) -> Result<Vec<&str>, String> {
    if token == "-" {
        return Err(format!("invalid keys token: <{token}>"));
    }

    if let Some(prefix) = token.strip_suffix("--") {
        if prefix.is_empty() {
            return Err(format!("invalid keys token: <{token}>"));
        }
        let mut parts = prefix.split('-').collect::<Vec<_>>();
        if parts.iter().any(|part| part.is_empty()) {
            return Err(format!("invalid keys token: <{token}>"));
        }
        parts.push("-");
        return Ok(parts);
    }

    let parts = token.split('-').collect::<Vec<_>>();
    if parts.iter().any(|part| part.is_empty()) {
        return Err(format!("invalid keys token: <{token}>"));
    }
    Ok(parts)
}

fn parse_modifier(modifier: &str) -> Option<u32> {
    key_to_code(match modifier {
        "ControlLeft" | "ControlRight" | "ShiftLeft" | "ShiftRight" | "AltLeft" | "AltRight"
        | "MetaLeft" | "MetaRight" => modifier,
        "C" | "Ctrl" | "Control" => "ControlLeft",
        "S" | "Shift" => "ShiftLeft",
        "A" | "M" | "Alt" => "AltLeft",
        "s" => "MetaLeft",
        "Meta" | "Super" | "Win" => "MetaLeft",
        _ => return None,
    })
}

fn parse_key(key: &str) -> Option<u32> {
    if let Some(code) = key_to_code(key) {
        return Some(code);
    }

    let alias = key.to_ascii_lowercase();
    let code_name = match alias.as_str() {
        "ret" | "return" | "enter" => "Enter",
        "tab" => "Tab",
        "spc" | "space" => "Space",
        "esc" | "escape" => "Escape",
        "bs" | "backspace" => "Backspace",
        "del" | "delete" => "Delete",
        "up" => "ArrowUp",
        "down" => "ArrowDown",
        "left" => "ArrowLeft",
        "right" => "ArrowRight",
        _ => return parse_single_char_key(key),
    };

    key_to_code(code_name)
}

fn parse_single_char_key(key: &str) -> Option<u32> {
    let mut chars = key.chars();
    let ch = chars.next()?;
    if chars.next().is_some() {
        return None;
    }

    let code_name = match ch {
        'a'..='z' => format!("Key{}", ch.to_ascii_uppercase()),
        'A'..='Z' => format!("Key{ch}"),
        '0'..='9' => format!("Digit{ch}"),
        '-' => "Minus".to_string(),
        '=' => "Equal".to_string(),
        '[' => "BracketLeft".to_string(),
        ']' => "BracketRight".to_string(),
        '\\' => "Backslash".to_string(),
        ';' => "Semicolon".to_string(),
        '\'' => "Quote".to_string(),
        '`' => "Backquote".to_string(),
        ',' => "Comma".to_string(),
        '.' => "Period".to_string(),
        '/' => "Slash".to_string(),
        _ => return None,
    };

    key_to_code(&code_name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::keyboard::key_to_code;

    #[test]
    fn parses_literal_text() {
        assert_eq!(
            parse_keys_sequence("hello world").unwrap(),
            vec![KeysSequenceAction::Text("hello world".to_string())]
        );
    }

    #[test]
    fn parses_chords_special_keys_and_text_without_delimiters() {
        assert_eq!(
            parse_keys_sequence("<C-t>example.org<RET>").unwrap(),
            vec![
                KeysSequenceAction::Chord(vec![
                    key_to_code("ControlLeft").unwrap(),
                    key_to_code("KeyT").unwrap(),
                ]),
                KeysSequenceAction::Text("example.org".to_string()),
                KeysSequenceAction::Chord(vec![key_to_code("Enter").unwrap()]),
            ]
        );
    }

    #[test]
    fn parses_wait_directive() {
        assert_eq!(
            parse_keys_sequence("<w:500>").unwrap(),
            vec![KeysSequenceAction::Wait { duration_ms: 500 }]
        );
    }

    #[test]
    fn supports_direct_key_code_names() {
        assert_eq!(
            parse_keys_sequence("<ControlRight-KeyL>").unwrap(),
            vec![KeysSequenceAction::Chord(vec![
                key_to_code("ControlRight").unwrap(),
                key_to_code("KeyL").unwrap(),
            ])]
        );
    }

    #[test]
    fn parses_special_key_aliases() {
        assert_eq!(
            parse_keys_sequence("<ESC><TAB><SPC><BS><DEL><UP><DOWN><LEFT><RIGHT>").unwrap(),
            vec![
                KeysSequenceAction::Chord(vec![key_to_code("Escape").unwrap()]),
                KeysSequenceAction::Chord(vec![key_to_code("Tab").unwrap()]),
                KeysSequenceAction::Chord(vec![key_to_code("Space").unwrap()]),
                KeysSequenceAction::Chord(vec![key_to_code("Backspace").unwrap()]),
                KeysSequenceAction::Chord(vec![key_to_code("Delete").unwrap()]),
                KeysSequenceAction::Chord(vec![key_to_code("ArrowUp").unwrap()]),
                KeysSequenceAction::Chord(vec![key_to_code("ArrowDown").unwrap()]),
                KeysSequenceAction::Chord(vec![key_to_code("ArrowLeft").unwrap()]),
                KeysSequenceAction::Chord(vec![key_to_code("ArrowRight").unwrap()]),
            ]
        );
    }

    #[test]
    fn parses_special_key_aliases_case_insensitively() {
        assert_eq!(
            parse_keys_sequence("<ret><Escape><left>").unwrap(),
            vec![
                KeysSequenceAction::Chord(vec![key_to_code("Enter").unwrap()]),
                KeysSequenceAction::Chord(vec![key_to_code("Escape").unwrap()]),
                KeysSequenceAction::Chord(vec![key_to_code("ArrowLeft").unwrap()]),
            ]
        );
    }

    #[test]
    fn supports_emacs_special_key_aliases() {
        assert_eq!(
            parse_keys_sequence("<return><escape><backspace><space>").unwrap(),
            vec![
                KeysSequenceAction::Chord(vec![key_to_code("Enter").unwrap()]),
                KeysSequenceAction::Chord(vec![key_to_code("Escape").unwrap()]),
                KeysSequenceAction::Chord(vec![key_to_code("Backspace").unwrap()]),
                KeysSequenceAction::Chord(vec![key_to_code("Space").unwrap()]),
            ]
        );
    }

    #[test]
    fn parses_function_and_media_keys() {
        assert_eq!(
            parse_keys_sequence("<F1><F12><F24><MediaPlayPause><MediaTrackNext><AudioVolumeMute>")
                .unwrap(),
            vec![
                KeysSequenceAction::Chord(vec![key_to_code("F1").unwrap()]),
                KeysSequenceAction::Chord(vec![key_to_code("F12").unwrap()]),
                KeysSequenceAction::Chord(vec![key_to_code("F24").unwrap()]),
                KeysSequenceAction::Chord(vec![key_to_code("MediaPlayPause").unwrap()]),
                KeysSequenceAction::Chord(vec![key_to_code("MediaTrackNext").unwrap()]),
                KeysSequenceAction::Chord(vec![key_to_code("AudioVolumeMute").unwrap()]),
            ]
        );
    }

    #[test]
    fn parses_emacs_minus_key_chords() {
        assert_eq!(
            parse_keys_sequence("<C--><C-M-->").unwrap(),
            vec![
                KeysSequenceAction::Chord(vec![
                    key_to_code("ControlLeft").unwrap(),
                    key_to_code("Minus").unwrap(),
                ]),
                KeysSequenceAction::Chord(vec![
                    key_to_code("ControlLeft").unwrap(),
                    key_to_code("AltLeft").unwrap(),
                    key_to_code("Minus").unwrap(),
                ]),
            ]
        );
    }

    #[test]
    fn parses_emacs_modifier_case_conventions() {
        assert_eq!(
            parse_keys_sequence("<M-x><S-x>").unwrap(),
            vec![
                KeysSequenceAction::Chord(vec![
                    key_to_code("AltLeft").unwrap(),
                    key_to_code("KeyX").unwrap(),
                ]),
                KeysSequenceAction::Chord(vec![
                    key_to_code("ShiftLeft").unwrap(),
                    key_to_code("KeyX").unwrap(),
                ]),
            ]
        );
    }

    #[test]
    fn parses_lowercase_s_as_super_only_in_modifier_position() {
        assert_eq!(
            parse_keys_sequence("<C-s><C-s-a><C-s-s>").unwrap(),
            vec![
                KeysSequenceAction::Chord(vec![
                    key_to_code("ControlLeft").unwrap(),
                    key_to_code("KeyS").unwrap(),
                ]),
                KeysSequenceAction::Chord(vec![
                    key_to_code("ControlLeft").unwrap(),
                    key_to_code("MetaLeft").unwrap(),
                    key_to_code("KeyA").unwrap(),
                ]),
                KeysSequenceAction::Chord(vec![
                    key_to_code("ControlLeft").unwrap(),
                    key_to_code("MetaLeft").unwrap(),
                    key_to_code("KeyS").unwrap(),
                ]),
            ]
        );
    }

    #[test]
    fn rejects_bare_minus_angle_token() {
        assert_eq!(
            parse_keys_sequence("<->").unwrap_err(),
            "invalid keys token: <->"
        );
    }

    #[test]
    fn escapes_literal_less_than() {
        assert_eq!(
            parse_keys_sequence(r"\<C-t>").unwrap(),
            vec![KeysSequenceAction::Text("<C-t>".to_string())]
        );
    }

    #[test]
    fn escapes_literal_backslash_before_chord() {
        assert_eq!(
            parse_keys_sequence(r"\\<C-t>").unwrap(),
            vec![
                KeysSequenceAction::Text(r"\".to_string()),
                KeysSequenceAction::Chord(vec![
                    key_to_code("ControlLeft").unwrap(),
                    key_to_code("KeyT").unwrap(),
                ]),
            ]
        );
    }

    #[test]
    fn keeps_backslashes_literal_unless_escaping_less_than() {
        assert_eq!(
            parse_keys_sequence(r"a\b").unwrap(),
            vec![KeysSequenceAction::Text(r"a\b".to_string())]
        );
    }

    #[test]
    fn rejects_unclosed_angle_token() {
        assert_eq!(
            parse_keys_sequence("hello <RET").unwrap_err(),
            "unterminated keys token"
        );
    }

    #[test]
    fn rejects_invalid_wait_directive() {
        assert_eq!(
            parse_keys_sequence("<w:nope>").unwrap_err(),
            "invalid wait duration: <w:nope>"
        );
    }
}
