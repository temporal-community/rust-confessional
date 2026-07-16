//! Stage-input guards for raw-display mode.
//!
//! With `SHOW_RAW_CONFESSIONS=true`, audience text is projected on stage. Two
//! small guards run first:
//! - [`sanitize_for_stage`] removes control characters (which can break
//!   rendering) and collapses whitespace, while keeping letters, numbers,
//!   punctuation, and emoji;
//! - [`mask_words`] blanks any operator-supplied words (from `MASK_WORDS`).
//!
//! No word list is bundled in this repository by design. This is a floor, not
//! moderation: a wordlist cannot catch creative spellings, context, or personal
//! information. Keep the dashboard Hold/Reset kill switch handy and rehearse
//! with real inputs.

/// Replace control characters with spaces (they can break terminal and DOM
/// rendering) and collapse runs of whitespace; keep letters, numbers,
/// punctuation, and emoji so audience text reads naturally on stage. Zero-width
/// joiners and variation selectors are not control characters, so multi-codepoint
/// emoji (for example family sequences) survive intact.
pub fn sanitize_for_stage(text: &str) -> String {
    let filtered: String = text
        .chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect();

    filtered.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Blank whole words that appear in `words` (already lowercased). Matching is
/// whole-word and case-insensitive, so `Scunthorpe` and `assassin` are left
/// untouched. Run after [`sanitize_for_stage`] so the asterisks are not stripped.
pub fn mask_words(text: &str, words: &[String]) -> String {
    if words.is_empty() {
        return text.to_owned();
    }

    let mut output = String::with_capacity(text.len());
    let mut word = String::new();

    for character in text.chars() {
        if character.is_alphanumeric() {
            word.push(character);
        } else {
            flush_word(&mut output, &mut word, words);
            output.push(character);
        }
    }
    flush_word(&mut output, &mut word, words);
    output
}

fn flush_word(output: &mut String, word: &mut String, words: &[String]) {
    if word.is_empty() {
        return;
    }
    let lowered = word.to_lowercase();
    if words.contains(&lowered) {
        output.push_str(&"*".repeat(word.chars().count()));
    } else {
        output.push_str(word);
    }
    word.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_keeps_unicode_letters_punctuation_and_emoji() {
        assert_eq!(sanitize_for_stage("café ☕ works"), "café ☕ works");
        assert_eq!(sanitize_for_stage("naïve façade"), "naïve façade");
        assert_eq!(sanitize_for_stage("ship it 🚀🔥"), "ship it 🚀🔥");
    }

    #[test]
    fn sanitize_preserves_multi_codepoint_emoji() {
        // A family emoji is joined by zero-width joiners, which are not control
        // characters and must survive sanitization.
        let family = "👨‍👩‍👧";
        assert_eq!(sanitize_for_stage(family), family);
    }

    #[test]
    fn sanitize_strips_control_characters_and_collapses_whitespace() {
        assert_eq!(sanitize_for_stage("a\u{0007}b\t c\n\nd"), "a b c d");
        assert_eq!(sanitize_for_stage("  spaced   out  "), "spaced out");
    }

    #[test]
    fn sanitize_keeps_basic_punctuation() {
        assert_eq!(
            sanitize_for_stage("I clone everything until it compiles."),
            "I clone everything until it compiles."
        );
    }

    #[test]
    fn mask_blanks_listed_words_whole_and_case_insensitively() {
        let words = vec!["badword".to_owned()];
        assert_eq!(mask_words("you badword", &words), "you *******");
        assert_eq!(mask_words("BadWord!", &words), "*******!");
    }

    #[test]
    fn mask_leaves_substrings_and_unlisted_words_untouched() {
        let words = vec!["cram".to_owned()];
        assert_eq!(mask_words("scramble the cram", &words), "scramble the ****");
        assert_eq!(mask_words("perfectly fine", &words), "perfectly fine");
    }

    #[test]
    fn mask_is_a_noop_with_no_words() {
        assert_eq!(mask_words("anything at all", &[]), "anything at all");
    }
}
