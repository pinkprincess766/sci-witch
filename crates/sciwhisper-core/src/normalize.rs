//! Transcript normalization: case, ё, punctuation, Whisper glue (2x-3).

pub fn normalize(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut prev_space = true;
    let mut i = 0;
    while i < chars.len() {
        let mut ch = chars[i];
        if ch == 'ё' || ch == 'Ё' {
            ch = 'е';
        }
        if ch.is_alphanumeric() {
            for c in ch.to_lowercase() {
                out.push(c);
            }
            prev_space = false;
        } else if ch == ',' {
            if !prev_space {
                out.push(' ');
            }
            out.push(',');
            out.push(' ');
            prev_space = true;
        } else if matches!(ch, ';' | ':' | '.' | '!' | '?' | '"' | '\'' | '«' | '»') {
            if !prev_space {
                out.push(' ');
            }
            prev_space = true;
        } else if matches!(ch, '-' | '—' | '–' | '−') {
            let prev_alpha = i > 0 && chars[i - 1].is_alphabetic();
            let next_alpha = i + 1 < chars.len() && chars[i + 1].is_alphabetic();
            if prev_alpha && next_alpha {
                // «плюс-минус» → two words, not a minus operator
                if !prev_space {
                    out.push(' ');
                    prev_space = true;
                }
            } else {
                if !prev_space {
                    out.push(' ');
                }
                out.push('-');
                out.push(' ');
                prev_space = true;
            }
        } else if ch.is_whitespace() {
            if !prev_space && !out.is_empty() {
                out.push(' ');
                prev_space = true;
            }
        } else {
            if !prev_space {
                out.push(' ');
            }
            out.push(ch);
            out.push(' ');
            prev_space = true;
        }
        i += 1;
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub fn normalize_word(word: &str) -> String {
    normalize(word)
}

pub fn words(text: &str) -> Vec<String> {
    let mut tokens: Vec<String> = normalize(text)
        .split_whitespace()
        .flat_map(explode_glued)
        .map(correct_scientific_word)
        .collect();

    // Context-bound repair for a frequent Russian Whisper split. Keeping this
    // behind an already recognized chemistry term avoids rewriting ordinary text.
    for i in 0..tokens.len().saturating_sub(1) {
        if tokens[i] == "гидроксид" && matches!(tokens[i + 1].as_str(), "железо" | "лезо")
        {
            tokens[i + 1] = "железа".into();
        }
    }
    tokens
}

fn correct_scientific_word(word: String) -> String {
    match word.as_str() {
        "гидраксид" | "гидраксит" | "гидроксит" | "кидроксид" | "кидроксидж" | "гидроксидж" => {
            "гидроксид".into()
        }
        _ => word,
    }
}

/// Whisper often emits `2x-3` or `2x`. Split digit/letter boundaries.
fn explode_glued(w: &str) -> Vec<String> {
    if w.chars().all(|c| c.is_alphabetic()) || w.chars().all(|c| c.is_ascii_digit() || c == ',') {
        return vec![w.to_string()];
    }
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut kind = ' ';
    let flush = |cur: &mut String, out: &mut Vec<String>| {
        if !cur.is_empty() {
            out.push(std::mem::take(cur));
        }
    };
    for ch in w.chars() {
        let k = if ch.is_ascii_digit() || ch == ',' {
            'd'
        } else if ch.is_alphabetic() {
            'a'
        } else {
            's'
        };
        if k == 's' {
            flush(&mut cur, &mut out);
            out.push(ch.to_string());
            kind = ' ';
            continue;
        }
        if cur.is_empty() || kind == k {
            cur.push(ch);
            kind = k;
        } else {
            flush(&mut cur, &mut out);
            cur.push(ch);
            kind = k;
        }
    }
    flush(&mut cur, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_punctuation_and_yo() {
        assert_eq!(
            normalize("Дробь: числитель ещё; знаменатель."),
            "дробь числитель еще знаменатель"
        );
    }

    #[test]
    fn keeps_comma_token() {
        assert_eq!(
            normalize("корень из икс, плюс один"),
            "корень из икс , плюс один"
        );
    }

    #[test]
    fn splits_whisper_glued_math() {
        assert_eq!(words("2x-3"), ["2", "x", "-", "3"]);
        assert_eq!(words("плюс-минус"), ["плюс", "минус"]);
    }

    #[test]
    fn repairs_observed_hydroxide_whisper_variants() {
        assert_eq!(words("гидраксид железа 3"), ["гидроксид", "железа", "3"]);
        assert_eq!(words("кидроксидж лезо три"), ["гидроксид", "железа", "три"]);
    }
}
