//! Escape untrusted display text. PTY bytes go through the terminal parser instead.

pub(crate) fn single_line(text: &str) -> String {
    sanitize(text, false)
}

pub(crate) fn multiline(text: &str) -> String {
    sanitize(text, true)
}

fn sanitize(text: &str, multiline: bool) -> String {
    text.chars()
        .map(|c| {
            if (c.is_control() && !(multiline && matches!(c, '\n' | '\t')))
                || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            {
                '�'
            } else {
                c
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes_controls_and_direction_overrides_but_preserves_unicode() {
        let text = "é\u{1b}[2J\r\n\t\u{202e}name\u{2069}";
        assert_eq!(single_line(text), "é�[2J����name�");
        assert_eq!(multiline(text), "é�[2J�\n\t�name�");
    }
}
