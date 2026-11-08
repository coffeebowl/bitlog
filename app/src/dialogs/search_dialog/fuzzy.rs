//! Finding titles by a few of their letters, like "neta" for "New Task".
//! The letters typed have to appear in the title in their order; they score
//! more at the start of a word and next to each other, so that the titles
//! meant come first.

/// What each letter typed scores where it is found.
const LETTER: u32 = 1;
/// Extra for a letter that starts a word of the title.
const WORD_START: u32 = 2;
/// Extra for a letter right after the one found before it.
const NEXT: u32 = 2;
/// Words scoring less per letter are found only by chance. Any part of a
/// title two letters long or more scores enough.
const MIN_PER_LETTER: u32 = 2;

/// How well `title` matches all of `words`, which are lowercase, or `None`
/// if one of them does not match. Without words, any title matches with 0.
pub fn score(title: &str, words: &[&str]) -> Option<u32> {
    let title: Vec<char> = title.to_lowercase().chars().collect();
    words.iter().map(|word| word_score(&title, word)).sum()
}

/// The best score of the letters of `word` found in `title` in their order.
fn word_score(title: &[char], word: &str) -> Option<u32> {
    // For each place in the title, the best score of the letters so far
    // with the last one found there.
    let mut scores: Vec<Option<u32>> = Vec::new();
    for (index, letter) in word.chars().enumerate() {
        let mut next = vec![None; title.len()];
        // The best score of the letters before, found before the place.
        let mut before: Option<u32> = None;
        for (place, &char) in title.iter().enumerate() {
            if char == letter {
                let starts_word = place == 0 || !title[place - 1].is_alphanumeric();
                let gain = LETTER + if starts_word { WORD_START } else { 0 };
                next[place] = if index == 0 {
                    Some(gain)
                } else {
                    let apart = before.map(|score| score + gain);
                    let after = place.checked_sub(1).and_then(|previous| scores[previous]);
                    apart.max(after.map(|score| score + gain + NEXT))
                };
            }
            if index > 0 {
                before = before.max(scores[place]);
            }
        }
        scores = next;
    }
    let letters = u32::try_from(word.chars().count()).expect("words typed are short");
    scores
        .into_iter()
        .max()
        .flatten()
        .filter(|&score| score >= MIN_PER_LETTER * letters)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_in_order_match() {
        for (title, word) in [
            ("New Task", "newta"),
            ("New Task", "neta"),
            ("New Task", "tsk"),
            ("New Block for Webshop", "nbw"),
            ("checkout-flow", "chfl"),
            ("Webshop", "shop"),
            ("Webshop", "w"),
        ] {
            assert!(score(title, &[word]).is_some(), "{word} in {title}");
        }
    }

    #[test]
    fn chance_and_wrong_order_do_not_match() {
        for (title, word) in [
            ("New Task", "tn"),
            ("Yesterday", "eta"),
            ("Webshop", "s"),
            ("Webshop", "x"),
        ] {
            assert_eq!(score(title, &[word]), None, "{word} in {title}");
        }
    }

    #[test]
    fn closer_matches_score_higher() {
        assert!(score("New Task", &["neta"]) > score("New Contact", &["neta"]));
        assert!(score("Webshop", &["webs"]) > score("Webshop", &["wbs"]));
    }

    #[test]
    fn all_words_have_to_match() {
        let title = "New Block for Webshop";
        assert!(score(title, &["block", "web"]).is_some());
        assert_eq!(score(title, &["block", "task"]), None);
        assert_eq!(score(title, &[]), Some(0));
    }
}
