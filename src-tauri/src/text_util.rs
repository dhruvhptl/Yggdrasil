// src-tauri/src/text_util.rs
//
// Small char-safe string helpers. Byte slicing arbitrary text (`&s[..n]`)
// panics when `n` lands inside a multi-byte UTF-8 char (Σ, µ, —, …); these
// helpers truncate on character boundaries instead.

/// Return at most `max_chars` characters from the start of `s`, collected into a
/// new String. Never panics on multi-byte input.
pub(crate) fn truncate_chars(s: &str, max_chars: usize) -> String {
    s.chars().take(max_chars).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncates_on_char_boundary_not_byte_boundary() {
        // "Σµ—x" is 4 chars but 8 bytes; byte-slicing [..2] would panic mid-char.
        let s = "Σµ—x";
        assert_eq!(truncate_chars(s, 2), "Σµ");
    }

    #[test]
    fn returns_whole_string_when_max_exceeds_length() {
        assert_eq!(truncate_chars("abc", 10), "abc");
    }

    #[test]
    fn zero_max_returns_empty() {
        assert_eq!(truncate_chars("Σµ", 0), "");
    }
}
