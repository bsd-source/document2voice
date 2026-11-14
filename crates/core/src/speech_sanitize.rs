//! Speech-only text sanitizer for Kokoro + espeak-ng.
//!
//! Drawn from practices in ebook2audiobook (NonTextFilter / punctuation_switch),
//! epub2tts (`prep_text`), book-to-audiobook preprocess, and abogen-style symbol
//! maps: unspeakable punctuation becomes a soft pause, glyph-name bait is
//! stripped, a short English word map expands common symbols, digit ranges
//! become “to”. Applied only on the TTS path — document extract is unchanged.

/// Prepare extracted book text for synthesis (not for display/export of source text).
pub fn sanitize_for_speech(text: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        return String::new();
    }

    // Stage A — line endings + repair common UTF-8-as-Latin-1 mojibake + ligatures
    let mut s = text.replace("\r\n", "\n").replace('\r', "\n");
    s = fix_utf8_mojibake(&s);
    s = fix_ligatures_and_quotes(&s);
    // Fold lone accented letters (espeak names them, e.g. “a circumflex”);
    // accented words such as café and hôtel are kept.
    s = fold_latin_diacritics(&s);

    // Stage A1 — book English / tech normalization (before @/$ glyph maps)
    s = strip_urls_and_emails(&s);
    // `captive.apple.com` (no `http://` / `www.`) → “captive dot apple dot com”.
    s = speak_bare_domains(&s);
    // `#BlueSky` → “hashtag Daves Grid” (bare `#7` stays “number 7” later).
    s = expand_hashtags(&s);
    // Phone numbers / ISBNs / ISO dates before the digit-range rule turns '-' into "to".
    s = expand_digit_groups(&s);
    s = expand_currency_amounts(&s);
    s = expand_times(&s);
    // Dates before fractions so 3/15/2020 is not left as slash digits
    s = expand_slash_dates(&s);
    // "May 2026" / "May 15, 2026" → casual year (twenty twenty six), not espeak
    // “two thousand twenty six”
    s = expand_month_year_dates(&s);
    s = expand_ordinals(&s);
    // `Case No 202303608`, `Ref 01/8311886/25` → digit by digit, not “two hundred
    // million …”; `80/20`, `24/7` → “80 20”, “24 7” (not “80 over 20”).
    s = expand_ids_and_ratios(&s);
    s = expand_ascii_fractions(&s);
    s = expand_units(&s);
    s = expand_abbreviations(&s);
    // `cope(d)` / `item(s)` → “coped” / “items” (not “cope, d”).
    s = join_paren_suffixes(&s);
    // `before/after`, `O2/Vodafone` → no spoken “slash” (dates, fractions, units
    // and URLs are already gone by now).
    s = expand_word_slashes(&s);
    // `copy-paste`: espeak glues it into one word and clips copy's “ee” (kˈɑːpɪpˈeɪst).
    s = split_y_hyphen_compounds(&s);
    s = expand_roman_in_context(&s);
    // "Henry VIII", "World War II", "Apollo XI" (after the Chapter/Part pass).
    s = expand_roman_after_names(&s);
    // Product codes before acronyms so `EMF-390` → “EMF three ninety”, then
    // acronym pass can letter-spell `EMF` (not leave a stranded “-390”).
    s = expand_model_numbers(&s);
    // Spoken forms: 23.5 → "23 point 5", 1990s → "nineteen nineties",
    // "in 1847" → "eighteen forty seven".
    s = expand_decimals_years_decades(&s);
    // `dayNo`, `TriggerActivated` (space lost in the PDF) → “day No”, “Trigger Activated”.
    s = split_glued_words(&s);
    // ALL-CAPS: dictionary words → words ("PRIDE", "STOP"); acronyms → spoken letters.
    s = expand_acronyms_and_dict(&s);

    // Stage B — char walk: pauses, strip never-speak, combining marks
    s = map_chars(&s);

    // Stage C — lone symbols left after amount flip (& @ % bare $ …)
    s = expand_symbol_words(&s);

    // Stage C2 — “A and B” letter lists: bare capital A is espeak schwa “uh”;
    // expand only in letter-list context so article “A well-known…” stays an article.
    s = expand_letter_list_capitals(&s);

    // Stage D — whitespace: keep blank-line paragraphs, collapse the rest
    let s = collapse_ws_keep_paragraphs(&s);
    // Stage E — `, .` / `, ,` / `word .` left where symbols were dropped → one mark.
    let s = tidy_punctuation(&s);
    // `eye-em-ess-eye -catcher`, `Post- ee-dee…` → hyphen dropped, words spaced.
    join_stray_hyphens(&s)
}

/// Undo UTF-8 bytes misread as Latin-1 (e.g. em dash → U+00E2 U+0080 U+0094).
/// espeak speaks lone â (U+00E2) as “circumflex”.
fn fix_utf8_mojibake(s: &str) -> String {
    s.replace("\u{00e2}\u{0080}\u{0094}", "\u{2014}") // â\x80\x94 → —
        .replace("\u{00e2}\u{0080}\u{0093}", "\u{2013}") // â\x80\x93 → –
        .replace("\u{00e2}\u{0080}\u{0092}", "\u{2019}") // â\x80\x92 → ’
        .replace("\u{00e2}\u{0080}\u{0098}", "\u{2018}") // â\x80\x98 → ‘
        .replace("\u{00e2}\u{0080}\u{009c}", "\u{201c}") // â\x80\x9c → “
        .replace("\u{00e2}\u{0080}\u{009d}", "\u{201d}") // â\x80\x9d → ”
        .replace("\u{00e2}\u{0080}\u{00a6}", "\u{2026}") // â\x80\xa6 → …
        .replace("\u{00e2}\u{0080}\u{00a2}", "\u{2022}") // â\x80\xa2 → •
        .replace(
            "\u{00c3}\u{00a2}\u{00c2}\u{0080}\u{00c2}\u{0094}",
            "\u{2014}",
        ) // double-encoded —
}

fn fix_ligatures_and_quotes(s: &str) -> String {
    s.replace('\u{fb00}', "ff") // ﬀ
        .replace('\u{fb01}', "fi") // ﬁ
        .replace('\u{fb02}', "fl") // ﬂ
        .replace('\u{fb03}', "ffi") // ﬃ
        .replace('\u{fb04}', "ffl") // ﬄ
        .replace(['\u{fb05}', '\u{fb06}'], "st") // ﬅ ﬆ
        .replace('\u{0153}', "oe") // œ → oe (espeak-safe)
        .replace('\u{0152}', "OE") // Œ
        // Keep æ/Æ: espeak handles them; spelling out “ae” hurts Danish/French.
        .replace(
            [
                '\u{201c}', '\u{201d}', '\u{201e}', '\u{201f}', '\u{00ab}', '\u{00bb}', '\u{2033}',
            ],
            "\"",
        )
        .replace(
            [
                '\u{2018}', '\u{2019}', '\u{201a}', '\u{201b}', '\u{2032}', '\u{2039}', '\u{203a}',
            ],
            "'",
        )
}

/// Fold diacritics where espeak would otherwise name them.
///
/// espeak-ng reads accented **words** correctly — `résumé` /ˈɹɛzuːmeɪ/, `café`,
/// `naïve`, `hôtel`, `façade`, `jalapeño` — and folding them changes the word
/// (`résumé` → the verb `resume`). It only names **lone** accented letters
/// (“e acute”, “a circumflex”). So: accented vowels, `ç` and `ñ`
/// stay when they touch another letter; lone ones, and consonant marks English
/// espeak mangles (`ř č š ž ł` …, e.g. Dvořák), are folded to ASCII.
fn fold_latin_diacritics(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut buf = [0u8; 4];
    for (i, &c) in chars.iter().enumerate() {
        if c.is_ascii() {
            out.push(c);
            continue;
        }
        let base = fold_latin_all(c.encode_utf8(&mut buf));
        if base.chars().eq(std::iter::once(c)) {
            out.push(c);
            continue;
        }
        let in_word = (i > 0 && chars[i - 1].is_alphabetic())
            || chars.get(i + 1).is_some_and(|n| n.is_alphabetic());
        let keep_in_word = matches!(c, 'ç' | 'Ç' | 'ñ' | 'Ñ')
            || (base.len() == 1 && "aeiouyAEIOUY".contains(base.as_str()));
        if in_word && keep_in_word {
            out.push(c);
        } else {
            out.push_str(&base);
        }
    }
    out
}

/// Map common Latin letters with diacritics to ASCII bases (every occurrence).
fn fold_latin_all(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            // a / A
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ă' | 'ą' | 'ǎ' | 'ȧ' | 'ạ' | 'ả' =>
            {
                out.push('a');
            }
            'À' | 'Á' | 'Â' | 'Ã' | 'Ä' | 'Å' | 'Ā' | 'Ă' | 'Ą' | 'Ǎ' | 'Ȧ' | 'Ạ' | 'Ả' =>
            {
                out.push('A');
            }
            // e / E
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ĕ' | 'ė' | 'ę' | 'ě' | 'ẹ' | 'ẻ' | 'ẽ' => {
                out.push('e')
            }
            'È' | 'É' | 'Ê' | 'Ë' | 'Ē' | 'Ĕ' | 'Ė' | 'Ę' | 'Ě' | 'Ẹ' | 'Ẻ' | 'Ẽ' => {
                out.push('E')
            }
            // i / I
            'ì' | 'í' | 'î' | 'ï' | 'ĩ' | 'ī' | 'ĭ' | 'į' | 'ǐ' | 'ị' | 'ỉ' => {
                out.push('i')
            }
            'Ì' | 'Í' | 'Î' | 'Ï' | 'Ĩ' | 'Ī' | 'Ĭ' | 'Į' | 'Ǐ' | 'Ị' | 'Ỉ' => {
                out.push('I')
            }
            // o / O
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' | 'ŏ' | 'ő' | 'ǒ' | 'ọ' | 'ỏ' | 'ơ' =>
            {
                out.push('o');
            }
            'Ò' | 'Ó' | 'Ô' | 'Õ' | 'Ö' | 'Ø' | 'Ō' | 'Ŏ' | 'Ő' | 'Ǒ' | 'Ọ' | 'Ỏ' | 'Ơ' =>
            {
                out.push('O');
            }
            // u / U
            'ù' | 'ú' | 'û' | 'ü' | 'ũ' | 'ū' | 'ŭ' | 'ů' | 'ű' | 'ų' | 'ǔ' | 'ụ' | 'ủ' | 'ư' =>
            {
                out.push('u');
            }
            'Ù' | 'Ú' | 'Û' | 'Ü' | 'Ũ' | 'Ū' | 'Ŭ' | 'Ů' | 'Ű' | 'Ų' | 'Ǔ' | 'Ụ' | 'Ủ' | 'Ư' =>
            {
                out.push('U');
            }
            // y / Y
            'ý' | 'ÿ' | 'ŷ' | 'ȳ' | 'ẏ' | 'ỵ' | 'ỷ' | 'ỹ' => out.push('y'),
            'Ý' | 'Ÿ' | 'Ŷ' | 'Ȳ' | 'Ẏ' | 'Ỵ' | 'Ỷ' | 'Ỹ' => out.push('Y'),
            // other common book letters
            'ç' | 'ć' | 'ĉ' | 'ċ' | 'č' => out.push('c'),
            'Ç' | 'Ć' | 'Ĉ' | 'Ċ' | 'Č' => out.push('C'),
            'ñ' | 'ń' | 'ņ' | 'ň' => out.push('n'),
            'Ñ' | 'Ń' | 'Ņ' | 'Ň' => out.push('N'),
            'ś' | 'ŝ' | 'ş' | 'š' | 'ș' => out.push('s'),
            'Ś' | 'Ŝ' | 'Ş' | 'Š' | 'Ș' => out.push('S'),
            'ź' | 'ż' | 'ž' => out.push('z'),
            'Ź' | 'Ż' | 'Ž' => out.push('Z'),
            'đ' | 'ď' => out.push('d'),
            'Đ' | 'Ď' => out.push('D'),
            'ł' | 'ĺ' | 'ļ' | 'ľ' => out.push('l'),
            'Ł' | 'Ĺ' | 'Ļ' | 'Ľ' => out.push('L'),
            'ř' | 'ŕ' | 'ŗ' => out.push('r'),
            'Ř' | 'Ŕ' | 'Ŗ' => out.push('R'),
            'ť' | 'ţ' | 'ț' => out.push('t'),
            'Ť' | 'Ţ' | 'Ț' => out.push('T'),
            'ğ' | 'ĝ' | 'ģ' | 'ġ' => out.push('g'),
            'Ğ' | 'Ĝ' | 'Ģ' | 'Ġ' => out.push('G'),
            'ĥ' | 'ħ' => out.push('h'),
            'Ĥ' | 'Ħ' => out.push('H'),
            'ĵ' => out.push('j'),
            'Ĵ' => out.push('J'),
            'ḱ' | 'ķ' | 'ǩ' => out.push('k'),
            'Ḱ' | 'Ķ' | 'Ǩ' => out.push('K'),
            'ẅ' | 'ŵ' | 'ẁ' | 'ẃ' => out.push('w'),
            'Ẅ' | 'Ŵ' | 'Ẁ' | 'Ẃ' => out.push('W'),
            'ß' => out.push_str("ss"),
            _ => out.push(c),
        }
    }
    out
}

fn map_chars(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut i = 0;

    while i < n {
        let ch = chars[i];

        // Triple (or more) ASCII dots → short pause
        if ch == '.' && i + 2 < n && chars[i + 1] == '.' && chars[i + 2] == '.' {
            push_soft_pause(&mut out);
            i += 3;
            while i < n && chars[i] == '.' {
                i += 1;
            }
            continue;
        }

        // ASCII double/triple hyphen often used as em dash
        if ch == '-' && i + 1 < n && chars[i + 1] == '-' {
            // digit--digit still “to”
            let prev = out.chars().rev().find(|c| !c.is_whitespace());
            let next = chars[i + 2..].iter().copied().find(|c| !c.is_whitespace());
            if prev.is_some_and(|c| c.is_ascii_digit()) && next.is_some_and(|c| c.is_ascii_digit())
            {
                push_token(&mut out, "to");
            } else {
                push_soft_pause(&mut out);
            }
            i += 2;
            while i < n && chars[i] == '-' {
                i += 1;
            }
            continue;
        }

        // Single ASCII hyphen: only “to” between digits (page ranges). Keep word hyphens.
        if ch == '-' {
            let prev = out.chars().rev().find(|c| !c.is_whitespace());
            let next = chars[i + 1..].iter().copied().find(|c| !c.is_whitespace());
            if heading_number_dash(&out, &chars, i) {
                push_soft_pause(&mut out);
                i += 1;
                continue;
            }
            if next.is_some_and(|c| c.is_ascii_digit())
                && (prev.is_some_and(|c| c.is_ascii_digit()) || number_unit_tail(&out))
            {
                push_token(&mut out, "to");
                i += 1;
                continue;
            }
            // Negative number: "-5 degrees", "(−3)". Not "COVID-19" (letter before).
            // Spaced sign in a list or bracket: `(51.0, - 3.19)`, `x = - 5`.
            let spaced_sign = chars.get(i + 1) == Some(&' ')
                && chars.get(i + 2).is_some_and(char::is_ascii_digit)
                && prev.is_some_and(|c| matches!(c, ',' | '(' | '[' | ':' | '='));
            if is_negative_sign_at(&chars, i) || spaced_sign {
                push_token(&mut out, "negative");
                i += 1;
                continue;
            }
            // Spaced hyphen used as a dash: `Appendix B - Logging`, `the rabbit - he maps`.
            if i > 0
                && chars[i - 1].is_whitespace()
                && chars.get(i + 1).is_some_and(|c| c.is_whitespace())
                && prev.is_some()
                && next.is_some_and(|c| !c.is_ascii_digit())
            {
                push_soft_pause(&mut out);
                i += 1;
                continue;
            }
            out.push('-');
            i += 1;
            continue;
        }

        // Exponent: x^2 → "x squared", 10^6 → "10 to the power of 6".
        if ch == '^'
            && i > 0
            && (chars[i - 1].is_alphanumeric() || chars[i - 1] == ')')
            && chars.get(i + 1).is_some_and(char::is_ascii_digit)
        {
            let mut j = i + 1;
            while j < n && chars[j].is_ascii_digit() {
                j += 1;
            }
            let exp: String = chars[i + 1..j].iter().collect();
            let words = match exp.as_str() {
                "2" => "squared".to_string(),
                "3" => "cubed".to_string(),
                e => format!("to the power of {e}"),
            };
            push_token(&mut out, &words);
            i = j;
            continue;
        }

        // Unicode minus: "5 − 3" is “minus” (subtraction), "−5" is “negative”.
        if ch == '\u{2212}' {
            let next = chars[i + 1..].iter().copied().find(|c| !c.is_whitespace());
            if is_negative_sign_at(&chars, i) {
                push_token(&mut out, "negative");
            } else if next.is_some_and(|c| c.is_ascii_digit()) {
                push_token(&mut out, "minus");
            } else {
                push_soft_pause(&mut out);
            }
            i += 1;
            continue;
        }

        match ch {
            // Soft hyphen: invisible hyphenation point — drop, no pause
            '\u{00AD}' => {}

            // Long dashes → soft pause, or “to” for digit ranges
            '\u{2014}' // em dash —
            | '\u{2013}' // en dash –
            | '\u{2012}' // figure dash
            | '\u{2015}' // horizontal bar
            | '\u{2E3A}' // two-em dash
            | '\u{2E3B}' // three-em dash
            | '\u{FE58}' // small em dash
            | '\u{FE63}' // small hyphen-minus
            | '\u{FF0D}' // fullwidth hyphen-minus
            => {
                let prev = out.chars().rev().find(|c| !c.is_whitespace());
                let next = chars[i + 1..].iter().copied().find(|c| !c.is_whitespace());
                // Typeset negatives (`–70 to –90 dBm`) use an en dash, not `-`.
                // Em dashes and bars are never minus signs.
                let minus_like = matches!(ch, '\u{2013}' | '\u{2012}' | '\u{FE63}' | '\u{FF0D}');
                let digit_next = next.is_some_and(|c| c.is_ascii_digit());
                if digit_next
                    && (prev.is_some_and(|c| c.is_ascii_digit()) || number_unit_tail(&out))
                    && !heading_number_dash(&out, &chars, i)
                {
                    push_token(&mut out, "to");
                } else if minus_like && is_negative_sign_at(&chars, i) {
                    push_token(&mut out, "negative");
                } else {
                    push_soft_pause(&mut out);
                }
            }

            // Unicode ellipsis
            '\u{2026}' => push_soft_pause(&mut out),

            // Bullets separate list items → short pause.
            '•' | '·' | '‣' | '∙' | '※' | '⁎' | '⁕' => {
                push_soft_pause(&mut out);
            }
            // Footnote / reference marks sit on words ("Note†") → silent, no pause.
            '†' | '‡' | '¶' => push_space(&mut out),

            // Brackets often mark asides/footnotes in books
            '(' | ')' | '[' | ']' | '{' | '}' | '⟨' | '⟩' | '«' | '»' | '‹' | '›' => {
                push_soft_pause(&mut out);
            }

            // Asterisk / diamond ornaments → space (not “asterisk”)
            '*' | '◇' | '◆' | '○' | '●' | '■' | '□' | '▪' | '▫' | '★' | '☆' | '♦' | '♠'
            | '♣' | '♥' | '✦' | '✧' | '❖' => push_space(&mut out),

            // Bare caret / modifier circumflex (glyph-name bait → “circumflex”)
            '^' | '\u{02C6}' | '\u{02C7}' | '\u{02D8}' | '\u{02D9}' | '\u{02DA}' | '\u{02DB}'
            | '\u{02DC}' | '\u{02DD}' | '\u{2038}' | '\u{A788}' => push_space(&mut out),

            // Underscore runs (PDF/OCR placeholders)
            '_' => push_space(&mut out),

            // Never speak: format / zero-width / BOM / bidi
            '\u{00A0}' // nbsp
            | '\u{1680}' // ogham space
            | '\u{2000}'..='\u{200B}' // en/em quad … zero-width space
            | '\u{200C}' | '\u{200D}' // ZWNJ ZWJ
            | '\u{200E}' | '\u{200F}' // LRM RLM
            | '\u{2028}' | '\u{2029}' // line/para sep
            | '\u{202A}'..='\u{202E}' // bidi overrides
            | '\u{202F}' // narrow nbsp
            | '\u{205F}' // medium math space
            | '\u{2060}'..='\u{2064}' // word joiner etc.
            | '\u{2066}'..='\u{2069}' // isolate controls
            | '\u{3000}' // ideographic space
            | '\u{FEFF}' // BOM
            => push_space(&mut out),

            // Form feed / vertical tab → paragraph break
            '\u{000B}' | '\u{000C}' => out.push('\n'),

            // Drop most control chars except tab/newline
            c if c.is_control() && c != '\n' && c != '\t' => {}

            // Emoji & pictographs → silence
            c if is_emoji_or_pictograph(c) => push_space(&mut out),

            // Private use
            c if is_private_use(c) => push_space(&mut out),

            // Free-standing modifier letters (ˆ ˇ …) espeak may name
            c if is_free_modifier_symbol(c) => push_space(&mut out),

            // Combining marks (Mn): keep only when attached to a letter (NFD
            // café). Lone U+0302 etc. make espeak say “circumflex”.
            c if is_combining_mark(c) => {
                if out.chars().last().is_some_and(|p| p.is_alphabetic()) {
                    out.push(c);
                }
                // else drop free-standing mark
            }

            // Pipe / backslash / trademark noise / replacement char
            '\\' | '|' | '©' | '®' | '™' | '�' | '¦' | '¬' => push_space(&mut out),

            // Arrows → soft pause (not “rightwards arrow”)
            '→' | '←' | '↔' | '⇒' | '⇐' | '⇔' | '⟶' | '⟵' | '↦' | '↵' | '↳' | '↪' | '↩' => {
                push_soft_pause(&mut out);
            }

            _ => out.push(ch),
        }
        i += 1;
    }
    out
}

// ---------------------------------------------------------------------------
// Book English / tech normalizers
// ---------------------------------------------------------------------------

/// Drop or soften URLs and emails so TTS does not read “slash slash”.
fn strip_urls_and_emails(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < n {
        // http:// https://
        if matches_ignore_case_at(&chars, i, "https://")
            || matches_ignore_case_at(&chars, i, "http://")
        {
            let start = i;
            i += if chars[i + 4].eq_ignore_ascii_case(&'s') {
                8
            } else {
                7
            };
            while i < n && !chars[i].is_whitespace() && chars[i] != ')' && chars[i] != ']' {
                i += 1;
            }
            let url: String = chars[start..i].iter().collect();
            let (spoken, tail) = speak_url_host(&url);
            push_token(&mut out, &spoken);
            push_tail(&mut out, tail);
            continue;
        }
        // www.
        if matches_ignore_case_at(&chars, i, "www.") {
            let start = i;
            while i < n && !chars[i].is_whitespace() && chars[i] != ')' && chars[i] != ']' {
                i += 1;
            }
            let url: String = chars[start..i].iter().collect();
            let (spoken, tail) = speak_url_host(&url);
            push_token(&mut out, &spoken);
            push_tail(&mut out, tail);
            continue;
        }
        // simple email: local@domain.tld (before @ → "at")
        if chars[i].is_ascii_alphanumeric() || chars[i] == '.' || chars[i] == '_' || chars[i] == '-'
        {
            let start = i;
            let mut j = i;
            while j < n
                && (chars[j].is_ascii_alphanumeric() || matches!(chars[j], '.' | '_' | '-' | '+'))
            {
                j += 1;
            }
            if j < n && chars[j] == '@' {
                let mut k = j + 1;
                let dom_start = k;
                while k < n
                    && (chars[k].is_ascii_alphanumeric() || chars[k] == '.' || chars[k] == '-')
                {
                    k += 1;
                }
                let mut domain: String = chars[dom_start..k].iter().collect();
                // Sentence full stop after the address is not part of it.
                let mut tail = String::new();
                while domain.ends_with('.') || domain.ends_with('-') {
                    tail.insert(0, domain.pop().unwrap_or('.'));
                }
                if domain.contains('.') && domain.len() >= 3 {
                    let local: String = chars[start..j].iter().collect();
                    let spoken = format!("{} at {}", speak_dotted(&local), speak_dotted(&domain));
                    push_token(&mut out, &spoken);
                    push_tail(&mut out, &tail);
                    i = k;
                    continue;
                }
            }
            // Every start inside this run ends at the same `j`: skip it whole.
            out.extend(&chars[start..j]);
            i = j;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Web suffixes that mark a bare host name. Two-letter country codes that are also
/// English words (`it`, `me`, `in`, `to`, `is`, `at`, `us`, `be`, `no`) are left out,
/// so a missing space after a full stop (`the end.it was`) is not read as an address.
const BARE_DOMAIN_TLDS: &[&str] = &[
    "com", "net", "org", "gov", "edu", "mil", "int", "info", "biz", "io", "co", "uk", "app", "dev",
    "ai", "tv", "xyz", "online", "site", "tech", "eu", "de", "fr", "ca", "au", "nz", "ie", "nl",
    "es", "ru", "cn", "jp", "ly", "gg",
];

/// `captive.apple.com`, `tinysa.org`, `gs-loc.apple.com` → “captive dot apple dot com” …
/// A host is 2+ dot-separated labels of letters, digits and `-`, ending in one of
/// [`BARE_DOMAIN_TLDS`], with at least one letter before the suffix. A full stop
/// right after it ends the sentence and is kept.
fn speak_bare_domains(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let label_char = |c: char| c.is_ascii_alphanumeric() || c == '-';
    let mut out = String::with_capacity(s.len() + 16);
    let mut i = 0;
    while i < n {
        let boundary_before =
            i == 0 || !(label_char(chars[i - 1]) || matches!(chars[i - 1], '.' | '@' | '/' | '_'));
        if boundary_before && chars[i].is_ascii_alphanumeric() {
            // Longest run of labels and dots.
            let mut j = i;
            while j < n
                && (label_char(chars[j])
                    || (chars[j] == '.'
                        && chars.get(j + 1).is_some_and(|c| c.is_ascii_alphanumeric())))
            {
                j += 1;
            }
            let after_ok = chars
                .get(j)
                .is_none_or(|c| !matches!(c, '@' | '/' | '_') && !c.is_alphanumeric());
            let host: String = chars[i..j].iter().collect();
            let labels: Vec<&str> = host.split('.').collect();
            let tld = labels
                .last()
                .map(|t| t.to_ascii_lowercase())
                .unwrap_or_default();
            let ok = after_ok
                && labels.len() >= 2
                && labels
                    .iter()
                    .all(|l| !l.is_empty() && !l.starts_with('-') && !l.ends_with('-'))
                && BARE_DOMAIN_TLDS.contains(&tld.as_str())
                && labels[..labels.len() - 1]
                    .iter()
                    .any(|l| l.chars().any(|c| c.is_ascii_alphabetic()));
            if ok {
                push_token(&mut out, &speak_dotted(&host));
                i = j;
                continue;
            }
            // Not a host: copy the run through unchanged.
            out.extend(&chars[i..j]);
            i = j;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Sentence punctuation right after a spoken token: no space before it.
fn push_tail(out: &mut String, tail: &str) {
    if !tail.is_empty() {
        while out.ends_with(' ') {
            out.pop();
        }
        out.push_str(tail);
    }
}

/// `https://www.example.com/path?q=1.` → ("example dot com", ".").
/// Only the host is spoken; paths and queries are noise when read aloud.
fn speak_url_host(url: &str) -> (String, &'static str) {
    let trimmed = url.trim_end_matches(['.', ',', ';', ':', '!', '?']);
    let tail = match &url[trimmed.len()..] {
        "" => "",
        t if t.starts_with('.') => ".",
        t if t.starts_with(',') => ",",
        t if t.starts_with(';') => ";",
        t if t.starts_with(':') => ":",
        t if t.starts_with('!') => "!",
        _ => "?",
    };
    let no_scheme = trimmed.split_once("://").map_or(trimmed, |(_, rest)| rest);
    let host = no_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .trim_start_matches("www.")
        .trim_start_matches("WWW.");
    if host.is_empty() {
        return ("link".into(), tail);
    }
    (speak_dotted(host), tail)
}

/// `mail.example.co` → “mail dot example dot co”; `john_smith` → “john underscore smith”.
fn speak_dotted(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        match c {
            '.' => out.push_str(" dot "),
            '_' => out.push_str(" underscore "),
            '-' => out.push_str(" dash "),
            '+' => out.push_str(" plus "),
            c => out.push(c),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn matches_ignore_case_at(chars: &[char], i: usize, lit: &str) -> bool {
    chars.get(i..i + lit.chars().count()).is_some_and(|w| {
        w.iter()
            .zip(lit.chars())
            .all(|(a, b)| a.eq_ignore_ascii_case(&b))
    })
}

/// `2:30` → “2 30”, `3:00` → “3 o'clock”, `2:05` → “2 oh 5” (Kokoro-style).
fn expand_times(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(s.len() + 8);
    let mut i = 0;
    while i < n {
        if chars[i].is_ascii_digit() && (i == 0 || !chars[i - 1].is_ascii_alphanumeric()) {
            let mut j = i;
            while j < n && chars[j].is_ascii_digit() {
                j += 1;
            }
            let hour: String = chars[i..j].iter().collect();
            if (1..=2).contains(&hour.len())
                && j < n
                && chars[j] == ':'
                && j + 1 < n
                && chars[j + 1].is_ascii_digit()
            {
                let mut k = j + 1;
                while k < n && chars[k].is_ascii_digit() {
                    k += 1;
                }
                let minute: String = chars[j + 1..k].iter().collect();
                let after_ok = k >= n || (!chars[k].is_ascii_alphanumeric() && chars[k] != ':');
                if minute.len() == 2 && after_ok {
                    if let (Ok(h), Ok(m)) = (hour.parse::<u32>(), minute.parse::<u32>()) {
                        if h <= 23 && m <= 59 {
                            let spoken = if m == 0 {
                                format!("{h} o'clock")
                            } else if m < 10 {
                                format!("{h} oh {m}")
                            } else {
                                format!("{h} {m}")
                            };
                            push_token(&mut out, &spoken);
                            i = k;
                            continue;
                        }
                    }
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// US slash dates `M/D/YYYY` / `MM/DD/YYYY` → spoken form.
///
/// Examples:
/// - `03/20/2026` → “March twentieth twenty twenty six”
/// - `12/10/2025` → “December tenth twenty twenty five”
/// - `3/15/2020` → “March fifteenth twenty twenty”
///
/// Only accepts a plausible calendar day (month 1–12, day valid for that month,
/// year 1000–9999). Invalid combos (e.g. `2/31/2020`) are left unchanged.
fn expand_slash_dates(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(s.len() + 24);
    let mut i = 0;
    while i < n {
        if chars[i].is_ascii_digit() && (i == 0 || !chars[i - 1].is_ascii_alphanumeric()) {
            // month
            let mut j = i;
            while j < n && chars[j].is_ascii_digit() {
                j += 1;
            }
            let month_digits: String = chars[i..j].iter().collect();
            if (1..=2).contains(&month_digits.len())
                && j < n
                && chars[j] == '/'
                && j + 1 < n
                && chars[j + 1].is_ascii_digit()
            {
                // day
                let mut k = j + 1;
                while k < n && chars[k].is_ascii_digit() {
                    k += 1;
                }
                let day_digits: String = chars[j + 1..k].iter().collect();
                if (1..=2).contains(&day_digits.len())
                    && k < n
                    && chars[k] == '/'
                    && k + 1 < n
                    && chars[k + 1].is_ascii_digit()
                {
                    // year (4 digits only)
                    let mut m = k + 1;
                    while m < n && chars[m].is_ascii_digit() {
                        m += 1;
                    }
                    let year_digits: String = chars[k + 1..m].iter().collect();
                    let after_ok = m >= n || !chars[m].is_ascii_alphanumeric();
                    if year_digits.len() == 4 && after_ok {
                        if let (Ok(month), Ok(day), Ok(year)) = (
                            month_digits.parse::<u32>(),
                            day_digits.parse::<u32>(),
                            year_digits.parse::<u32>(),
                        ) {
                            if let Some(spoken) = format_slash_date(month, day, year) {
                                push_token(&mut out, &spoken);
                                i = m;
                                continue;
                            }
                        }
                    }
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn format_slash_date(month: u32, day: u32, year: u32) -> Option<String> {
    if !(1..=12).contains(&month) || !(1000..=9999).contains(&year) {
        return None;
    }
    let max_day = days_in_month(month, year);
    if day == 0 || day > max_day {
        return None;
    }
    let month_name = MONTH_NAMES[month as usize];
    let day_ord = day_ordinal_word(day)?;
    let year_spoken = year_words(year);
    Some(format!("{month_name} {day_ord} {year_spoken}"))
}

const MONTH_NAMES: [&str; 13] = [
    "",
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Written month + year (and optional day): date-style years, not cardinals.
///
/// - `May 2026` → “May twenty twenty six”
/// - `May 15, 2026` → “May fifteenth twenty twenty six”
/// - `Jan. 3 1999` → “January third nineteen ninety nine”
///
/// Bare years without a month (`in 2026`) are left alone so counts/IDs stay digits.
fn expand_month_year_dates(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(s.len() + 24);
    let mut i = 0;
    while i < n {
        if chars[i].is_ascii_alphabetic() && (i == 0 || !chars[i - 1].is_ascii_alphanumeric()) {
            if let Some((month_idx, month_end)) = match_month_name_at(&chars, i) {
                // skip whitespace after month
                let mut j = month_end;
                while j < n && chars[j].is_whitespace() {
                    j += 1;
                }
                if j >= n || !chars[j].is_ascii_digit() {
                    out.push(chars[i]);
                    i += 1;
                    continue;
                }

                // Peek digit run after month: either YYYY or D/DD (+ optional st/nd/rd/th) + YYYY
                let mut k = j;
                while k < n && chars[k].is_ascii_digit() {
                    k += 1;
                }
                let first_digits: String = chars[j..k].iter().collect();

                // Case A: month + 4-digit year  →  "May 2026"
                if first_digits.len() == 4 {
                    let after_ok = k >= n || !chars[k].is_ascii_alphanumeric();
                    if after_ok {
                        if let Ok(year) = first_digits.parse::<u32>() {
                            if (1000..=9999).contains(&year) {
                                let spoken = format!(
                                    "{} {}",
                                    MONTH_NAMES[month_idx as usize],
                                    year_words(year)
                                );
                                push_token(&mut out, &spoken);
                                i = k;
                                continue;
                            }
                        }
                    }
                }

                // Case B: month + day + year  →  "May 15, 2026" / "May 15th 2026"
                if (1..=2).contains(&first_digits.len()) {
                    if let Ok(d) = first_digits.parse::<u32>() {
                        let mut after_day = k;
                        if k + 1 < n {
                            let suf: String = chars[k..]
                                .iter()
                                .take(2)
                                .collect::<String>()
                                .to_ascii_lowercase();
                            if matches!(suf.as_str(), "st" | "nd" | "rd" | "th") {
                                after_day = k + 2;
                            }
                        }
                        let mut y = after_day;
                        while y < n && (chars[y].is_whitespace() || chars[y] == ',') {
                            y += 1;
                        }
                        if y < n && chars[y].is_ascii_digit() {
                            let mut z = y;
                            while z < n && chars[z].is_ascii_digit() {
                                z += 1;
                            }
                            let year_digits: String = chars[y..z].iter().collect();
                            let after_ok = z >= n || !chars[z].is_ascii_alphanumeric();
                            if year_digits.len() == 4 && after_ok {
                                if let Ok(year) = year_digits.parse::<u32>() {
                                    if (1000..=9999).contains(&year)
                                        && (1..=days_in_month(month_idx, year)).contains(&d)
                                    {
                                        if let Some(ord) = day_ordinal_word(d) {
                                            let spoken = format!(
                                                "{} {} {}",
                                                MONTH_NAMES[month_idx as usize],
                                                ord,
                                                year_words(year)
                                            );
                                            push_token(&mut out, &spoken);
                                            i = z;
                                            continue;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Match full or abbreviated English month at `i`. Returns (month 1–12, end index).
fn match_month_name_at(chars: &[char], i: usize) -> Option<(u32, usize)> {
    const MONTHS: &[(&str, u32)] = &[
        ("january", 1),
        ("february", 2),
        ("september", 9),
        ("october", 10),
        ("november", 11),
        ("december", 12),
        ("august", 8),
        ("march", 3),
        ("april", 4),
        ("june", 6),
        ("july", 7),
        ("jan", 1),
        ("feb", 2),
        ("mar", 3),
        ("apr", 4),
        ("may", 5),
        ("jun", 6),
        ("jul", 7),
        ("aug", 8),
        ("sep", 9),
        ("sept", 9),
        ("oct", 10),
        ("nov", 11),
        ("dec", 12),
    ];
    // Keep the longest match (March over Mar, Sept. over Sep).
    let mut best: Option<(u32, usize)> = None;
    for &(name, idx) in MONTHS {
        let len = name.len();
        if !matches_ignore_case_at(chars, i, name) {
            continue;
        }
        let mut end = i + len;
        // optional period after abbrev: "Jan."
        if end < chars.len() && chars[end] == '.' {
            end += 1;
        }
        // word boundary after month
        if end < chars.len() && chars[end].is_ascii_alphanumeric() {
            continue;
        }
        if best.is_none_or(|(_, be)| end > be) {
            best = Some((idx, end));
        }
    }
    best
}

fn days_in_month(month: u32, year: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

fn is_leap_year(year: u32) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

/// Day-of-month ordinal as spoken English (1–31).
fn day_ordinal_word(day: u32) -> Option<&'static str> {
    Some(match day {
        1 => "first",
        2 => "second",
        3 => "third",
        4 => "fourth",
        5 => "fifth",
        6 => "sixth",
        7 => "seventh",
        8 => "eighth",
        9 => "ninth",
        10 => "tenth",
        11 => "eleventh",
        12 => "twelfth",
        13 => "thirteenth",
        14 => "fourteenth",
        15 => "fifteenth",
        16 => "sixteenth",
        17 => "seventeenth",
        18 => "eighteenth",
        19 => "nineteenth",
        20 => "twentieth",
        21 => "twenty first",
        22 => "twenty second",
        23 => "twenty third",
        24 => "twenty fourth",
        25 => "twenty fifth",
        26 => "twenty sixth",
        27 => "twenty seventh",
        28 => "twenty eighth",
        29 => "twenty ninth",
        30 => "thirtieth",
        31 => "thirty first",
        _ => return None,
    })
}

/// Year as casual spoken English: 2026 → “twenty twenty six”.
fn year_words(year: u32) -> String {
    match year {
        2000 => "two thousand".into(),
        2001..=2009 => format!("two thousand {}", ones_word(year % 10)),
        // 2010–2099: “twenty ten” … “twenty twenty six”
        2010..=2099 => format!("twenty {}", two_digit_words(year % 100)),
        // Fallback: split as pairs (e.g. 1789 → “seventeen eighty nine”)
        y if (1000..=9999).contains(&y) => {
            let hi = y / 100;
            let lo = y % 100;
            if lo == 0 {
                format!("{} hundred", two_digit_words(hi))
            } else if lo < 10 {
                format!("{} oh {}", two_digit_words(hi), ones_word(lo))
            } else {
                format!("{} {}", two_digit_words(hi), two_digit_words(lo))
            }
        }
        y => y.to_string(),
    }
}

/// Decimals, decades and years in context, as words.
///
/// Written out so the reading never depends on the phonemizer's own number rules
/// (some read `23.5` as "two three five", `1847` as "one thousand eight hundred and
/// forty seven", `1990s` as "… ninety ess").
///
/// Years are only rewritten where context says "year": after `in/since/by/of/from/
/// until/to/circa/around/before/after/during/early/late/mid/year/born/died/AD`,
/// inside brackets `(1847)`, or in a year range `1847–1850`. Elsewhere a 4-digit
/// number stays a count ("1847 soldiers", "$2,026").
fn expand_decimals_years_decades(s: &str) -> String {
    const YEAR_CONTEXT: &[&str] = &[
        "in", "since", "by", "of", "from", "until", "till", "to", "through", "circa", "ca",
        "around", "before", "after", "during", "early", "late", "mid", "year", "born", "died",
        "ad", "summer", "winter", "spring", "autumn", "fall", "between", "and",
    ];
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(s.len() + 32);
    let mut i = 0;
    let digits_at = |at: usize| -> usize {
        let mut k = at;
        while k < n && chars[k].is_ascii_digit() {
            k += 1;
        }
        k
    };
    let year_ok = |v: u32| (1100..=2099).contains(&v);
    while i < n {
        let c = chars[i];
        // '80s / ’80s → "eighties"
        if c == '\'' && chars.get(i + 1).is_some_and(char::is_ascii_digit) {
            let k = digits_at(i + 1);
            if k - (i + 1) == 2
                && chars.get(k) == Some(&'s')
                && !chars.get(k + 1).is_some_and(|c| c.is_alphanumeric())
            {
                let v: u32 = chars[i + 1..k]
                    .iter()
                    .collect::<String>()
                    .parse()
                    .unwrap_or(1);
                if v.is_multiple_of(10) && v >= 10 {
                    push_token(&mut out, &pluralize_number_words(&two_digit_words(v)));
                    i = k + 1;
                    continue;
                }
            }
        }
        let start_ok = c.is_ascii_digit()
            && (i == 0
                || !(chars[i - 1].is_alphanumeric()
                    || matches!(chars[i - 1], '.' | ',' | '$' | '€' | '£' | '#')));
        if !start_ok {
            out.push(c);
            i += 1;
            continue;
        }
        let k = digits_at(i);
        let int_part: String = chars[i..k].iter().collect();

        // Decimal / dotted number: 23.5, 3.14159, 2.0.1
        if chars.get(k) == Some(&'.') && chars.get(k + 1).is_some_and(char::is_ascii_digit) {
            let mut groups = vec![int_part.clone()];
            let mut e = k;
            while chars.get(e) == Some(&'.') && chars.get(e + 1).is_some_and(char::is_ascii_digit) {
                let g_end = digits_at(e + 1);
                groups.push(chars[e + 1..g_end].iter().collect());
                e = g_end;
            }
            if !chars.get(e).is_some_and(|c| c.is_alphanumeric()) {
                let mut spoken = groups[0].clone();
                for g in &groups[1..] {
                    spoken.push_str(" point ");
                    spoken.push_str(&g.chars().map(String::from).collect::<Vec<_>>().join(" "));
                }
                push_token(&mut out, &spoken);
                i = e;
                continue;
            }
        }

        if k - i == 4 {
            let v: u32 = int_part.parse().unwrap_or(0);
            // Decade: 1990s, 1800s
            if chars.get(k) == Some(&'s')
                && !chars.get(k + 1).is_some_and(|c| c.is_alphanumeric())
                && v.is_multiple_of(10)
                && year_ok(v)
            {
                push_token(&mut out, &pluralize_number_words(&year_words(v)));
                i = k + 1;
                continue;
            }
            let next_is_alnum = chars.get(k).is_some_and(|c| c.is_alphanumeric());
            let next_is_number_cont = matches!(chars.get(k), Some(',' | '.'))
                && chars.get(k + 1).is_some_and(char::is_ascii_digit);
            if year_ok(v) && !next_is_alnum && !next_is_number_cont && chars.get(k) != Some(&'%') {
                let prev_word: String = out
                    .trim_end()
                    .chars()
                    .rev()
                    .take_while(|c| c.is_alphabetic())
                    .collect::<Vec<_>>()
                    .into_iter()
                    .rev()
                    .collect::<String>()
                    .to_lowercase();
                let in_brackets = out.trim_end().ends_with('(') && chars.get(k) == Some(&')');
                // Year range partner: 1847-1850 / 1847–1850 (either side)
                let dash_after = matches!(chars.get(k), Some('-' | '–' | '—')) && {
                    let e = digits_at(k + 1);
                    e - (k + 1) == 4
                        && chars[k + 1..e]
                            .iter()
                            .collect::<String>()
                            .parse::<u32>()
                            .is_ok_and(year_ok)
                };
                let dash_before = matches!(out.chars().last(), Some('-' | '–' | '—')) && {
                    let t: String = out
                        .chars()
                        .rev()
                        .skip(1)
                        .take(4)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect();
                    t.len() == 4 && t.parse::<u32>().is_ok_and(year_ok)
                };
                let context =
                    YEAR_CONTEXT.contains(&prev_word.as_str()) || in_brackets || dash_after;
                if context && !dash_before {
                    push_token(&mut out, &year_words(v));
                    // A following range year is converted on its own turn via `dash_after`
                    // of this one: mark by converting it here.
                    if dash_after {
                        let e = digits_at(k + 1);
                        let v2: u32 = chars[k + 1..e]
                            .iter()
                            .collect::<String>()
                            .parse()
                            .unwrap_or(0);
                        push_token(&mut out, "to");
                        push_token(&mut out, &year_words(v2));
                        i = e;
                        continue;
                    }
                    i = k;
                    continue;
                }
            }
        }
        out.push_str(&int_part);
        i = k;
    }
    out
}

/// "nineteen ninety" → "nineteen nineties", "eighteen hundred" → "eighteen hundreds".
fn pluralize_number_words(words: &str) -> String {
    match words.strip_suffix('y') {
        Some(stem) => format!("{stem}ies"),
        None => format!("{words}s"),
    }
}

fn ones_word(n: u32) -> &'static str {
    match n {
        0 => "zero",
        1 => "one",
        2 => "two",
        3 => "three",
        4 => "four",
        5 => "five",
        6 => "six",
        7 => "seven",
        8 => "eight",
        9 => "nine",
        _ => "",
    }
}

fn two_digit_words(n: u32) -> String {
    match n {
        0..=9 => ones_word(n).to_string(),
        10 => "ten".into(),
        11 => "eleven".into(),
        12 => "twelve".into(),
        13 => "thirteen".into(),
        14 => "fourteen".into(),
        15 => "fifteen".into(),
        16 => "sixteen".into(),
        17 => "seventeen".into(),
        18 => "eighteen".into(),
        19 => "nineteen".into(),
        20 => "twenty".into(),
        30 => "thirty".into(),
        40 => "forty".into(),
        50 => "fifty".into(),
        60 => "sixty".into(),
        70 => "seventy".into(),
        80 => "eighty".into(),
        90 => "ninety".into(),
        21..=29 => format!("twenty {}", ones_word(n % 10)),
        31..=39 => format!("thirty {}", ones_word(n % 10)),
        41..=49 => format!("forty {}", ones_word(n % 10)),
        51..=59 => format!("fifty {}", ones_word(n % 10)),
        61..=69 => format!("sixty {}", ones_word(n % 10)),
        71..=79 => format!("seventy {}", ones_word(n % 10)),
        81..=89 => format!("eighty {}", ones_word(n % 10)),
        91..=99 => format!("ninety {}", ones_word(n % 10)),
        _ => n.to_string(),
    }
}

/// `1st` `2nd` `21st` → spoken ordinals (common set + generic “N th”).
fn expand_ordinals(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(s.len() + 8);
    let mut i = 0;
    while i < n {
        if chars[i].is_ascii_digit() && (i == 0 || !chars[i - 1].is_ascii_alphanumeric()) {
            let mut j = i;
            while j < n && chars[j].is_ascii_digit() {
                j += 1;
            }
            let digits: String = chars[i..j].iter().collect();
            let rest: String = chars[j..]
                .iter()
                .take(2)
                .collect::<String>()
                .to_ascii_lowercase();
            let (suf_len, word) = if rest.starts_with("st")
                || rest.starts_with("nd")
                || rest.starts_with("rd")
                || rest.starts_with("th")
            {
                let suf = &rest[..2];
                let after = j + 2;
                let boundary = after >= n || !chars[after].is_ascii_alphanumeric();
                if boundary {
                    if let Ok(num) = digits.parse::<u32>() {
                        if let Some(w) = ordinal_word(num) {
                            (2usize, Some(w))
                        } else {
                            // No word form: keep "42nd" as written; espeak reads it.
                            (2, Some(format!("{digits}{suf}")))
                        }
                    } else {
                        (0, None)
                    }
                } else {
                    (0, None)
                }
            } else {
                (0, None)
            };
            if let Some(w) = word {
                push_token(&mut out, &w);
                i = j + suf_len;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn ordinal_word(n: u32) -> Option<String> {
    day_ordinal_word(n).map(str::to_string)
}

/// `out` ends in “number + unit/currency words” (`20 pounds`, `100 kilohertz`,
/// `3 volts per meter`): a dash then a digit is a range (`£20–£30`,
/// `100 kHz – 7.3 GHz`) → “to”, not a minus or a pause.
fn number_unit_tail(out: &str) -> bool {
    const UNIT_WORDS: &[&str] = &[
        "hertz",
        "kilohertz",
        "megahertz",
        "gigahertz",
        "hour",
        "hours",
        "second",
        "seconds",
        "milligrams",
        "grams",
        "kilograms",
        "kilogram",
        "millimeters",
        "centimeters",
        "centimeter",
        "meters",
        "meter",
        "kilometers",
        "kilometer",
        "milliseconds",
        "watts",
        "milliwatts",
        "microwatts",
        "nanowatts",
        "picowatts",
        "kilowatts",
        "megawatts",
        "volts",
        "millivolts",
        "kilovolts",
        "amps",
        "milligauss",
        "nanotesla",
        "microtesla",
        "decibels",
        "Celsius",
        "Fahrenheit",
        "degrees",
        "percent",
        "pounds",
        "pound",
        "dollars",
        "dollar",
        "euros",
        "euro",
        "pence",
        "cents",
        "yen",
        "per",
        "square",
        "miles",
        "peak",
        "to",
        "dee",
        "bee",
        "em",
        "eye",
        // Singular forms after “1”.
        "milligram",
        "gram",
        "millimeter",
        "millisecond",
        "watt",
        "milliwatt",
        "microwatt",
        "nanowatt",
        "picowatt",
        "kilowatt",
        "megawatt",
        "volt",
        "millivolt",
        "kilovolt",
        "amp",
        "decibel",
        "degree",
        "mile",
    ];
    let line = out.rsplit('\n').next().unwrap_or("");
    let mut saw_unit = false;
    for w in line.split_whitespace().rev().take(6) {
        if w.chars().all(|c| c.is_ascii_digit()) {
            return saw_unit;
        }
        // Connectors only count inside a unit phrase (`peak to peak`, `per meter`),
        // never as the last word: `70 to –90` is a minus, not a range.
        if !UNIT_WORDS.contains(&w) || (!saw_unit && matches!(w, "to" | "per" | "square")) {
            return false;
        }
        saw_unit = true;
    }
    false
}

/// `#BlueSky` → “hashtag Daves Grid”. `#7` (a number) is left for “number 7”.
fn expand_hashtags(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(s.len() + 16);
    let mut i = 0;
    while i < n {
        if chars[i] == '#'
            && (i == 0 || !chars[i - 1].is_alphanumeric())
            && chars.get(i + 1).is_some_and(|c| c.is_ascii_alphabetic())
        {
            let mut j = i + 1;
            while j < n && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            let mut words = String::new();
            for k in i + 1..j {
                let c = chars[k];
                if c == '_' {
                    words.push(' ');
                    continue;
                }
                let p = chars[k - 1];
                let boundary = k > i + 1
                    && ((p.is_ascii_lowercase() && c.is_ascii_uppercase())
                        || (p.is_ascii_alphabetic() && c.is_ascii_digit()));
                if boundary {
                    words.push(' ');
                }
                words.push(c);
            }
            push_token(&mut out, &format!("hashtag {}", words.trim()));
            i = j;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Two words run together where the PDF lost a space or line break:
/// `dayNo` → “day No”, `TriggerActivated` → “Trigger Activated”.
/// Only when every part is a dictionary word and the whole is not, so brand
/// names like `iPhone`, `TinySA`, `TriField` are left alone.
fn split_glued_words(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(s.len() + 8);
    let mut i = 0;
    while i < n {
        if chars[i].is_ascii_alphabetic() && (i == 0 || !chars[i - 1].is_alphanumeric()) {
            let mut j = i;
            while j < n && chars[j].is_ascii_alphabetic() {
                j += 1;
            }
            let after_ok = chars.get(j).is_none_or(|c| !c.is_alphanumeric());
            let tok = &chars[i..j];
            // Split before each capital that follows a lowercase letter.
            let mut parts: Vec<String> = vec![String::new()];
            for (k, &c) in tok.iter().enumerate() {
                if k > 0 && tok[k - 1].is_ascii_lowercase() && c.is_ascii_uppercase() {
                    parts.push(String::new());
                }
                parts.last_mut().unwrap().push(c);
            }
            let whole: String = tok.iter().collect();
            // `TETRAadjacent`, `IMSIcatcher`: caps run + lowercase dictionary word.
            let caps_len = tok.iter().take_while(|c| c.is_ascii_uppercase()).count();
            let rest_lower = tok[caps_len..].iter().all(char::is_ascii_lowercase);
            if after_ok && caps_len >= 2 && rest_lower && tok.len() - caps_len >= 3 {
                let lower: String = tok[caps_len..].iter().collect();
                // `IMSIcatcher` → IMSI | catcher; `HTMLElement`-style → HTML | Element.
                let one_back: String = tok[caps_len - 1..].iter().collect();
                let split_at = if is_dictionary_word(&lower) {
                    Some(caps_len)
                } else if caps_len >= 3 && is_dictionary_word(&one_back) {
                    Some(caps_len - 1)
                } else {
                    None
                };
                if let Some(k) = split_at {
                    out.extend(&tok[..k]);
                    out.push(' ');
                    out.extend(&tok[k..]);
                    i = j;
                    continue;
                }
            }
            let ok = after_ok
                && parts.len() >= 2
                && parts.iter().all(|p| {
                    p.chars().count() >= 2
                        && p.chars().skip(1).all(|c| c.is_ascii_lowercase())
                        && is_dictionary_word(p)
                })
                && !is_dictionary_word(&whole);
            if ok {
                out.push_str(&parts.join(" "));
            } else {
                out.push_str(&whole);
            }
            i = j;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Spaced dash right after a heading number: `Chapter 4.5 – 400 MHz`,
/// `Part 2 - 3 Ways` → a pause, not a “4.5 to 400” range. By the time
/// `map_chars` runs, `4.5` already reads `4 point 5`.
fn heading_number_dash(out: &str, chars: &[char], i: usize) -> bool {
    const HEADS: &[&str] = &[
        "chapter", "part", "section", "appendix", "book", "volume", "lesson", "step",
    ];
    let spaced = i > 0
        && chars[i - 1].is_whitespace()
        && chars.get(i + 1).is_some_and(|c| c.is_whitespace());
    if !spaced {
        return false;
    }
    let line = out.rsplit('\n').next().unwrap_or("");
    let words: Vec<&str> = line.split_whitespace().collect();
    // Walk back over the number words (`4`, `point`, `5`) to the heading word.
    let mut k = words.len();
    while k > 0 && (words[k - 1] == "point" || words[k - 1].chars().all(|c| c.is_ascii_digit())) {
        k -= 1;
    }
    k < words.len()
        && k > 0
        && words[k..]
            .iter()
            .any(|w| w.chars().all(|c| c.is_ascii_digit()))
        && HEADS.contains(&words[k - 1].to_lowercase().as_str())
}

/// Reference numbers and ratios, before the `N/M` → “N over M” fraction rule.
///
/// - A run of 7+ digits, or 3+ digits starting with `0` (`202303608`,
///   `02153268`, `007`), is an ID: digits read one by one. Amounts with
///   separators (`1,000,000`) and decimals (`3.191892`) are not touched.
/// - A slash chain with such an ID or 5+ digit part (`01/8311886/25`,
///   `26/133012`) → every part read as digits, parts paused.
/// - A 3+ part non-date chain (`392/480/363`) is a list → “392, 480, 363”.
/// - Common ratios `24/7`, `50/50`, `80/20` … → “24 7”, “50 50”, “80 20”.
fn expand_ids_and_ratios(s: &str) -> String {
    const RATIOS: &[&str] = &[
        "24/7", "50/50", "60/40", "70/30", "80/20", "90/10", "20/20", "9/11",
    ];
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(s.len() + 16);
    let digits_at = |mut k: usize| {
        while k < n && chars[k].is_ascii_digit() {
            k += 1;
        }
        k
    };
    let spell = |g: &[char]| g.iter().map(char::to_string).collect::<Vec<_>>().join(" ");
    let mut i = 0;
    while i < n {
        // After a letter group is fine (`01/RoA/26/133012`): a chain that began on a
        // digit is consumed whole, so `/` here never splits one of ours.
        let start_ok =
            i == 0 || !(chars[i - 1].is_alphanumeric() || matches!(chars[i - 1], '.' | ','));
        if chars[i].is_ascii_digit() && start_ok {
            // Collect `digits(/digits)*`.
            let mut groups: Vec<(usize, usize)> = vec![(i, digits_at(i))];
            loop {
                let e = groups.last().unwrap().1;
                if chars.get(e) == Some(&'/') && chars.get(e + 1).is_some_and(char::is_ascii_digit)
                {
                    groups.push((e + 1, digits_at(e + 1)));
                } else {
                    break;
                }
            }
            let end = groups.last().unwrap().1;
            let cont = chars.get(end).is_some_and(|c| c.is_alphanumeric())
                || (matches!(chars.get(end), Some('.' | ',' | ':'))
                    && chars.get(end + 1).is_some_and(char::is_ascii_digit));
            if !cont {
                let text: String = chars[i..end].iter().collect();
                let is_id =
                    |&(a, b): &(usize, usize)| b - a >= 7 || (b - a >= 3 && chars[a] == '0');
                if RATIOS.contains(&text.as_str()) {
                    push_token(&mut out, &text.replace('/', " "));
                    i = end;
                    continue;
                }
                let id_chain =
                    groups.len() > 1 && groups.iter().any(|&(a, b)| b - a >= 5 || is_id(&(a, b)));
                // 3+ part chain that is not a date (dates went earlier): a list,
                // `392/480/363 MHz` → “392, 480, 363”.
                if groups.len() >= 3 && !id_chain {
                    let spoken = groups
                        .iter()
                        .map(|&(a, b)| chars[a..b].iter().collect::<String>())
                        .collect::<Vec<_>>()
                        .join(", ");
                    push_token(&mut out, &spoken);
                    i = end;
                    continue;
                }
                if id_chain || (groups.len() == 1 && is_id(&groups[0])) {
                    let spoken = groups
                        .iter()
                        .map(|&(a, b)| spell(&chars[a..b]))
                        .collect::<Vec<_>>()
                        .join(", ");
                    push_token(&mut out, &spoken);
                    i = end;
                    continue;
                }
            }
            // Not ours: copy the whole run so a later group is not re-read alone.
            for &c in &chars[i..end] {
                out.push(c);
            }
            i = end;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// `1/2` `3/4` → words; other `N/M` → “N over M”.
fn expand_ascii_fractions(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(s.len() + 8);
    let mut i = 0;
    while i < n {
        if chars[i].is_ascii_digit() && (i == 0 || !chars[i - 1].is_ascii_alphanumeric()) {
            let mut j = i;
            while j < n && chars[j].is_ascii_digit() {
                j += 1;
            }
            if j < n && chars[j] == '/' && j + 1 < n && chars[j + 1].is_ascii_digit() {
                let mut k = j + 1;
                while k < n && chars[k].is_ascii_digit() {
                    k += 1;
                }
                let after_ok = k >= n || !chars[k].is_ascii_alphanumeric();
                // avoid dates 3/15/2020 — if another /digit follows, skip
                let looks_like_date =
                    k < n && chars[k] == '/' && k + 1 < n && chars[k + 1].is_ascii_digit();
                if after_ok && !looks_like_date {
                    let num: String = chars[i..j].iter().collect();
                    let den: String = chars[j + 1..k].iter().collect();
                    let spoken = match (num.as_str(), den.as_str()) {
                        ("1", "2") => "one half".into(),
                        ("1", "3") => "one third".into(),
                        ("2", "3") => "two thirds".into(),
                        ("1", "4") => "one quarter".into(),
                        ("3", "4") => "three quarters".into(),
                        ("1", "5") => "one fifth".into(),
                        ("2", "5") => "two fifths".into(),
                        ("1", "8") => "one eighth".into(),
                        ("3", "8") => "three eighths".into(),
                        ("5", "8") => "five eighths".into(),
                        ("7", "8") => "seven eighths".into(),
                        _ => format!("{num} over {den}"),
                    };
                    push_token(&mut out, &spoken);
                    i = k;
                    continue;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Number + unit: `5km`, `2.4 GHz`, `72°F`, `400 MHz`.
fn expand_units(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(s.len() + 16);
    let mut i = 0;
    'walk: while i < n {
        // Optional number (with decimal)
        if chars[i].is_ascii_digit() && (i == 0 || !chars[i - 1].is_ascii_alphanumeric()) {
            let mut j = i;
            while j < n && chars[j].is_ascii_digit() {
                j += 1;
            }
            if j < n && chars[j] == '.' && j + 1 < n && chars[j + 1].is_ascii_digit() {
                j += 1;
                while j < n && chars[j].is_ascii_digit() {
                    j += 1;
                }
            }
            let num: String = chars[i..j].iter().collect();
            let mut k = j;
            while k < n && chars[k].is_whitespace() {
                k += 1;
            }
            // °F / °C
            if k < n && chars[k] == '°' {
                let mut u = k + 1;
                while u < n && chars[u].is_whitespace() {
                    u += 1;
                }
                if u < n && matches!(chars[u], 'F' | 'f' | 'C' | 'c') {
                    let end = u + 1;
                    if end >= n || !chars[end].is_ascii_alphanumeric() {
                        let name = if chars[u].eq_ignore_ascii_case(&'f') {
                            "degrees Fahrenheit"
                        } else {
                            "degrees Celsius"
                        };
                        push_token(&mut out, &format!("{num} {}", unit_number(&num, name)));
                        i = end;
                        continue;
                    }
                }
            }
            if let Some((ulen, unit_words)) = match_unit_at(&chars, k) {
                // Compound rate: `V/m`, `nW/cm²` → “volts per meter”, “… per square centimeter”.
                let phrase = match match_unit_denominator_at(&chars, k + ulen) {
                    Some((dlen, per)) => {
                        i = k + ulen + dlen;
                        format!("{num} {} per {per}", unit_number(&num, unit_words))
                    }
                    None => {
                        i = k + ulen;
                        format!("{num} {}", unit_number(&num, unit_words))
                    }
                };
                push_token(&mut out, &phrase);
                continue;
            }
        }
        // Bare compound unit, e.g. a table header `(V/m)` or `µW/cm²`: unambiguous
        // with the slash, so expand it even with no number in front.
        if i == 0 || !chars[i - 1].is_alphanumeric() {
            if let Some((ulen, unit_words)) = match_unit_at(&chars, i) {
                if let Some((dlen, per)) = match_unit_denominator_at(&chars, i + ulen) {
                    push_token(&mut out, &format!("{unit_words} per {per}"));
                    i += ulen + dlen;
                    continue;
                }
            }
        }
        // `dBm` / `dBi` can't be anything else: expand even alone (`… and dBm`).
        if i == 0 || !chars[i - 1].is_alphanumeric() {
            let tail_ok = |k: usize| chars.get(k).is_none_or(|c| !c.is_alphanumeric());
            if matches_ignore_case_at(&chars, i, "dbm") && chars[i + 1] == 'B' && tail_ok(i + 3) {
                push_token(&mut out, "dee bee em");
                i += 3;
                continue;
            }
            for (u, w) in [
                ("MHz", "megahertz"),
                ("GHz", "gigahertz"),
                ("kHz", "kilohertz"),
            ] {
                if chars
                    .get(i..i + 3)
                    .is_some_and(|sl| sl.iter().copied().eq(u.chars()))
                    && tail_ok(i + 3)
                {
                    push_token(&mut out, w);
                    i += 3;
                    continue 'walk;
                }
            }
            // Power and field units (`power in pW/nW`, `(mG)`): case-sensitive, unambiguous.
            for (u, w) in [
                ("pW", "picowatts"),
                ("nW", "nanowatts"),
                ("µW", "microwatts"),
                ("μW", "microwatts"),
                ("mW", "milliwatts"),
                ("mG", "milligauss"),
            ] {
                if chars
                    .get(i..i + 2)
                    .is_some_and(|sl| sl.iter().copied().eq(u.chars()))
                    && tail_ok(i + 2)
                {
                    push_token(&mut out, w);
                    i += 2;
                    continue 'walk;
                }
            }
            if matches_ignore_case_at(&chars, i, "mvpp") && chars[i + 1] == 'V' && tail_ok(i + 4) {
                push_token(&mut out, "millivolts peak to peak");
                i += 4;
                continue;
            }
            if matches_ignore_case_at(&chars, i, "dbi") && chars[i + 1] == 'B' && tail_ok(i + 3) {
                push_token(&mut out, "dee bee eye");
                i += 3;
                continue;
            }
        }
        // Other bare unit tokens: skip — only with numbers
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// “1 milliwatt”, “1 degree Fahrenheit”, “1 mile per hour”: singular unit after exactly 1.
/// Hertz, gauss and tesla are the same either way.
fn unit_number(num: &str, words: &str) -> String {
    if num != "1" {
        return words.to_string();
    }
    let (first, rest) = words.split_once(' ').unwrap_or((words, ""));
    let first = if first.ends_with('s') && !first.ends_with("ss") {
        &first[..first.len() - 1]
    } else {
        first
    };
    if rest.is_empty() {
        first.to_string()
    } else {
        format!("{first} {rest}")
    }
}

/// Longest-first unit match at `i`. Returns (char_len, spoken words).
fn match_unit_at(chars: &[char], i: usize) -> Option<(usize, &'static str)> {
    if i >= chars.len() {
        return None;
    }
    // First match wins: multi-letter units before single letters.
    const UNITS: &[(&str, &str)] = &[
        ("MHz", "megahertz"),
        ("GHz", "gigahertz"),
        ("kHz", "kilohertz"),
        ("mph", "miles per hour"),
        ("km/h", "kilometers per hour"),
        ("mg", "milligrams"),
        ("kg", "kilograms"),
        ("mm", "millimeters"),
        ("cm", "centimeters"),
        ("km", "kilometers"),
        ("Hz", "hertz"),
        ("ms", "milliseconds"),
        ("mW", "milliwatts"),
        ("µW", "microwatts"),
        ("μW", "microwatts"),
        ("uW", "microwatts"),
        ("nW", "nanowatts"),
        ("pW", "picowatts"),
        ("mVpp", "millivolts peak to peak"),
        ("Vpp", "volts peak to peak"),
        ("mV", "millivolts"),
        ("nT", "nanotesla"),
        ("µT", "microtesla"),
        ("μT", "microtesla"),
        ("uT", "microtesla"),
        ("kV", "kilovolts"),
        ("mG", "milligauss"),
        ("kW", "kilowatts"),
        ("MW", "megawatts"),
        ("dBm", "dee bee em"),
        ("dBi", "dee bee eye"),
        ("dB", "decibels"),
        ("°F", "degrees Fahrenheit"),
        ("°C", "degrees Celsius"),
        ("m", "meters"),
        ("g", "grams"),
        ("W", "watts"),
        ("V", "volts"),
        ("A", "amps"),
    ];
    for (unit, words) in UNITS {
        let u: Vec<char> = unit.chars().collect();
        if i + u.len() > chars.len() {
            continue;
        }
        let slice = &chars[i..i + u.len()];
        let eq = if unit.chars().all(|c| c.is_ascii_alphabetic()) {
            // Case carries meaning for single letters and milli/mega prefixes
            // (`mW` vs `MW`, `mG` vs `mg`); other units match any case (`MHZ`).
            let exact = slice.iter().zip(u.iter()).all(|(a, b)| a == b);
            let case_matters = unit.len() <= 1
                || unit.starts_with(['m', 'M']) && !matches!(*unit, "mm" | "mph" | "MHz");
            exact
                || (!case_matters
                    && slice
                        .iter()
                        .zip(unit.chars())
                        .all(|(a, b)| a.eq_ignore_ascii_case(&b)))
        } else {
            slice.iter().zip(u.iter()).all(|(a, b)| a == b)
        };
        if !eq {
            continue;
        }
        let end = i + u.len();
        if end < chars.len() && chars[end].is_ascii_alphanumeric() {
            continue;
        }
        return Some((u.len(), words));
    }
    None
}

/// `/m`, `/cm²`, `/s` right after a unit → (char_len incl. slash, singular words).
fn match_unit_denominator_at(chars: &[char], i: usize) -> Option<(usize, &'static str)> {
    if chars.get(i) != Some(&'/') {
        return None;
    }
    // Longest first; `²`/`2` squared forms before the bare unit.
    const DENOMS: &[(&str, &str)] = &[
        ("cm²", "square centimeter"),
        ("cm2", "square centimeter"),
        ("m²", "square meter"),
        ("m2", "square meter"),
        ("cm", "centimeter"),
        ("km", "kilometer"),
        ("kg", "kilogram"),
        ("hr", "hour"),
        ("m", "meter"),
        ("s", "second"),
        ("h", "hour"),
    ];
    for (d, words) in DENOMS {
        let len = d.chars().count();
        let matches = chars
            .get(i + 1..i + 1 + len)
            .is_some_and(|sl| sl.iter().copied().eq(d.chars()));
        let end_ok = chars.get(i + 1 + len).is_none_or(|c| !c.is_alphanumeric());
        if matches && end_ok {
            return Some((len + 1, words));
        }
    }
    None
}

/// Title / Latin abbreviations (longer keys first).
fn expand_abbreviations(s: &str) -> String {
    // (pattern, replacement) — match as whole token with optional trailing .
    const ABBR: &[(&str, &str)] = &[
        ("Mrs.", "Missus"),
        ("Mrs", "Missus"),
        ("Mr.", "Mister"),
        ("Mr", "Mister"),
        ("Ms.", "Miz"),
        ("Ms", "Miz"),
        ("Dr.", "Doctor"),
        ("Dr", "Doctor"),
        ("Prof.", "Professor"),
        ("Prof", "Professor"),
        ("Sr.", "Senior"),
        ("Jr.", "Junior"),
        ("vs.", "versus"),
        ("vs", "versus"),
        ("etc.", "et cetera"),
        ("etc", "et cetera"),
        ("e.g.", "for example"),
        ("i.e.", "that is"),
        ("approx.", "approximately"),
        ("Dept.", "Department"),
        ("Inc.", "Incorporated"),
        ("Ltd.", "Limited"),
        ("Vol.", "volume"),
        ("vol.", "volume"),
        ("Ch.", "chapter"),
        ("ch.", "chapter"),
        ("pp.", "pages"),
        // "No." / "Nos." only when followed by a digit — see expand_no_number_abbrev.
        // Bare "No." is the word no + period (must not become “number”).
        ("Fig.", "figure"),
        ("fig.", "figure"),
        ("Eq.", "equation"),
        ("U.S.", "U S"),
        ("U.K.", "U K"),
        ("U.S.A.", "U S A"),
        ("Ph.D.", "P H D"),
        ("M.D.", "M D"),
        ("a.m.", "A M"),
        ("p.m.", "P M"),
        ("A.M.", "A M"),
        ("P.M.", "P M"),
    ];

    let mut pairs: Vec<(&str, &str)> = ABBR.to_vec();
    pairs.sort_by_key(|b| std::cmp::Reverse(b.0.len()));
    let mut out = replace_abbrev_tokens(s, &pairs);
    // Context-sensitive: "No. 3" → number, but "No." alone stays “No.”
    out = expand_no_number_abbrev(&out);
    out = expand_saint_street(&out);
    out
}

/// `St.` before a capitalised name → “Saint” (`St. Paul`); after a capitalised
/// name or a number → “Street” (`Main St.`, `5th St.`). Otherwise unchanged.
fn expand_saint_street(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(s.len() + 16);
    let mut i = 0;
    while i < n {
        let is_st = i + 3 <= n
            && chars[i] == 'S'
            && chars[i + 1] == 't'
            && chars[i + 2] == '.'
            && (i == 0 || !chars[i - 1].is_alphanumeric());
        if is_st {
            // Only a word in the same sentence counts (not the end of the previous line).
            let tail = out.trim_end_matches([' ', '\t']);
            let same_sentence =
                !tail.is_empty() && !tail.ends_with(['\n', '.', '!', '?', ':', ';']);
            let prev: String = if same_sentence { tail.trim_end() } else { "" }
                .chars()
                .rev()
                .take_while(|c| c.is_alphanumeric())
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            let next = chars[i + 3..].iter().copied().find(|c| !c.is_whitespace());
            // Ordinals were already spelled out ("5th" → "fifth") by this point.
            let ordinal_word = ["th", "st", "nd", "rd"].iter().any(|e| prev.ends_with(e))
                && prev.chars().all(|c| c.is_ascii_lowercase())
                && prev.len() >= 4;
            let prev_is_name_or_number = ordinal_word
                || prev
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_uppercase() || c.is_ascii_digit());
            let next_is_name = next.is_some_and(char::is_uppercase);
            if prev_is_name_or_number {
                out.push_str("Street");
                // Keep the full stop only when it ends the sentence ("on Main St.").
                if !next_is_name && next.is_none_or(|c| c.is_uppercase() || c == '\n') {
                    out.push('.');
                }
                i += 3;
                continue;
            }
            if next_is_name {
                out.push_str("Saint");
                i += 3;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// `No. 12` / `Nos. 3–5` → “number …” / “numbers …”.
/// Plain `No.` (sentence “No.”) is left alone so it is not spoken as “number”.
fn expand_no_number_abbrev(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(s.len() + 8);
    let mut i = 0;
    // Longer forms first
    const PATS: &[(&str, &str)] = &[
        ("Nos.", "numbers"),
        ("nos.", "numbers"),
        ("No.", "number"),
        ("no.", "number"),
    ];
    while i < n {
        let boundary_before = i == 0 || !chars[i - 1].is_ascii_alphanumeric();
        let mut matched = false;
        if boundary_before {
            for &(pat, rep) in PATS {
                let after = i + pat.len();
                if !chars
                    .get(i..after)
                    .is_some_and(|w| w.iter().copied().eq(pat.chars()))
                {
                    continue;
                }
                // Require a digit after optional space (No. 3 / No.12)
                let mut k = after;
                while k < n && chars[k].is_whitespace() {
                    k += 1;
                }
                if k < n && chars[k].is_ascii_digit() {
                    push_token(&mut out, rep);
                    out.push_str(&chars[after..k].iter().collect::<String>());
                    i = k;
                    matched = true;
                    break;
                }
            }
        }
        if matched {
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Replace whole-token, case-sensitive abbreviations in one pass. `pairs`
/// must be sorted longest pattern first so `U.S.A.` wins over `U.S.`.
fn replace_abbrev_tokens(s: &str, pairs: &[(&str, &str)]) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(s.len() + 8);
    let mut i = 0;
    'scan: while i < n {
        if i == 0 || !chars[i - 1].is_ascii_alphanumeric() {
            for &(pat, rep) in pairs {
                let len = pat.chars().count();
                let after = i + len;
                let hit = chars
                    .get(i..after)
                    .is_some_and(|w| w.iter().copied().eq(pat.chars()));
                if hit && (after >= n || !chars[after].is_ascii_alphanumeric()) {
                    push_token(&mut out, rep);
                    i = after;
                    continue 'scan;
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// `Chapter XIV` / `Part III` → arabic for speech.
fn expand_roman_in_context(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(s.len() + 8);
    let mut i = 0;
    const CONTEXTS: &[&str] = &[
        "chapter", "part", "section", "book", "act", "volume", "appendix", "scene", "canto",
        "verse",
    ];
    while i < n {
        if chars[i].is_ascii_alphabetic() && (i == 0 || !chars[i - 1].is_alphanumeric()) {
            let mut j = i;
            while j < n && chars[j].is_ascii_alphabetic() {
                j += 1;
            }
            let word: String = chars[i..j].iter().collect::<String>().to_ascii_lowercase();
            if !CONTEXTS.contains(&word.as_str()) {
                out.extend(&chars[i..j]);
                i = j;
                continue;
            }
            {
                let ctx_start = i;
                out.extend(&chars[i..j]);
                i = j;
                // skip whitespace
                while i < n && chars[i].is_whitespace() {
                    out.push(chars[i]);
                    i += 1;
                }
                // `Appendix A`, `Appendix D`, `Part B`, `Section C`: a lettered label,
                // said as the letter name. Not a numeral (`Appendix D` ≠ “500”), and not
                // the article “a” (`Appendix A- FOLLOW THE RABBIT`).
                const LETTER_CONTEXTS: &[&str] = &["appendix", "part", "section"];
                let capital_context = chars[ctx_start].is_uppercase();
                if capital_context
                    && LETTER_CONTEXTS.contains(&word.as_str())
                    && i < n
                    && chars[i].is_ascii_uppercase()
                    && chars
                        .get(i + 1)
                        .is_none_or(|c| !c.is_alphanumeric() && *c != '\'' && *c != '’')
                {
                    let letter = chars[i];
                    let next_word = chars[i + 1..].iter().copied().find(|c| !c.is_whitespace());
                    // `Part I` / `Part V` / `Section X` stay roman (only appendices are
                    // always lettered); `Part A Tale…` reads on as an article.
                    let roman_like = matches!(letter, 'I' | 'V' | 'X') && word != "appendix";
                    let article_like =
                        letter == 'A' && next_word.is_some_and(|c| c.is_alphabetic());
                    if !roman_like && !article_like {
                        out.push_str(letter_spoken_name(letter));
                        i += 1;
                        continue;
                    }
                }
                // Roman token: uppercase and canonically written. A lone `I` only
                // after a capitalised context word (`Chapter I`, not `book I read`).
                if i < n && is_roman_char(chars[i]) {
                    let mut k = i;
                    while k < n && is_roman_char(chars[k]) {
                        k += 1;
                    }
                    let boundary = k >= n || !chars[k].is_ascii_alphanumeric();
                    let roman: String = chars[i..k].iter().collect();
                    let pronoun_risk = roman == "I" && !capital_context;
                    if boundary && !pronoun_risk {
                        if let Some(val) = parse_roman(&roman).filter(|v| to_roman(*v) == roman) {
                            if (1..=3999).contains(&val) {
                                out.push_str(&val.to_string());
                                out.push(' ');
                                i = k;
                                continue;
                            }
                        }
                    }
                }
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// `Henry VIII` → “Henry the eighth”, `World War II` → “World War 2”,
/// `Apollo XI` → “Apollo 11”.
///
/// Only an uppercase, canonically written numeral (2–40, or `I` after “War”)
/// right after a capitalised word. `I MIX paint` / `Washington DC` / `size XL`
/// are left alone (value too big, or no capitalised name before it).
fn expand_roman_after_names(s: &str) -> String {
    const REGNAL: &[&str] = &[
        "henry",
        "edward",
        "george",
        "william",
        "richard",
        "charles",
        "james",
        "elizabeth",
        "mary",
        "anne",
        "victoria",
        "louis",
        "philip",
        "john",
        "paul",
        "pius",
        "leo",
        "gregory",
        "benedict",
        "innocent",
        "clement",
        "alexander",
        "peter",
        "ivan",
        "frederick",
        "napoleon",
        "ferdinand",
        "victor",
        "emmanuel",
        "alfonso",
        "carlos",
        "juan",
        "felipe",
        "pedro",
        "ramesses",
        "ramses",
        "amenhotep",
        "thutmose",
        "ptolemy",
        "constantine",
        "justinian",
        "otto",
        "rudolf",
        "wilhelm",
        "gustav",
        "christian",
        "harald",
        "olaf",
        "haakon",
        "stephen",
        "catherine",
        "nicholas",
        "boniface",
        "urban",
        "sixtus",
        "julius",
        "hadrian",
        "martin",
        "celestine",
        "honorius",
        "eugene",
        "francis",
        "joseph",
        "isabella",
        "sancho",
        "baldwin",
        "malcolm",
        "david",
        "robert",
        "duncan",
        "kenneth",
        "alfred",
        "harold",
        "edmund",
    ];
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(s.len() + 16);
    let mut i = 0;
    while i < n {
        let at_word = chars[i].is_ascii_uppercase() && (i == 0 || !chars[i - 1].is_alphanumeric());
        if at_word {
            let mut k = i;
            while k < n && matches!(chars[k], 'I' | 'V' | 'X' | 'L') {
                k += 1;
            }
            let boundary = k >= n || !chars[k].is_alphanumeric();
            if k > i && boundary {
                let roman: String = chars[i..k].iter().collect();
                // Previous word (one space back), as typed.
                // Previous word (one space back), as typed.
                let single_space = i >= 1 && chars[i - 1] == ' ' && (i < 2 || chars[i - 2] != ' ');
                let mut w_end = i;
                while w_end > 0 && chars[w_end - 1] == ' ' {
                    w_end -= 1;
                }
                let mut w_start = w_end;
                while w_start > 0 && chars[w_start - 1].is_alphabetic() {
                    w_start -= 1;
                }
                let prev_word: String = chars[w_start..w_end].iter().collect();
                let capitalised = prev_word.chars().count() >= 2
                    && prev_word.chars().next().is_some_and(char::is_uppercase)
                    && prev_word.chars().skip(1).any(char::is_lowercase);
                let lower = prev_word.to_lowercase();
                let after_war = lower == "war";
                let val = parse_roman(&roman).filter(|v| to_roman(*v) == roman);
                let ok_len = roman.len() >= 2 || after_war;
                if let (Some(v), true, true, true) = (val, single_space, capitalised, ok_len) {
                    if (1..=40).contains(&v) {
                        let spoken = if REGNAL.contains(&lower.as_str()) {
                            ordinal_word(v).map(|o| format!("the {o}"))
                        } else {
                            Some(v.to_string())
                        };
                        if let Some(sp) = spoken {
                            out.push_str(&sp);
                            i = k;
                            continue;
                        }
                    }
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Canonical Roman numeral for `n` (1–3999).
fn to_roman(mut n: u32) -> String {
    const T: &[(u32, &str)] = &[
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut s = String::new();
    for &(v, r) in T {
        while n >= v {
            s.push_str(r);
            n -= v;
        }
    }
    s
}

/// Hyphenated digit groups that are not ranges:
/// - ISO dates `2024-01-15` → spoken date,
/// - phone numbers / ISBNs / serials with 3+ groups (`555-123-4567`,
///   `978-3-16-148410-0`) or `555-1234` → digits read one by one, groups paused.
///
/// Plain two-group ranges (`10-20`, `1990-2000`) are left for the “to” rule.
fn expand_digit_groups(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(s.len() + 16);
    let mut i = 0;
    while i < n {
        let start_ok = chars[i].is_ascii_digit() && (i == 0 || !chars[i - 1].is_alphanumeric());
        if start_ok {
            let mut k = i;
            while k < n
                && (chars[k].is_ascii_digit()
                    || (chars[k] == '-'
                        && k > i
                        && chars[k - 1].is_ascii_digit()
                        && chars.get(k + 1).is_some_and(char::is_ascii_digit)))
            {
                k += 1;
            }
            let end_ok = k >= n || !chars[k].is_alphanumeric();
            let token: String = chars[i..k].iter().collect();
            let groups: Vec<&str> = token.split('-').collect();
            if end_ok && groups.len() >= 2 {
                let lens: Vec<usize> = groups.iter().map(|g| g.len()).collect();
                if lens == [4, 2, 2] {
                    let y: u32 = groups[0].parse().unwrap_or(0);
                    let m: u32 = groups[1].parse().unwrap_or(0);
                    let d: u32 = groups[2].parse().unwrap_or(0);
                    if let Some(date) = format_slash_date(m, d, y) {
                        push_token(&mut out, &date);
                        i = k;
                        continue;
                    }
                }
                // `04-01-2026` / `25-12-2024` / `12-25-2024`: a date, not a serial.
                // Unambiguous order → month name; ambiguous (both ≤ 12, UK or
                // US?) → the numbers as written, year spoken as a year.
                if groups.len() == 3
                    && (1..=2).contains(&lens[0])
                    && (1..=2).contains(&lens[1])
                    && lens[2] == 4
                {
                    let a: u32 = groups[0].parse().unwrap_or(0);
                    let b: u32 = groups[1].parse().unwrap_or(0);
                    let y: u32 = groups[2].parse().unwrap_or(0);
                    let spoken = match (a > 12, b > 12) {
                        (true, false) => format_slash_date(b, a, y),
                        (false, true) => format_slash_date(a, b, y),
                        _ if (1..=12).contains(&a)
                            && (1..=12).contains(&b)
                            && (1000..=9999).contains(&y) =>
                        {
                            Some(format!("{a}, {b}, {}", year_words(y)))
                        }
                        _ => None,
                    };
                    if let Some(sp) = spoken {
                        push_token(&mut out, &sp);
                        i = k;
                        continue;
                    }
                }
                let phone7 = lens == [3, 4];
                if groups.len() >= 3 || phone7 {
                    let spoken: Vec<String> = groups
                        .iter()
                        .map(|g| g.chars().map(String::from).collect::<Vec<_>>().join(" "))
                        .collect();
                    push_token(&mut out, &spoken.join(", "));
                    i = k;
                    continue;
                }
            }
            out.push_str(&token);
            i = k;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

fn is_roman_char(c: char) -> bool {
    matches!(c, 'I' | 'V' | 'X' | 'L' | 'C' | 'D' | 'M')
}

fn parse_roman(s: &str) -> Option<u32> {
    let s = s.to_ascii_uppercase();
    if s.is_empty()
        || !s
            .chars()
            .all(|c| matches!(c, 'I' | 'V' | 'X' | 'L' | 'C' | 'D' | 'M'))
    {
        return None;
    }
    let val = |c: char| -> u32 {
        match c {
            'I' => 1,
            'V' => 5,
            'X' => 10,
            'L' => 50,
            'C' => 100,
            'D' => 500,
            'M' => 1000,
            _ => 0,
        }
    };
    let bytes: Vec<char> = s.chars().collect();
    let mut total = 0i32;
    let mut i = 0;
    while i < bytes.len() {
        let cur = val(bytes[i]) as i32;
        let next = if i + 1 < bytes.len() {
            val(bytes[i + 1]) as i32
        } else {
            0
        };
        if cur < next {
            total += next - cur;
            i += 2;
        } else {
            total += cur;
            i += 1;
        }
    }
    if total <= 0 {
        None
    } else {
        Some(total as u32)
    }
}

/// ALL-CAPS tokens: dictionary words are read as words (headings, shouting);
/// acronyms become letter words (`USB` → “you-ess-bee”, `AI` → “eigh eye”),
/// hyphen-joined so Kokoro says them as one quick unit. Word acronyms
/// (`NASA`) stay whole; `OK` → “okay”.
fn expand_acronyms_and_dict(s: &str) -> String {
    const DICT: &[(&str, &str)] = &[("OK", "okay")];
    // Real words that are usually acronyms when they stand alone in normal text
    // ("the US", "IT team", "your ID", "the WHO"). Inside an ALL-CAPS run
    // ("LET US GO", "WHO ARE YOU") they are read as words.
    const ACRONYM_UNLESS_IN_CAPS_RUN: &[&str] = &[
        "US", "IT", "ID", "AM", "PM", "OR", "ER", "WHO", "LED", "CAT", "ACE", "PIN", "SIM", "SAR",
    ];
    // Spoken as a whole word — do not letter-spell.
    const WORD_ACRONYMS: &[&str] = &[
        "NASA", "NATO", "UNESCO", "UNICEF", "ASCII", "COVID", "LASER", "RADAR", "SONAR", "SCUBA",
        "AIDS", "OPEC", "FIFA", "NBA", "NFL", "MLS",
    ];

    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(s.len() + 16);
    let mut i = 0;
    while i < n {
        if chars[i].is_ascii_alphabetic() {
            let mut j = i;
            while j < n && chars[j].is_ascii_alphanumeric() {
                j += 1;
            }
            let token: String = chars[i..j].iter().collect();
            let upper = token.to_ascii_uppercase();
            let all_caps = token
                .chars()
                .all(|c| !c.is_ascii_alphabetic() || c.is_ascii_uppercase())
                && token.chars().any(|c| c.is_ascii_alphabetic())
                && token.chars().all(|c| c.is_ascii_alphabetic()); // letters only, not USB2
            let boundary_before = i == 0 || !chars[i - 1].is_ascii_alphanumeric();
            let boundary_after = j >= n || !chars[j].is_ascii_alphanumeric();

            if boundary_before && boundary_after && all_caps {
                // ALL-CAPS *words* (headings, title pages, shouting) are read as words:
                // "PRIDE AND PREJUDICE", "THE END", "STOP!". Acronyms that are not
                // dictionary words (FBI, EMF, USB) are still spelled below.
                let in_run = caps_run_neighbour(&chars, i, j);
                let ambiguous = ACRONYM_UNLESS_IN_CAPS_RUN.contains(&upper.as_str());
                let is_word = !WORD_ACRONYMS.contains(&upper.as_str())
                    && !DICT.iter().any(|(k, _)| *k == upper.as_str())
                    && is_dictionary_word(&token)
                    && (in_run || !ambiguous)
                    // Lone "A"/"I" keep their own handling (letter lists, pronoun).
                    && (token.len() >= 2 || in_run);
                if is_word {
                    // In place, no padding: keeps `"STOP!"` as `"stop!"`.
                    out.push_str(&token.to_lowercase());
                    i = j;
                    continue;
                }
                if let Some((_, spoken)) = DICT.iter().find(|(k, _)| *k == upper.as_str()) {
                    push_token(&mut out, spoken);
                    i = j;
                    continue;
                }
                if WORD_ACRONYMS.contains(&upper.as_str()) {
                    // keep as written (e.g. NASA)
                    for &c in &chars[i..j] {
                        out.push(c);
                    }
                    out.push(' ');
                    i = j;
                    continue;
                }
                // 2–5 letter ALLCAPS → spoken letter words at normal pace
                if (2..=5).contains(&token.len()) {
                    push_token(&mut out, &spell_acronym_as_words(&token));
                    i = j;
                    continue;
                }
            }
            // Plural acronym `IDs`, `SUVs`, `SSIDs` → “eye dees”, “ess-you-vees”.
            let stem = token.strip_suffix('s').unwrap_or("");
            let stem_caps =
                (2..=5).contains(&stem.len()) && stem.chars().all(|c| c.is_ascii_uppercase());
            if boundary_before
                && boundary_after
                && stem_caps
                && !WORD_ACRONYMS.contains(&stem)
                && (!is_dictionary_word(stem) || ACRONYM_UNLESS_IN_CAPS_RUN.contains(&stem))
            {
                let mut spelled = spell_acronym_as_words(stem);
                spelled.push_str(if spelled.ends_with(['s', 'x', 'h']) {
                    "es"
                } else {
                    "s"
                });
                push_token(&mut out, &spelled);
                i = j;
                continue;
            }
            for &c in &chars[i..j] {
                out.push(c);
            }
            i = j;
            continue;
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Is the word next to `chars[i..j]` (before or after, same sentence) also an
/// ALL-CAPS dictionary word? That marks a heading or shouted line.
fn caps_run_neighbour(chars: &[char], i: usize, j: usize) -> bool {
    let is_caps_word = |a: usize, b: usize| -> bool {
        b > a
            && b - a >= 2
            && chars[a..b].iter().all(char::is_ascii_uppercase)
            && is_dictionary_word(&chars[a..b].iter().collect::<String>())
    };
    // Previous word: skip spaces and soft punctuation, stop at sentence breaks.
    let mut a = i;
    while a > 0 && matches!(chars[a - 1], ' ' | ',' | '"' | '\'' | '-' | ':' | ';') {
        a -= 1;
    }
    let mut a0 = a;
    while a0 > 0 && chars[a0 - 1].is_ascii_alphabetic() {
        a0 -= 1;
    }
    if is_caps_word(a0, a) {
        return true;
    }
    let n = chars.len();
    let mut b = j;
    while b < n && matches!(chars[b], ' ' | ',' | '"' | '\'' | '-' | ':' | ';') {
        b += 1;
    }
    let mut b1 = b;
    while b1 < n && chars[b1].is_ascii_alphabetic() {
        b1 += 1;
    }
    is_caps_word(b, b1)
}

/// English dictionary word (misaki-rs's gold lexicon, used only as a word list;
/// acronyms like FBI/EMF/TV live
/// only in its "silver" list or not at all). Loaded once, on first ALL-CAPS token.
fn is_dictionary_word(token: &str) -> bool {
    use std::collections::HashSet;
    use std::sync::OnceLock;
    static WORDS: OnceLock<HashSet<String>> = OnceLock::new();
    let words = WORDS.get_or_init(|| {
        misaki_rs::data::load_us_gold()
            .into_keys()
            .filter(|k| k.chars().all(|c| c.is_ascii_lowercase()))
            .collect()
    });
    let w = token.to_ascii_lowercase();
    if words.contains(&w) {
        return true;
    }
    // Inflections the lexicon stores only as stems: ARRIVED, DIED, GOES, RUNNING.
    let known = |s: &str| s.len() >= 2 && words.contains(s);
    let undouble = |s: &str| {
        let b = s.as_bytes();
        (b.len() >= 3 && b[b.len() - 1] == b[b.len() - 2]).then(|| s[..s.len() - 1].to_string())
    };
    for suf in ["'s", "es", "s", "ed", "d", "ing"] {
        if let Some(stem) = w.strip_suffix(suf) {
            if known(stem)
                || known(&format!("{stem}e"))
                || undouble(stem).is_some_and(|u| known(&u))
            {
                return true;
            }
            if suf == "es" || suf == "ed" {
                if let Some(y) = stem.strip_suffix('i') {
                    if known(&format!("{y}y")) {
                        return true; // CRIES, TRIED
                    }
                }
            }
        }
    }
    false
}

/// Map ALLCAPS letters to spoken English letter names (American).
///
/// - 3–5 letters: hyphen join (`ee-em-ef`, `you-ess-bee`) for snappier pace.
/// - 2 letters: **space** join (`ar ef`, `ee ef`). Hyphen after letter R
///   (`ar-ef`) makes espeak glue into “are-ref” / “are ref”.
fn spell_acronym_as_words(token: &str) -> String {
    let names: Vec<&str> = token
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .map(|c| letter_spoken_name(c))
        .filter(|s| !s.is_empty())
        .collect();
    let sep = if names.len() <= 2 { " " } else { "-" };
    names.join(sep)
}

fn letter_spoken_name(c: char) -> &'static str {
    match c.to_ascii_uppercase() {
        // Not "ay": espeak pronounces “ay” as /aɪ/ (same as letter I / “eye”).
        // “eigh” (as in eight/weigh) reliably yields letter-A /eɪ/.
        'A' => "eigh",
        'B' => "bee",
        'C' => "see",
        'D' => "dee",
        'E' => "ee",
        // Not "eff": espeak says ,i:;,Ef'Ef (slow mangled form). "ef" → clean 'Ef.
        'F' => "ef",
        'G' => "jee",
        'H' => "aitch",
        'I' => "eye",
        'J' => "jay",
        'K' => "kay",
        'L' => "el",
        'M' => "em",
        'N' => "en",
        'O' => "oh",
        'P' => "pee",
        'Q' => "cue",
        'R' => "ar",
        'S' => "ess",
        'T' => "tee",
        'U' => "you",
        'V' => "vee",
        'W' => "double-you",
        'X' => "ex",
        'Y' => "why",
        'Z' => "zee",
        _ => "",
    }
}

/// `copy-paste`, `early-warning` → “copy paste”, “early warning”. espeak reads a
/// hyphenated compound as one word, and after a first part ending in `y` it can
/// clip the final “ee” to a short “i” (kˈɑːpɪpˈeɪst, ˈɜːlɪwˈɔːɹnɪŋ). With a space
/// each part keeps its own pronunciation. Only when both parts are dictionary
/// words; other `-y` compounds (`ready-made`, `heavy-duty`) sound the same either way.
fn split_y_hyphen_compounds(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < n {
        if chars[i] == '-'
            && i >= 3
            && chars[i - 1] == 'y'
            && chars.get(i + 1).is_some_and(char::is_ascii_alphabetic)
        {
            let mut a = i;
            while a > 0 && chars[a - 1].is_ascii_alphabetic() {
                a -= 1;
            }
            let mut b = i + 1;
            while b < n && chars[b].is_ascii_alphabetic() {
                b += 1;
            }
            let left: String = chars[a..i].iter().collect();
            let right: String = chars[i + 1..b].iter().collect();
            let bounded = (a == 0 || !chars[a - 1].is_alphanumeric())
                && chars.get(b).is_none_or(|c| !c.is_alphanumeric());
            if bounded
                && left.len() >= 3
                && right.len() >= 2
                && is_dictionary_word(&left)
                && is_dictionary_word(&right)
            {
                out.push(' ');
                i += 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// `cope(d)`, `item(s)`, `box(es)`, `use(ed)` → suffix joined, brackets dropped.
/// Only after a word of 2+ letters, so `f(x)` and list markers like `a(i)` stay.
fn join_paren_suffixes(s: &str) -> String {
    const SUFFIXES: [&str; 4] = ["s", "es", "d", "ed"];
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < n {
        if chars[i] == '(' && i >= 2 && chars[i - 1].is_alphabetic() && chars[i - 2].is_alphabetic()
        {
            let mut j = i + 1;
            while j < n && chars[j].is_ascii_lowercase() {
                j += 1;
            }
            let suffix: String = chars[i + 1..j].iter().collect();
            let closes = j < n && chars[j] == ')';
            let after_ok = chars.get(j + 1).is_none_or(|c| !c.is_alphanumeric());
            if closes && after_ok && SUFFIXES.contains(&suffix.as_str()) {
                out.push_str(&suffix);
                i = j + 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// A slash between words (`before/after`, `time/location`, `O2/Vodafone`,
/// `lac/ TAC`) becomes a plain word gap, so espeak never says “slash”.
/// Pronoun pairs read naturally with “or” (`he/she` → “he or she”).
fn expand_word_slashes(s: &str) -> String {
    const PAIRS: [(&str, &str); 5] = [
        ("he/she", "he or she"),
        ("she/he", "she or he"),
        ("his/her", "his or her"),
        ("him/her", "him or her"),
        ("s/he", "she or he"),
    ];
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    'walk: while i < n {
        if chars[i].is_ascii_alphabetic() && (i == 0 || !chars[i - 1].is_alphanumeric()) {
            for (pat, rep) in PAIRS {
                let len = pat.chars().count();
                let end_ok = chars.get(i + len).is_none_or(|c| !c.is_alphanumeric());
                if end_ok && matches_ignore_case_at(&chars, i, pat) {
                    out.push_str(rep);
                    i += len;
                    continue 'walk;
                }
            }
        }
        if chars[i] == '/' {
            let prev = if i > 0 { Some(chars[i - 1]) } else { None };
            let next = chars.get(i + 1).copied();
            let prev_word = prev.is_some_and(char::is_alphanumeric);
            let next_word = next.is_some_and(char::is_alphanumeric);
            let both_digits = prev.is_some_and(|c| c.is_ascii_digit())
                && next.is_some_and(|c| c.is_ascii_digit());
            let next_spaced_word = next.is_some_and(char::is_whitespace)
                && chars.get(i + 2).is_some_and(|c| c.is_alphanumeric());
            // Spaced separator `Wi-Fi / 4G`, `EF / RF / magnetic` → short pause.
            let spaced = prev.is_some_and(|c| c == ' ' || c == '\t')
                && next.is_some_and(|c| c == ' ' || c == '\t')
                && out
                    .trim_end()
                    .ends_with(|c: char| c.is_alphanumeric() || c == ')' || c == '"')
                && chars[i + 1..]
                    .iter()
                    .find(|c| !c.is_whitespace())
                    .is_some_and(|c| c.is_alphanumeric() || *c == '"' || *c == '(');
            if spaced {
                while out.ends_with([' ', '\t']) {
                    out.pop();
                }
                out.push(',');
                i += 1;
                continue;
            }
            if !both_digits && prev_word && (next_word || next_spaced_word) {
                // Acronym pair (`LAC/TAC`, `UHF/EF`): spell both sides here, or the
                // caps pass reads a dictionary-word half (`LAC`) as the word “lac”.
                let mut a = i;
                while a > 0 && chars[a - 1].is_ascii_alphanumeric() {
                    a -= 1;
                }
                let mut b = i + 1;
                while b < n && chars[b].is_whitespace() {
                    b += 1;
                }
                let b0 = b;
                while b < n && chars[b].is_ascii_alphanumeric() {
                    b += 1;
                }
                let left: String = chars[a..i].iter().collect();
                let right: String = chars[b0..b].iter().collect();
                let caps = |t: &str| {
                    (2..=5).contains(&t.len()) && t.chars().all(|c| c.is_ascii_uppercase())
                };
                if caps(&left)
                    && caps(&right)
                    && !(is_dictionary_word(&left) && is_dictionary_word(&right))
                {
                    out.truncate(out.len() - left.len());
                    out.push_str(&spell_acronym_as_words(&left));
                    out.push(' ');
                    out.push_str(&spell_acronym_as_words(&right));
                    i = b;
                    continue;
                }
                if next_word {
                    out.push(' ');
                }
                i += 1;
                continue;
            }
            // Table leftovers: `Catcher /`, `Notes /Room` — a slash touching
            // whitespace or a line end is never meant to be read aloud.
            if prev.is_none_or(char::is_whitespace) || next.is_none_or(char::is_whitespace) {
                push_space(&mut out);
                i += 1;
                continue;
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// Letter+digit product codes and test IDs attached to a name.
///
/// - `EMF-390` / `EMF390` → casual model reading (“three ninety”)
/// - `A01` / `B03` / `Z01` → letter name + **each digit** (“eigh zero one”)
///   so leading zeros are not dropped (`A01` must not become “A one”)
/// - `EM100` → all-caps letters spelled (“ee em one hundred”); `O2` → “oh two”
///
/// Ordinary counts like “390 people” are unchanged (no letter glued on).
fn expand_model_numbers(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(s.len() + 24);
    let mut i = 0;
    while i < n {
        // Start of a letter run at a word boundary?
        if chars[i].is_ascii_alphabetic() && (i == 0 || !chars[i - 1].is_ascii_alphanumeric()) {
            let mut j = i;
            while j < n && chars[j].is_ascii_alphabetic() {
                j += 1;
            }
            let letters: String = chars[i..j].iter().collect();
            // optional hyphen before digits: EMF-390
            let mut k = j;
            let mut had_hyphen = false;
            if k < n && chars[k] == '-' {
                had_hyphen = true;
                k += 1;
            }
            if k < n && chars[k].is_ascii_digit() {
                let digit_start = k;
                while k < n && chars[k].is_ascii_digit() {
                    k += 1;
                }
                let digits: String = chars[digit_start..k].iter().collect();
                // trailing letter like F02b → leave whole token alone
                let after_ok = k >= n || !chars[k].is_ascii_alphanumeric();
                // `O2`, `B2`, `G5`: one capital + one digit. espeak reads `O2` as
                // “zero two”, so spell it (“oh two”). Lowercase (`a1`) is left alone.
                let single_cap_digit = digits.len() == 1
                    && letters.len() == 1
                    && letters.chars().all(|c| c.is_ascii_uppercase());
                // `EIR000824`: caps letters + a long or zero-led number = reference code.
                let ref_code = digits.len() >= 5
                    && (2..=4).contains(&letters.len())
                    && letters.chars().all(|c| c.is_ascii_uppercase())
                    && !had_hyphen;
                if after_ok && ref_code {
                    let mut spoken = spell_acronym_as_words(&letters);
                    for d in digits.chars() {
                        spoken.push(' ');
                        spoken.push_str(digit_word(d).unwrap_or(""));
                    }
                    push_token(&mut out, &spoken);
                    i = k;
                    continue;
                }
                if after_ok && ((2..=4).contains(&digits.len()) || single_cap_digit) {
                    // Single-letter IDs (A01, B03) or leading-zero codes → spell
                    // each digit so “0” is “zero”, not swallowed as integer 1.
                    let spell_digits = letters.chars().count() == 1 || digits.starts_with('0');
                    if spell_digits {
                        let mut spoken = String::new();
                        for ch in letters.chars() {
                            let name = letter_spoken_name(ch);
                            if !name.is_empty() {
                                if !spoken.is_empty() {
                                    spoken.push(' ');
                                }
                                spoken.push_str(name);
                            }
                        }
                        for d in digits.chars() {
                            if let Some(w) = digit_word(d) {
                                if !spoken.is_empty() {
                                    spoken.push(' ');
                                }
                                spoken.push_str(w);
                            }
                        }
                        if !spoken.is_empty() {
                            push_token(&mut out, &spoken);
                            i = k;
                            continue;
                        }
                    } else if let Ok(num) = digits.parse::<u32>() {
                        if let Some(num_words) = model_number_words(num) {
                            // Keep multi-letter name, drop hyphen, speak number casually
                            if !out.is_empty() && !out.ends_with([' ', '\n', '\t']) {
                                out.push(' ');
                            }
                            // All-caps names are letter codes: spell them here
                            // (`EM100` → “ee em one hundred”). Left for the acronym
                            // pass, a short one like `EM` reads as the word “em”.
                            let caps = (2..=4).contains(&letters.len())
                                && letters.chars().all(|c| c.is_ascii_uppercase());
                            if caps {
                                out.push_str(&spell_acronym_as_words(&letters));
                            } else {
                                out.push_str(&letters);
                            }
                            out.push(' ');
                            out.push_str(&num_words);
                            out.push(' ');
                            i = k;
                            continue;
                        }
                    }
                }
            }
        }

        out.push(chars[i]);
        i += 1;
    }
    out
}

fn digit_word(c: char) -> Option<&'static str> {
    c.to_digit(10).map(ones_word)
}

/// Casual model-number reading (not full cardinal “three hundred ninety”).
fn model_number_words(n: u32) -> Option<String> {
    match n {
        0..=99 => Some(two_digit_words(n)),
        100..=999 => {
            let h = n / 100;
            let r = n % 100;
            let h_word = ones_word(h);
            if h_word.is_empty() {
                return None;
            }
            if r == 0 {
                Some(format!("{h_word} hundred"))
            } else if r < 10 {
                Some(format!("{h_word} oh {}", ones_word(r)))
            } else {
                Some(format!("{h_word} {}", two_digit_words(r)))
            }
        }
        // 4-digit models: 1200 → “twelve hundred”; 2024 → “twenty twenty four”
        1000..=9999 => {
            let hi = n / 100;
            let lo = n % 100;
            if lo == 0 {
                Some(format!("{} hundred", two_digit_words(hi)))
            } else if lo < 10 {
                Some(format!("{} oh {}", two_digit_words(hi), ones_word(lo)))
            } else {
                Some(format!("{} {}", two_digit_words(hi), two_digit_words(lo)))
            }
        }
        _ => None,
    }
}

/// Rewrite `$5` / `$5.99` / `€10` etc. to spoken order (amount, then unit).
///
/// Pattern used by Kokoro G2P `flip_money` and KittenTTS currency expand:
/// number first, currency word after, optional fractional minor unit.
/// Digits are kept (espeak reads them); we do not full-document num2words.
fn expand_currency_amounts(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(s.len() + 16);
    let mut i = 0;

    while i < n {
        let ch = chars[i];
        let unit = match ch {
            '$' => Some(CurrencyUnit {
                major_one: "dollar",
                major_many: "dollars",
                minor_one: Some("cent"),
                minor_many: Some("cents"),
            }),
            '€' => Some(CurrencyUnit {
                major_one: "euro",
                major_many: "euros",
                minor_one: Some("cent"),
                minor_many: Some("cents"),
            }),
            '£' => Some(CurrencyUnit {
                major_one: "pound",
                major_many: "pounds",
                minor_one: Some("penny"),
                minor_many: Some("pence"),
            }),
            '¥' => Some(CurrencyUnit {
                major_one: "yen",
                major_many: "yen",
                minor_one: None,
                minor_many: None,
            }),
            '₹' => Some(CurrencyUnit {
                major_one: "rupee",
                major_many: "rupees",
                minor_one: None,
                minor_many: None,
            }),
            _ => None,
        };

        if let Some(unit) = unit {
            let mut j = i + 1;
            while j < n && chars[j].is_whitespace() {
                j += 1;
            }
            if j < n && chars[j].is_ascii_digit() {
                // Integer part (allow thousand separators: 1,000)
                let mut major_digits = String::new();
                while j < n {
                    if chars[j].is_ascii_digit() {
                        major_digits.push(chars[j]);
                        j += 1;
                    } else if chars[j] == ',' && j + 1 < n && chars[j + 1].is_ascii_digit() {
                        j += 1; // skip grouping comma
                    } else {
                        break;
                    }
                }

                // Magnitude suffix: $5M, $2.5k, €3bn, $1.2 billion → "2.5 thousand dollars".
                if let Some((decimal, mag, end)) = currency_magnitude(&chars, j) {
                    let whole = if decimal.is_empty() {
                        major_digits.clone()
                    } else {
                        format!("{major_digits}.{decimal}")
                    };
                    push_token(&mut out, &format!("{whole} {mag} {}", unit.major_many));
                    i = end;
                    continue;
                }

                // Optional fractional: .99 or .9
                let mut minor_digits = String::new();
                if j < n
                    && chars[j] == '.'
                    && j + 1 < n
                    && chars[j + 1].is_ascii_digit()
                    && unit.minor_many.is_some()
                {
                    j += 1;
                    while j < n && chars[j].is_ascii_digit() && minor_digits.len() < 2 {
                        minor_digits.push(chars[j]);
                        j += 1;
                    }
                    // Swallow extra fractional digits without speaking them as more cents
                    while j < n && chars[j].is_ascii_digit() {
                        j += 1;
                    }
                }

                if !major_digits.is_empty() {
                    let phrase = format_currency_phrase(&major_digits, &minor_digits, &unit);
                    push_token(&mut out, &phrase);
                    i = j;
                    continue;
                }
            }
        }

        out.push(ch);
        i += 1;
    }
    out
}

/// After the integer part of an amount at `j`: optional `.decimal`, then a
/// magnitude (`k`/`K`, `m`/`M`/`mn`, `b`/`B`/`bn`, `t`/`T`/`tn`, or the words
/// thousand/million/billion/trillion). Returns (decimal digits, word, end index).
fn currency_magnitude(chars: &[char], j: usize) -> Option<(String, &'static str, usize)> {
    let n = chars.len();
    let mut k = j;
    let mut decimal = String::new();
    if k + 1 < n && chars[k] == '.' && chars[k + 1].is_ascii_digit() {
        k += 1;
        while k < n && chars[k].is_ascii_digit() {
            decimal.push(chars[k]);
            k += 1;
        }
    }
    // Optional single space before a spelled-out magnitude word.
    let word_at = |at: usize| -> Option<(&'static str, usize)> {
        let rest: String = chars[at..n.min(at + 9)]
            .iter()
            .collect::<String>()
            .to_ascii_lowercase();
        for (w, m) in [
            ("thousand", "thousand"),
            ("million", "million"),
            ("billion", "billion"),
            ("trillion", "trillion"),
        ] {
            if rest.starts_with(w) {
                let end = at + w.len();
                if end >= n || !chars[end].is_alphanumeric() {
                    return Some((m, end));
                }
            }
        }
        None
    };
    if let Some((m, end)) =
        word_at(k).or_else(|| (k < n && chars[k] == ' ').then(|| word_at(k + 1)).flatten())
    {
        return Some((decimal, m, end));
    }
    // Letter suffixes glued to the number.
    let suffix: String = chars[k..n.min(k + 2)].iter().collect();
    let (mag, len) = match suffix.as_str() {
        s2 if s2.eq_ignore_ascii_case("bn") => ("billion", 2),
        s2 if s2.eq_ignore_ascii_case("mn") => ("million", 2),
        s2 if s2.eq_ignore_ascii_case("tn") => ("trillion", 2),
        _ => match chars.get(k) {
            Some('k' | 'K') => ("thousand", 1),
            Some('m' | 'M') => ("million", 1),
            Some('b' | 'B') => ("billion", 1),
            Some('T') => ("trillion", 1),
            _ => return None,
        },
    };
    let end = k + len;
    if end < n && chars[end].is_alphanumeric() {
        return None; // "$5 Mustang", "$5km"…
    }
    Some((decimal, mag, end))
}

struct CurrencyUnit {
    major_one: &'static str,
    major_many: &'static str,
    minor_one: Option<&'static str>,
    minor_many: Option<&'static str>,
}

fn format_currency_phrase(major_digits: &str, minor_digits: &str, unit: &CurrencyUnit) -> String {
    let major_n: u64 = major_digits.parse().unwrap_or(0);
    let major_word = if major_n == 1 {
        unit.major_one
    } else {
        unit.major_many
    };

    // One fractional digit is tenths: $1.5 → 50 cents.
    let minor_digits = if minor_digits.len() == 1 {
        format!("{minor_digits}0")
    } else {
        minor_digits.to_string()
    };
    let minor_n: u64 = minor_digits.parse().unwrap_or(0);

    if minor_n == 0 {
        return format!("{major_digits} {major_word}");
    }

    let (Some(minor_one), Some(minor_many)) = (unit.minor_one, unit.minor_many) else {
        return format!("{major_digits} {major_word}");
    };
    let minor_word = if minor_n == 1 { minor_one } else { minor_many };
    // Leading zeros dropped: "05" → "5".
    let minor_speak = minor_digits.trim_start_matches('0');
    let minor_speak = if minor_speak.is_empty() {
        "0"
    } else {
        minor_speak
    };

    format!("{major_digits} {major_word} and {minor_speak} {minor_word}")
}

/// Expand capital letters in list-like patterns: `A and B`, `X or Y`, `A B`.
///
/// Does **not** rewrite sentence articles (`A well-known fact`) or the pronoun `I`
/// unless they sit in a letter list.
fn expand_letter_list_capitals(s: &str) -> String {
    // Preserve blank lines *and* single newlines (sentence.\nNext for pauses).
    s.split("\n\n")
        .map(|block| {
            block
                .split('\n')
                .map(expand_letter_list_capitals_block)
                .collect::<Vec<_>>()
                .join("\n")
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn expand_letter_list_capitals_block(s: &str) -> String {
    let words: Vec<&str> = s.split_whitespace().collect();
    if words.is_empty() {
        return String::new();
    }
    let n = words.len();
    let mut expand = vec![false; n];

    let is_single_cap = |w: &str| -> bool {
        let mut chs = w.chars();
        matches!((chs.next(), chs.next()), (Some(c), None) if c.is_ascii_uppercase())
    };
    let is_connector =
        |w: &str| -> bool { matches!(w.to_ascii_lowercase().as_str(), "and" | "or" | "&" | "+") };

    for i in 0..n {
        if !is_single_cap(words[i]) {
            continue;
        }
        // A and B  /  A or B
        if i + 2 < n && is_connector(words[i + 1]) && is_single_cap(words[i + 2]) {
            expand[i] = true;
            expand[i + 2] = true;
        }
        // B is second half (A and B) — also mark when looking from B
        if i >= 2 && is_connector(words[i - 1]) && is_single_cap(words[i - 2]) {
            expand[i] = true;
            expand[i - 2] = true;
        }
        // Adjacent capitals: A B
        if i + 1 < n && is_single_cap(words[i + 1]) {
            expand[i] = true;
            expand[i + 1] = true;
        }
    }

    words
        .iter()
        .enumerate()
        .map(|(i, w)| {
            if expand[i] {
                let c = w.chars().next().unwrap();
                // Pronoun I in “I and B” still gets eye — rare; letter list is intentional.
                letter_spoken_name(c).to_string()
            } else {
                (*w).to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn expand_symbol_words(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 32);
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut i = 0;
    while i < n {
        let ch = chars[i];
        let word: Option<&str> = match ch {
            '&' | '＆' | '﹠' => Some("and"),
            '@' => Some("at"),
            '%' | '％' => Some("percent"),
            '°' => Some("degrees"),
            '§' => Some("section"),
            '€' => Some("euros"),
            '£' => Some("pounds"),
            '¥' => Some("yen"),
            '¢' => Some("cents"),
            '₹' => Some("rupees"),
            '₩' => Some("won"),
            '$' => Some("dollars"),
            '#' => Some("number"),
            '+' | '＋' => Some("plus"),
            '=' | '＝' => Some("equals"),
            '×' | '✕' | '✖' => Some("times"),
            '÷' => Some("divided by"),
            '±' => Some("plus or minus"),
            // ~50% → "approximately 50 percent", not “tilde”.
            '~' | '∼' | '≈' | '≃' | '≅' | '⁓' | '～' => Some("approximately"),
            '≠' | '≢' => Some("not equal to"),
            '≤' | '≦' => Some("less than or equal to"),
            '≥' | '≧' => Some("greater than or equal to"),
            '<' if !looks_like_markup_boundary(&chars, i) => Some("less than"),
            '>' if !looks_like_markup_boundary(&chars, i) => Some("greater than"),
            '½' => Some("one half"),
            '⅓' => Some("one third"),
            '⅔' => Some("two thirds"),
            '¼' => Some("one quarter"),
            '¾' => Some("three quarters"),
            '⅕' => Some("one fifth"),
            '⅖' => Some("two fifths"),
            '⅗' => Some("three fifths"),
            '⅘' => Some("four fifths"),
            '⅙' => Some("one sixth"),
            '⅚' => Some("five sixths"),
            '⅐' => Some("one seventh"),
            '⅛' => Some("one eighth"),
            '⅜' => Some("three eighths"),
            '⅝' => Some("five eighths"),
            '⅞' => Some("seven eighths"),
            '⅑' => Some("one ninth"),
            '⅒' => Some("one tenth"),
            '№' => Some("number"),
            'ℓ' => Some("liters"),
            'µ' | 'μ' => Some("micro"),
            'Ω' => Some("ohms"),
            '∞' => Some("infinity"),
            '√' => Some("square root of"),
            _ => None,
        };
        // "About ~50" / "roughly ~3": the word already says it — just drop the tilde.
        if matches!(ch, '~' | '∼' | '⁓' | '～') && prev_word_is_approximate(&out) {
            push_space(&mut out);
            i += 1;
            continue;
        }
        if let Some(w) = word {
            push_token(&mut out, w);
            i += 1;
            continue;
        }
        out.push(ch);
        i += 1;
    }
    out
}

fn prev_word_is_approximate(out: &str) -> bool {
    let w: String = out
        .trim_end()
        .chars()
        .rev()
        .take_while(|c| c.is_alphabetic() || *c == '.')
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    matches!(
        w.to_lowercase().trim_end_matches('.'),
        "about"
            | "around"
            | "roughly"
            | "approximately"
            | "nearly"
            | "almost"
            | "some"
            | "circa"
            | "ca"
            | "approx"
    )
}

/// `-` at `i` is a negative sign: a digit right after, and no letter or digit
/// right before (`-5`, `(−3)`, table-cell `spikes)-90`; not `COVID-19` or `3-5`).
fn is_negative_sign_at(chars: &[char], i: usize) -> bool {
    let before_ok = i == 0 || !chars[i - 1].is_alphanumeric();
    before_ok && chars.get(i + 1).is_some_and(char::is_ascii_digit)
}

/// Avoid turning every angle bracket into words if it looks like a tag leftover.
fn looks_like_markup_boundary(chars: &[char], i: usize) -> bool {
    let ch = chars[i];
    if ch == '<' {
        chars
            .get(i + 1)
            .is_some_and(|c| c.is_ascii_alphabetic() || *c == '/')
    } else if ch == '>' {
        i > 0 && chars[..i].iter().rev().take(24).any(|c| *c == '<')
    } else {
        false
    }
}

fn collapse_ws_keep_paragraphs(s: &str) -> String {
    let mut paras = Vec::new();
    let mut cur = String::new();
    let mut newline_run = 0u32;

    let flush = |cur: &mut String, paras: &mut Vec<String>| {
        let p = cur.split_whitespace().collect::<Vec<_>>().join(" ");
        cur.clear();
        if !p.is_empty() {
            paras.push(p);
        }
    };

    for ch in s.chars() {
        if ch == '\n' {
            newline_run += 1;
            continue;
        }
        if newline_run > 0 {
            if newline_run >= 2 {
                flush(&mut cur, &mut paras);
            } else if !cur.is_empty() && !cur.ends_with(' ') {
                cur.push(' ');
            }
            newline_run = 0;
        }
        if ch == '\t' {
            cur.push(' ');
        } else {
            cur.push(ch);
        }
    }
    flush(&mut cur, &mut paras);
    paras.join("\n\n")
}

/// Merge punctuation piles left by dropped symbols, per paragraph:
/// `cope , .` → `cope.`, `main, , I` → `main, I`, `dee bee em .` → `dee bee em.`,
/// a paragraph starting with `, ` loses it.
fn tidy_punctuation(s: &str) -> String {
    const MARKS: [char; 6] = ['.', ',', ';', ':', '!', '?'];
    let mut out = String::with_capacity(s.len());
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut i = 0;
    while i < n {
        let ch = chars[i];
        if MARKS.contains(&ch) {
            // Drop spaces before a mark.
            while out.ends_with(' ') {
                out.pop();
            }
            let at_para_start = out.is_empty() || out.ends_with('\n');
            if ch == ',' && at_para_start {
                i += 1;
                while chars.get(i) == Some(&' ') {
                    i += 1;
                }
                continue;
            }
            // A comma (or repeat) right before another mark gives way to it.
            if let Some(last) = out.chars().last() {
                if ch == '.' && matches!(last, '?' | '!') {
                    // `?.` → `?`
                    i += 1;
                    continue;
                }
                if last == ',' || (last == ch && ch != '.') {
                    out.pop();
                } else if MARKS.contains(&last) && ch == ',' {
                    // `., ` / `?, ` → keep the stronger mark, drop this comma.
                    i += 1;
                    continue;
                }
            }
            out.push(ch);
            i += 1;
            continue;
        }
        if ch == ' ' && (out.is_empty() || out.ends_with(['\n', ' '])) {
            i += 1;
            continue;
        }
        out.push(ch);
        i += 1;
    }
    out
}

/// A hyphen left hanging by a token rewrite: `word -word` or `word- word`
/// (one side spaced, letters both sides) → `word word`. Spaced dashes
/// (`a - b`) and normal hyphens (`well-known`) are kept.
fn join_stray_hyphens(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let n = chars.len();
    let mut out = String::with_capacity(s.len());
    for i in 0..n {
        if chars[i] == '-' && i > 0 && i + 1 < n {
            let (p, q) = (chars[i - 1], chars[i + 1]);
            let pp = if i >= 2 { chars[i - 2] } else { ' ' };
            let qq = chars.get(i + 2).copied().unwrap_or(' ');
            let lead = p == ' ' && pp.is_alphabetic() && q.is_alphabetic();
            let trail = q == ' ' && p.is_alphabetic() && qq.is_alphabetic();
            if lead {
                continue; // `x -catcher` → `x catcher` (space already there)
            }
            if trail {
                continue; // `Post- ee` → `Post ee`
            }
        }
        out.push(chars[i]);
    }
    out
}

fn push_soft_pause(out: &mut String) {
    // Avoid ", , ," piles
    let trimmed_end = out.trim_end_matches([' ', '\t']);
    if trimmed_end.ends_with([',', '.', '!', '?', ';', ':']) {
        if !out.ends_with(' ') {
            out.push(' ');
        }
        return;
    }
    while out.ends_with(' ') {
        out.pop();
    }
    if !out.is_empty() && !out.ends_with('\n') {
        out.push(',');
    }
    out.push(' ');
}

fn push_space(out: &mut String) {
    if out.is_empty() || out.ends_with([' ', '\n', '\t']) {
        return;
    }
    out.push(' ');
}

fn push_token(out: &mut String, token: &str) {
    // A number right after a sign stays attached (`–90 dBm`, `-52.8`): a space
    // would hide the negative from the minus check in `map_chars`.
    let signed_number = token.starts_with(|c: char| c.is_ascii_digit())
        && out.ends_with([
            '-', '\u{2212}', '\u{2013}', '\u{2012}', '\u{FE63}', '\u{FF0D}',
        ]);
    if !out.is_empty() && !out.ends_with([' ', '\n', '\t']) && !signed_number {
        out.push(' ');
    }
    out.push_str(token);
    out.push(' ');
}

fn is_private_use(c: char) -> bool {
    let u = c as u32;
    (0xE000..=0xF8FF).contains(&u)
        || (0xF0000..=0xFFFFD).contains(&u)
        || (0x100000..=0x10FFFD).contains(&u)
}

/// Free-standing modifier letters that espeak may name (not combining marks Mn).
fn is_free_modifier_symbol(c: char) -> bool {
    let u = c as u32;
    // Spacing modifier letters block (includes ˆ ˇ ˘ …)
    (0x02B0..=0x02FF).contains(&u)
        // Modifier tone letters
        || (0xA700..=0xA71F).contains(&u)
}

/// Combining diacritical marks (kept only after a base letter).
fn is_combining_mark(c: char) -> bool {
    let u = c as u32;
    (0x0300..=0x036F).contains(&u) // Combining Diacritical Marks (incl. U+0302)
        || (0x1AB0..=0x1AFF).contains(&u)
        || (0x1DC0..=0x1DFF).contains(&u)
        || (0x20D0..=0x20FF).contains(&u)
        || (0xFE20..=0xFE2F).contains(&u)
}

fn is_emoji_or_pictograph(c: char) -> bool {
    let u = c as u32;
    (0x1F300..=0x1F5FF).contains(&u) // misc symbols & pictographs
        || (0x1F600..=0x1F64F).contains(&u) // emoticons
        || (0x1F680..=0x1F6FF).contains(&u) // transport
        || (0x1F700..=0x1F77F).contains(&u)
        || (0x1F780..=0x1F7FF).contains(&u)
        || (0x1F800..=0x1F8FF).contains(&u)
        || (0x1F900..=0x1F9FF).contains(&u) // supplemental
        || (0x1FA00..=0x1FA6F).contains(&u)
        || (0x1FA70..=0x1FAFF).contains(&u)
        || (0x2600..=0x26FF).contains(&u) // misc symbols
        || (0x2700..=0x27BF).contains(&u) // dingbats
        || (0xFE00..=0xFE0F).contains(&u) // variation selectors
        || u == 0x200D // ZWJ
        || (0x1F1E0..=0x1F1FF).contains(&u) // flags
        || (0x2300..=0x23FF).contains(&u) // misc technical (often pictographic)
}

#[cfg(test)]
mod tests {
    use super::sanitize_for_speech;

    #[test]
    fn em_dash_becomes_pause_not_circumflex() {
        let out = sanitize_for_speech("hello — world");
        assert!(!out.to_lowercase().contains("circumflex"), "{out}");
        assert!(out.contains("hello") && out.contains("world"), "{out}");
        // soft pause is a comma
        assert!(out.contains(','), "{out}");
    }

    #[test]
    fn en_dash_digit_range_is_to() {
        let out = sanitize_for_speech("pages 1–5");
        assert!(out.contains("1 to 5"), "{out}");
    }

    #[test]
    fn ascii_digit_range_is_to() {
        let out = sanitize_for_speech("see 10-20");
        assert!(out.contains("10 to 20"), "{out}");
    }

    #[test]
    fn keeps_word_hyphen() {
        let out = sanitize_for_speech("well-known fact");
        assert!(out.contains("well-known"), "{out}");
    }

    #[test]
    fn caret_and_modifier_circumflex_stripped() {
        let out = sanitize_for_speech("x^2 and ˆ mark");
        assert!(!out.contains('^'), "{out}");
        assert!(!out.contains('\u{02C6}'), "{out}");
        assert!(!out.to_lowercase().contains("circumflex"), "{out}");
    }

    #[test]
    fn expands_common_symbols() {
        let out = sanitize_for_speech("A & B @ home 50%");
        assert!(out.contains("and"), "{out}");
        assert!(out.contains("at"), "{out}");
        assert!(out.contains("percent"), "{out}");
        // Letter A must not stay bare (espeak reads it as schwa).
        let low = out.to_lowercase();
        assert!(
            low.contains("eigh") && low.contains("bee"),
            "expected letter names for A and B, got: {out}"
        );
        assert!(
            !low.split_whitespace().any(|w| w == "a"),
            "bare A left in: {out}"
        );
    }

    #[test]
    fn article_a_not_expanded_to_letter_name() {
        let out = sanitize_for_speech("A well-known fact");
        let low = out.to_lowercase();
        // Article “A” must stay; not “eigh well-known”
        assert!(
            !low.contains("eigh"),
            "article A should not become letter name: {out}"
        );
        assert!(low.contains("well-known"), "{out}");
    }

    #[test]
    fn currency_amount_then_unit() {
        // Kokoro flip_money style — not "dollars 5"
        let out = sanitize_for_speech("Prices: $5, €10, £3, ¥100.");
        assert!(out.contains("5 dollars"), "{out}");
        assert!(out.contains("10 euros"), "{out}");
        assert!(out.contains("3 pounds"), "{out}");
        assert!(out.contains("100 yen"), "{out}");
        assert!(!out.contains("dollars 5"), "{out}");
    }

    #[test]
    fn currency_with_cents() {
        let out = sanitize_for_speech("It costs $5.99 today.");
        assert!(out.contains("5 dollars and 99 cents"), "{out}");
    }

    #[test]
    fn currency_singular() {
        let out = sanitize_for_speech("Only $1 left.");
        assert!(out.contains("1 dollar"), "{out}");
        assert!(!out.contains("1 dollars"), "{out}");
    }

    #[test]
    fn currency_thousands_comma() {
        let out = sanitize_for_speech("Budget $1,000.");
        assert!(
            out.contains("1000 dollars") || out.contains("1,000 dollars"),
            "{out}"
        );
        // we strip grouping commas in the digit run → 1000
        assert!(out.contains("1000 dollars"), "{out}");
    }

    #[test]
    fn tilde_is_approximately() {
        let out = sanitize_for_speech("~50% noise");
        assert!(out.contains("approximately"), "{out}");
        assert!(out.contains("percent"), "{out}");
        assert!(!out.contains('~'), "{out}");
    }

    #[test]
    fn abbreviations_expand() {
        let out = sanitize_for_speech("Mr. Smith saw Dr. Jones, e.g. vs. etc.");
        assert!(out.contains("Mister"), "{out}");
        assert!(out.contains("Doctor"), "{out}");
        assert!(out.contains("for example"), "{out}");
        assert!(out.contains("versus"), "{out}");
        assert!(out.contains("et cetera"), "{out}");
    }

    #[test]
    fn no_period_only_number_when_digit_follows() {
        // Sentence / word “No.” must not become “number”
        let alone = sanitize_for_speech("She said No. Then left.");
        assert!(
            !alone.to_lowercase().contains("number"),
            "bare No. should not be number: {alone}"
        );
        assert!(
            alone.to_lowercase().contains("no"),
            "expected word no kept: {alone}"
        );

        // Catalog style still expands
        let cat = sanitize_for_speech("See No. 12 and Nos. 3 for details.");
        assert!(
            cat.to_lowercase().contains("number") && cat.contains("12"),
            "No. 12 should expand: {cat}"
        );
        assert!(
            cat.to_lowercase().contains("numbers") && cat.contains("3"),
            "Nos. 3 should expand: {cat}"
        );
    }

    #[test]
    fn times_expand() {
        let out = sanitize_for_speech("Meet at 2:30 or 3:00 or 2:05.");
        assert!(out.contains("2 30"), "{out}");
        assert!(out.contains("3 o'clock"), "{out}");
        assert!(out.contains("2 oh 5"), "{out}");
    }

    #[test]
    fn ascii_fractions_expand() {
        let out = sanitize_for_speech("Add 1/2 and 3/4 cup.");
        assert!(out.contains("one half"), "{out}");
        assert!(out.contains("three quarters"), "{out}");
    }

    #[test]
    fn slash_dates_spoken_casual_year() {
        // Prefer “twenty twenty six” over espeak’s “two thousand twenty six”.
        let a = sanitize_for_speech("Meeting on 03/20/2026.");
        assert!(
            a.to_lowercase()
                .contains("march twentieth twenty twenty six"),
            "{a}"
        );
        assert!(!a.contains("03/20/2026"), "{a}");
        assert!(!a.to_lowercase().contains("two thousand"), "{a}");
        assert!(!a.to_lowercase().contains("slash"), "{a}");

        let b = sanitize_for_speech("Due 12/10/2025.");
        assert!(
            b.to_lowercase()
                .contains("december tenth twenty twenty five"),
            "{b}"
        );

        // Fractions still expand; dates do not become “N over M”.
        let c = sanitize_for_speech("Ratio 5/8 and date 3/15/2020.");
        assert!(
            c.contains("five eighths") || c.contains("five over eight"),
            "{c}"
        );
        assert!(
            c.to_lowercase().contains("march fifteenth twenty twenty"),
            "{c}"
        );
    }

    #[test]
    fn month_year_spoken_casual_not_two_thousand() {
        // espeak says “May two thousand twenty six”; we want pair-style years.
        let a = sanitize_for_speech("Published May 2026.");
        assert!(a.to_lowercase().contains("may twenty twenty six"), "{a}");
        assert!(!a.contains("2026"), "{a}");
        assert!(!a.to_lowercase().contains("two thousand"), "{a}");

        let b = sanitize_for_speech("May 15, 2026 update");
        assert!(
            b.to_lowercase().contains("may fifteenth twenty twenty six"),
            "{b}"
        );

        let c = sanitize_for_speech("Jan. 3 1999");
        assert!(
            c.to_lowercase()
                .contains("january third nineteen ninety nine"),
            "{c}"
        );

        // Bare year without month stays digits (not a forced date rewrite)
        let d = sanitize_for_speech("About 2026 users.");
        assert!(d.contains("2026"), "{d}");
    }

    #[test]
    fn invalid_slash_date_left_alone() {
        // Not a real calendar day — leave for fraction / plain digit path.
        let out = sanitize_for_speech("Bogus 2/31/2020.");
        assert!(
            out.contains("2/31/2020") || out.contains("2 / 31") || out.contains("31"),
            "{out}"
        );
        // Should not invent “February thirty first”
        assert!(
            !out.to_lowercase().contains("february thirty first"),
            "{out}"
        );
    }

    #[test]
    fn urls_and_emails_softened() {
        // Host spoken (paths are noise); no "slash slash", "h t t p" or "at sign".
        let out = sanitize_for_speech(
            "See https://www.example.com/path?q=1 and john.smith@mail.co.uk now.",
        );
        assert!(out.contains("example dot com"), "{out}");
        assert!(
            out.contains("john dot smith at mail dot co dot uk"),
            "{out}"
        );
        assert!(
            !out.contains("https") && !out.contains("www") && !out.contains("path"),
            "{out}"
        );
        assert!(!out.contains('@') && !out.contains('/'), "{out}");
        let end = sanitize_for_speech("Visit www.gutenberg.org.");
        assert!(end.contains("gutenberg dot org."), "{end}");
    }

    #[test]
    fn ordinals_expand() {
        let out = sanitize_for_speech("The 1st and 21st day.");
        assert!(out.contains("first"), "{out}");
        assert!(out.contains("twenty first"), "{out}");
    }

    #[test]
    fn units_expand() {
        let out = sanitize_for_speech("Signal at 400 MHz and 2.4GHz, temp 72°F.");
        assert!(out.contains("megahertz"), "{out}");
        assert!(out.contains("gigahertz"), "{out}");
        assert!(out.contains("Fahrenheit"), "{out}");
    }

    #[test]
    fn units_singular_milligauss_and_bare_power() {
        let one = sanitize_for_speech("1 mW, 1 km, 1 mph, 1°F, 1 MHz, 1.0 nW, 21 mW.");
        assert!(one.contains("1 milliwatt,"), "{one}");
        assert!(one.contains("1 kilometer,"), "{one}");
        assert!(one.contains("1 mile per hour"), "{one}");
        assert!(one.contains("1 degree Fahrenheit"), "{one}");
        assert!(one.contains("1 megahertz"), "{one}");
        assert!(one.contains("nanowatts"), "{one}");
        assert!(one.contains("21 milliwatts"), "{one}");

        let mg = sanitize_for_speech("0.2-1 mG, 5 mg dose.");
        assert!(mg.contains("1 milligauss"), "{mg}");
        assert!(mg.contains("5 milligrams"), "{mg}");

        let bare = sanitize_for_speech("Power in pW/nW, shown in pW.");
        assert!(bare.contains("picowatts"), "{bare}");
        assert!(bare.contains("nanowatts"), "{bare}");
        assert!(!bare.contains("pW"), "{bare}");
    }

    #[test]
    fn acronyms_and_dict() {
        // ALLCAPS letter-acronyms → hyphenated spoken letter words (snappy).
        // Word-acronyms like NASA stay whole.
        let out = sanitize_for_speech("EMF and RF from NASA and the FBI.");
        assert!(out.contains("ee-em-ef"), "{out}");
        // 2-letter RF: spaced (hyphen would sound like “are ref”)
        assert!(out.contains("ar ef"), "{out}");
        assert!(!out.contains("ar-ef"), "{out}");
        assert!(out.contains("NASA"), "{out}");
        assert!(out.contains("ef-bee-eye"), "{out}");
        assert!(!out.contains("eff"), "{out}"); // espeak mangles "eff"
        assert!(!out.contains("F B I"), "{out}");
        assert!(!out.contains("ee em ef"), "{out}"); // 3-letter stays hyphenated
        assert!(!out.contains("EMF"), "{out}");

        // Acronym EF (not the word "eff") → spaced "ee ef"
        let ef = sanitize_for_speech("Measure EF and EMF levels.");
        assert!(ef.contains("ee ef"), "{ef}");
        assert!(!ef.contains("ee-ef"), "{ef}");
        assert!(ef.contains("ee-em-ef"), "{ef}");
        assert!(!ef.to_lowercase().contains("eff"), "{ef}");
    }

    #[test]
    fn model_number_em_f_390_is_three_ninety() {
        let out = sanitize_for_speech("GQ EMF-390 meter.");
        let low = out.to_lowercase();
        assert!(low.contains("three ninety"), "{out}");
        assert!(!low.contains("hundred"), "{out}");
        assert!(!out.contains("390"), "{out}");
        // ordinary counts still leave digits for espeak cardinals
        let count = sanitize_for_speech("There were 390 people.");
        assert!(
            count.contains("390"),
            "bare count should stay digits, got: {count}"
        );
    }

    #[test]
    fn test_ids_spell_letter_and_each_digit() {
        // Leading zeros are kept; letter A is “eigh”, not “ay”.
        let a = sanitize_for_speech("Test A01");
        assert!(
            a.to_lowercase().contains("eigh zero one"),
            "expected eigh zero one, got: {a}"
        );
        assert!(!a.contains("A01"), "{a}");
        assert!(!a.to_lowercase().contains("ay zero"), "{a}");

        let b = sanitize_for_speech("Test B03");
        assert!(b.to_lowercase().contains("bee zero three"), "{b}");

        let z = sanitize_for_speech("Test Z01");
        assert!(z.to_lowercase().contains("zee zero one"), "{z}");
    }

    #[test]
    fn acronyms_as_spoken_letter_words() {
        // Not compact letter-mode (slow) and not capital U S B (robotic).
        let out = sanitize_for_speech("OK use USB and PDF and AI tools.");
        let low = out.to_lowercase();
        assert!(low.contains("okay"), "{out}");
        assert!(low.contains("you-ess-bee"), "{out}");
        assert!(low.contains("pee-dee-ef"), "{out}");
        assert!(low.contains("eigh eye"), "{out}");
        assert!(!low.contains("eff"), "{out}");
        assert!(!out.contains("USB"), "{out}");
        assert!(!out.contains("U S B"), "{out}");
        assert!(!out.contains("A. I"), "{out}");
    }

    #[test]
    fn roman_in_context() {
        let out = sanitize_for_speech("See Chapter XIV and Part III.");
        assert!(out.contains("14"), "{out}");
        assert!(out.contains("3"), "{out}");
        // bare I as pronoun should remain
        let out2 = sanitize_for_speech("I went home.");
        assert!(out2.starts_with('I') || out2.contains("I went"), "{out2}");
    }

    #[test]
    fn ellipsis_and_double_hyphen_pause() {
        let a = sanitize_for_speech("wait... go");
        let b = sanitize_for_speech("wait -- go");
        assert!(a.contains(','), "{a}");
        assert!(b.contains(','), "{b}");
    }

    #[test]
    fn keeps_paragraph_breaks() {
        let out = sanitize_for_speech("first para.\n\nsecond para.");
        assert!(out.contains("\n\n"), "{out:?}");
    }

    #[test]
    fn single_newlines_collapse_to_spaces() {
        let out = sanitize_for_speech("Hello world.\nNext line starts here.");
        assert!(
            !out.contains("world.\n"),
            "single newlines must join with a space: {out:?}"
        );
        assert!(
            out.to_lowercase().contains("hello world.") && out.to_lowercase().contains("next line"),
            "{out:?}"
        );
        let wrap = sanitize_for_speech("Hello\nworld continues.");
        assert!(wrap.to_lowercase().contains("hello world"), "{wrap:?}");
    }

    #[test]
    fn strips_emoji() {
        let out = sanitize_for_speech("hello 😀 world");
        assert!(!out.contains('😀'), "{out}");
        assert!(out.contains("hello") && out.contains("world"), "{out}");
    }

    #[test]
    fn folds_diacritics_for_english_speech() {
        // espeak reads accented words correctly; folding would turn résumé into "resume".
        let out = sanitize_for_speech("café naïve hôtel résumé jalapeño façade");
        for w in ["café", "naïve", "hôtel", "résumé", "jalapeño", "façade"] {
            assert!(out.contains(w), "{w} lost in {out}");
        }
        // Lone accented letters and consonant marks are still folded.
        let lone = sanitize_for_speech("Grade é and Dvořák.");
        assert!(!lone.contains('é') && !lone.contains('ř'), "{lone}");
    }

    #[test]
    fn allcaps_words_read_as_words_acronyms_spelled() {
        let t = sanitize_for_speech("PRIDE AND PREJUDICE");
        assert_eq!(t.to_lowercase(), "pride and prejudice", "{t}");
        let t = sanitize_for_speech("THE END");
        assert_eq!(t, "the end", "{t}");
        let t = sanitize_for_speech("\"STOP!\" he shouted. \"GET OUT NOW.\"");
        assert!(t.contains("stop!") && t.contains("get out now"), "{t}");
        let t = sanitize_for_speech("the CIVIL code; I MIX paint");
        assert!(t.contains("civil code") && t.contains("mix paint"), "{t}");
        // Real acronyms still spelled, even inside a caps heading.
        let t = sanitize_for_speech("THE FBI ARRIVED with the BBC and EMF meters");
        assert!(
            t.contains("ef-bee-eye") && t.contains("bee-bee-see") && t.contains("ee-em-ef"),
            "{t}"
        );
        assert!(t.contains("the") && t.contains("arrived"), "{t}");
        // Word-ish acronyms: spelled alone, words inside a caps run.
        let t = sanitize_for_speech("He moved to the US. LET US GO NOW.");
        assert!(t.contains("you ess") || t.contains("you-ess"), "{t}");
        assert!(t.contains("let us go now"), "{t}");
        // NASA stays a word; OK → okay.
        let t = sanitize_for_speech("NASA said OK");
        assert!(t.contains("NASA") && t.contains("okay"), "{t}");
    }

    #[test]
    fn bare_domains_spoken_with_dots() {
        let out = sanitize_for_speech("Lookups (captive.apple.com, skadsdkless.appsflyersdk.com, gs-loc.apple.com) and tinysa.org.");
        assert!(out.contains("captive dot apple dot com"), "{out}");
        assert!(
            out.contains("skadsdkless dot appsflyersdk dot com"),
            "{out}"
        );
        assert!(out.contains("gs dash loc dot apple dot com"), "{out}");
        assert!(out.contains("tinysa dot org."), "{out}");
        let plain = sanitize_for_speech(
            "It ended.It was 3.5 volts. See report.pdf and e.g. this. Mr.Smith came.",
        );
        assert!(!plain.contains(" dot "), "{plain}");
        let email = sanitize_for_speech("Write to help@example.com today.");
        assert!(email.contains("help at example dot com"), "{email}");
    }

    #[test]
    fn y_hyphen_compounds_split() {
        let out = sanitize_for_speech(
            "Make it copy-paste easy. An early-warning sign. A well-known x-ray.",
        );
        assert!(out.contains("copy paste easy"), "{out}");
        assert!(out.contains("early warning"), "{out}");
        assert!(out.contains("well-known"), "{out}");
        let num = sanitize_for_speech("Model Sony-A7 and ninety-nine.");
        assert!(!num.contains("Sony A7"), "{num}");
    }

    #[test]
    fn lettered_appendix_and_part_labels() {
        let out = sanitize_for_speech("Appendix A- FOLLOW THE RABBIT");
        assert!(out.starts_with("Appendix eigh"), "{out}");
        let cd = sanitize_for_speech("See Appendix C and Appendix D. Part B, Section E.");
        assert!(cd.contains("Appendix see and Appendix dee"), "{cd}");
        assert!(cd.contains("Part bee"), "{cd}");
        assert!(cd.contains("Section ee"), "{cd}");
        assert!(!cd.contains("100") && !cd.contains("500"), "{cd}");
        let roman = sanitize_for_speech("Part IV and Part I and Chapter X and Section V.");
        assert!(roman.contains("Part 4"), "{roman}");
        assert!(roman.contains("Part 1"), "{roman}");
        assert!(roman.contains("Chapter 10"), "{roman}");
        assert!(roman.contains("Section 5"), "{roman}");
        let article = sanitize_for_speech("I read part A Tale of Two Cities.");
        assert!(article.contains("part A Tale"), "{article}");
    }

    #[test]
    fn roman_numerals_after_names() {
        let t = sanitize_for_speech(
            "World War II ended. Henry VIII had wives; Elizabeth II spoke; Apollo XI landed.",
        );
        assert!(t.contains("World War 2"), "{t}");
        assert!(t.contains("Henry the eighth"), "{t}");
        assert!(t.contains("Elizabeth the second"), "{t}");
        assert!(t.contains("Apollo 11"), "{t}");
        let t = sanitize_for_speech("World War I began. I MIX paint in Washington DC.");
        assert!(t.contains("World War 1"), "{t}");
        assert!(!t.contains("1009") && !t.contains("600"), "{t}");
    }

    #[test]
    fn negative_numbers_and_minus_sign() {
        assert!(sanitize_for_speech("It was −5 degrees.").contains("negative 5 degrees"));
        assert!(sanitize_for_speech("It was -5 degrees.").contains("negative 5 degrees"));
        assert!(sanitize_for_speech("Then 8 − 3 is 5.").contains("8 minus 3"));
        let t = sanitize_for_speech("COVID-19 and pages 10-20");
        assert!(
            t.contains("COVID") && !t.contains("minus") && !t.contains("negative"),
            "{t}"
        );
        assert!(t.contains("10 to 20"), "{t}");
    }

    #[test]
    fn phone_isbn_and_iso_dates_not_ranges() {
        let t = sanitize_for_speech("Call 555-123-4567 or 555-1234.");
        assert!(t.contains("5 5 5, 1 2 3, 4 5 6 7"), "{t}");
        assert!(t.contains("5 5 5, 1 2 3 4"), "{t}");
        assert!(!t.contains(" to "), "{t}");
        let t = sanitize_for_speech("ISBN 978-3-16-148410-0");
        assert!(t.contains("9 7 8, 3, 1 6") && !t.contains(" to "), "{t}");
        let t = sanitize_for_speech("Released 2024-01-15, years 1990-2000.");
        assert!(t.contains("January fifteenth"), "{t}");
        // A 4-digit range of plausible years is read as years.
        assert!(t.contains("nineteen ninety to two thousand"), "{t}");
    }

    #[test]
    fn money_with_magnitudes() {
        assert!(sanitize_for_speech("It cost $5M.").contains("5 million dollars"));
        assert!(sanitize_for_speech("Only $2.5k left").contains("2 point 5 thousand dollars"));
        assert!(sanitize_for_speech("A €3bn deal").contains("3 billion euros"));
        assert!(sanitize_for_speech("$1.2 billion").contains("1 point 2 billion dollars"));
        // Not a magnitude: a word that starts with M.
        let t = sanitize_for_speech("$5 Mustang");
        assert!(t.contains("5 dollars") && t.contains("Mustang"), "{t}");
    }

    #[test]
    fn exponents_and_approximate_tilde() {
        let t = sanitize_for_speech("x^2 + y^3 and 10^6");
        assert!(t.contains("x squared plus y cubed"), "{t}");
        assert!(t.contains("10 to the power of 6"), "{t}");
        let t = sanitize_for_speech("About ~50 people, roughly ~3 hours, ~20 left");
        assert!(
            !t.contains("About approximately") && !t.contains("roughly approximately"),
            "{t}"
        );
        assert!(t.contains("approximately 20"), "{t}");
    }

    #[test]
    fn saint_or_street() {
        let t = sanitize_for_speech("St. Paul and Main St.\n\nSt. Mary.");
        assert!(
            t.contains("Saint Paul") && t.contains("Main Street") && t.contains("Saint Mary"),
            "{t}"
        );
        let t = sanitize_for_speech("St. Paul lives on Main St. near 5th St. in St. Louis.");
        assert!(t.contains("Saint Paul"), "{t}");
        assert!(t.contains("Main Street"), "{t}");
        assert!(
            t.contains("5th Street") || t.contains("fifth Street"),
            "{t}"
        );
        assert!(t.contains("Saint Louis"), "{t}");
    }

    #[test]
    fn footnote_marks_silent_not_pauses() {
        let t = sanitize_for_speech("Note† and ‡ more");
        assert!(!t.contains(','), "{t}");
    }

    #[test]
    fn standalone_circumflex_vowels_not_named() {
        let out = sanitize_for_speech("Marks: â ê î ô û.");
        for ch in ['â', 'ê', 'î', 'ô', 'û'] {
            assert!(!out.contains(ch), "still has {ch:?} in {out}");
        }
        assert!(
            out.contains('a') && out.contains('e') && out.contains('i'),
            "{out}"
        );
        assert!(out.contains('o') && out.contains('u'), "{out}");
    }

    #[test]
    fn mojibake_em_dash_not_circumflex() {
        // UTF-8 em dash bytes misread as Latin-1.
        let broken = "gaslighting \u{00e2}\u{0080}\u{0094} are real";
        let out = sanitize_for_speech(broken);
        assert!(!out.contains('\u{00e2}'), "{out}");
        assert!(!out.to_lowercase().contains("circumflex"), "{out}");
        assert!(
            out.contains("gaslighting") && out.contains("are real"),
            "{out}"
        );
        assert!(out.contains(','), "{out}"); // soft pause
    }

    #[test]
    fn lone_combining_circumflex_dropped() {
        let out = sanitize_for_speech("Before \u{0302} after.");
        assert!(!out.contains('\u{0302}'), "{out}");
        assert!(out.contains("Before") && out.contains("after"), "{out}");
    }

    #[test]
    fn combining_mark_on_letter_not_spoken_as_name() {
        // NFD: mark kept or base letter remains; never leave a lone mark word.
        let out = sanitize_for_speech("cafe\u{0301}");
        assert!(out.contains("cafe") || out.contains("caf"), "{out}");
    }

    #[test]
    fn decimals_spoken_with_point() {
        let t = sanitize_for_speech("23.5 miles, 0.75 cups, pi is 3.14159, version 2.0.1, 10.5%.");
        assert!(t.contains("23 point 5 miles"), "{t}");
        assert!(t.contains("0 point 7 5 cups"), "{t}");
        assert!(t.contains("3 point 1 4 1 5 9"), "{t}");
        assert!(t.contains("2 point 0 point 1"), "{t}");
        assert!(t.contains("10 point 5 percent"), "{t}");
        // Money keeps cents; magnitudes keep their decimal as "point".
        assert!(sanitize_for_speech("$5.99").contains("5 dollars and 99 cents"));
        assert!(sanitize_for_speech("$2.5k").contains("2 point 5 thousand dollars"));
    }

    #[test]
    fn years_in_context_and_decades() {
        let t = sanitize_for_speech(
            "In 1847 there were 1847 soldiers; since 2026 (1999) it grew, 1914-1918.",
        );
        assert!(t.contains("In eighteen forty seven"), "{t}");
        assert!(t.contains("were 1847 soldiers"), "{t}");
        assert!(t.contains("since twenty twenty six"), "{t}");
        assert!(t.contains("nineteen ninety nine"), "{t}");
        assert!(t.contains("nineteen fourteen to nineteen eighteen"), "{t}");
        let t = sanitize_for_speech("The 1990s and the '80s and the 1800s.");
        assert!(
            t.contains("nineteen nineties")
                && t.contains("eighties")
                && t.contains("eighteen hundreds"),
            "{t}"
        );
        // Amounts are counts, not years.
        let t = sanitize_for_speech("It cost $2,026 and 1,500 people came.");
        assert!(
            !t.contains("twenty twenty six") && !t.contains("fifteen hundred"),
            "{t}"
        );
    }

    #[test]
    fn caps_model_code_spells_letters() {
        let out = sanitize_for_speech("the TriField EM100 hit numbers");
        assert!(out.contains("ee em one hundred"), "{out}");
        assert!(
            !out.contains("EM100") && !out.contains("TriField em one"),
            "{out}"
        );
    }

    #[test]
    fn single_cap_digit_code_is_spelled() {
        let out = sanitize_for_speech("legitimate O2 masts and a B2 bomber");
        assert!(out.contains("oh two"), "{out}");
        assert!(out.contains("bee two"), "{out}");
        let lower = sanitize_for_speech("cell a1 here");
        assert!(lower.contains("a1"), "{lower}");
    }

    #[test]
    fn word_slashes_are_not_spoken() {
        let out = sanitize_for_speech("I documented the before/after. Note time/location.");
        assert!(!out.contains('/') && !out.contains("slash"), "{out}");
        assert!(out.contains("before after"), "{out}");
        assert!(out.contains("time location"), "{out}");
        let o2 = sanitize_for_speech("masquerading as O2/Vodafone masts, LAC/ TAC values");
        assert!(o2.contains("oh two Vodafone"), "{o2}");
        assert!(o2.contains("el-eigh-see tee-eigh-see"), "{o2}");
        let sp = sanitize_for_speech("Ambient Wi-Fi / 4G and EF / RF / magnetic.");
        assert!(!sp.contains('/'), "{sp}");
        assert!(sp.contains("Wi-Fi, 4G"), "{sp}");
        let tbl = sanitize_for_speech("Mobile Catcher /\n\nNotes /Room photo");
        assert!(!tbl.contains('/'), "{tbl}");
        let on = sanitize_for_speech("Breaker ON/OFF test, IMSI/IMEI catchers.");
        assert!(on.contains("on off"), "{on}");
        assert!(on.contains("eye-em-ess-eye eye-em-ee-eye"), "{on}");
        assert!(!o2.contains('/'), "{o2}");
        let he = sanitize_for_speech("Ask him/her if he/she agrees.");
        assert!(
            he.contains("him or her") && he.contains("he or she"),
            "{he}"
        );
    }

    #[test]
    fn paren_suffix_joins_word() {
        let out = sanitize_for_speech("How I cope(d) with the item(s) and box(es).");
        assert!(out.contains("coped"), "{out}");
        assert!(out.contains("items") && out.contains("boxes"), "{out}");
        assert!(!out.contains("cope, d"), "{out}");
        let fx = sanitize_for_speech("Let f(x) be given.");
        assert!(!fx.contains("fx"), "{fx}");
    }

    #[test]
    fn compound_units_read_per() {
        let out = sanitize_for_speech("readings of 1023 V/m and 8 nW/cm² or 3 uW/cm2 at 60 km/h.");
        assert!(out.contains("1023 volts per meter"), "{out}");
        assert!(out.contains("8 nanowatts per square centimeter"), "{out}");
        assert!(out.contains("3 microwatts per square centimeter"), "{out}");
        assert!(out.contains("kilometers per hour"), "{out}");
        assert!(!out.contains('/'), "{out}");
    }

    #[test]
    fn en_dash_negative_numbers() {
        let out = sanitize_for_speech("Ambient: –70 to –90 dBm at 10–20 m. Hits reach –72 dBm.");
        assert!(
            out.contains("negative 70 to negative 90 dee bee em"),
            "{out}"
        );
        assert!(out.contains("10 to 20"), "{out}");
        assert!(out.contains("negative 72 dee bee em"), "{out}");
        let ascii = sanitize_for_speech("Range -30 to -40 dBm here, and -5 degrees.");
        assert!(
            ascii.contains("negative 30 to negative 40 dee bee em"),
            "{ascii}"
        );
        let dec = sanitize_for_speech("signal at –52.8 dBm and -40.9 dBm.");
        assert!(dec.contains("negative 52 point 8 dee bee em"), "{dec}");
        assert!(dec.contains("negative 40 point 9 dee bee em"), "{dec}");
        let gps = sanitize_for_speech("grid reference (51.004321, - 2.123456) while");
        assert!(gps.contains("negative 2 point 1"), "{gps}");
        let said = sanitize_for_speech("He said - 3 of them left.");
        assert!(
            !said.contains("minus") && !said.contains("negative"),
            "{said}"
        );
        let cell = sanitize_for_speech("RF spikes)-90 to -70");
        assert!(cell.contains("negative 90 to negative 70"), "{cell}");
        // En dash as punctuation, and hyphenated words, are unchanged.
        let dash =
            sanitize_for_speech("He paused – 5 minutes later he left. COVID-19 and pages 3-5.");
        assert!(
            !dash.contains("minus") && !dash.contains("negative"),
            "{dash}"
        );
    }

    #[test]
    fn heading_number_dash_is_a_pause() {
        let out = sanitize_for_speech("Chapter 4.5 – 400 MHz Targeting\n\nChapter 10 - 20 Ways");
        assert!(!out.contains(" to 400"), "{out}");
        assert!(out.contains("400 megahertz Targeting"), "{out}");
        assert!(!out.contains("10 to 20"), "{out}");
        let dash = sanitize_for_speech("Appendix B - Logging Template. The rabbit - he maps.");
        assert!(dash.contains("Appendix bee, Logging"), "{dash}");
        assert!(dash.contains("rabbit, he maps"), "{dash}");
        let range = sanitize_for_speech("Read pages 4 – 10 and 10-20.");
        assert!(
            range.contains("4 to 10") && range.contains("10 to 20"),
            "{range}"
        );
    }

    #[test]
    fn reference_numbers_read_digit_by_digit() {
        let out = sanitize_for_speech(
            "Case No 123456789, Cases 01234567 and Ref 01/2345678/25, Ref 26/123456.",
        );
        assert!(out.contains("1 2 3 4 5 6 7 8 9"), "{out}");
        assert!(out.contains("0 1 2 3 4 5 6 7"), "{out}");
        assert!(out.contains("0 1, 2 3 4 5 6 7 8, 2 5"), "{out}");
        assert!(out.contains("2 6, 1 2 3 4 5 6"), "{out}");
        let roa = sanitize_for_speech("Ref 01/RoA/26/123456/V, confirming. Layer 392/480/363 MHz.");
        assert!(roa.contains("2 6, 1 2 3 4 5 6"), "{roa}");
        assert!(roa.contains("392, 480, 363 megahertz"), "{roa}");
        assert!(!roa.contains("over"), "{roa}");
        let date = sanitize_for_speech("On 3/15/2020 it rained.");
        assert!(!date.contains("3, 15"), "{date}");
        assert!(!out.contains("over"), "{out}");
        let plain = sanitize_for_speech(
            "About 80000 people paid 10500 and 1,000,000 at 51.004321 in 2024.",
        );
        assert!(
            plain.contains("80000") && plain.contains("10500"),
            "{plain}"
        );
        assert!(plain.contains("0 0 4 3 2 1"), "{plain}");
        assert!(plain.contains("1,000,000"), "{plain}");
    }

    #[test]
    fn common_ratios() {
        let out = sanitize_for_speech(
            "The 80/20 Visibility Rule, open 24/7, a 50/50 split, and 3/7 left.",
        );
        assert!(out.contains("80 20 Visibility"), "{out}");
        assert!(out.contains("24 7") && out.contains("50 50"), "{out}");
        assert!(out.contains("3 over 7"), "{out}");
    }

    #[test]
    fn bare_compound_units_in_headers() {
        let out = sanitize_for_speech("EF (V/m), RF (µW/cm² or nW/cm²), speed in m/s.");
        assert!(out.contains("volts per meter"), "{out}");
        assert!(out.contains("microwatts per square centimeter"), "{out}");
        assert!(out.contains("nanowatts per square centimeter"), "{out}");
        assert!(out.contains("meters per second"), "{out}");
        assert!(!out.contains('²'), "{out}");
        let dbm = sanitize_for_speech("Record frequency and dBm. Not DBMS.");
        assert!(dbm.contains("and dee bee em"), "{dbm}");
        let ab = sanitize_for_speech("We ran A/B tests.");
        assert!(!ab.contains("amps"), "{ab}");
    }

    #[test]
    fn punctuation_piles_merge() {
        let out =
            sanitize_for_speech("when you don't break (™).\n\nring main (©), I photographed it");
        assert!(
            !out.contains(", .") && !out.contains(", ,") && !out.contains(" ."),
            "{out}"
        );
        assert!(out.contains("break."), "{out}");
        assert_eq!(
            super::tidy_punctuation(", hello , , world , . Next ?"),
            "hello, world. Next?"
        );
    }

    #[test]
    fn ranges_after_unit_or_currency() {
        let out = sanitize_for_speech(
            "RTL-SDR dongle (£20–£30 on Amazon), analyser (100 kHz – 7.3 GHz), and $5-$10.",
        );
        assert!(out.contains("20 pounds to 30 pounds"), "{out}");
        assert!(
            out.contains("100 kilohertz to 7 point 3 gigahertz"),
            "{out}"
        );
        assert!(out.contains("to 10 dollars"), "{out}");
        assert!(!out.contains("minus") && !out.contains("negative"), "{out}");
        let neg = sanitize_for_speech("from –70 to –90 dBm, then 3 V/m – 5 V/m.");
        assert!(neg.contains("negative 70 to negative 90"), "{neg}");
        assert!(
            neg.contains("3 volts per meter to 5 volts per meter"),
            "{neg}"
        );
        let pause = sanitize_for_speech("He paused – 5 minutes later he left.");
        assert!(!pause.contains(" to 5"), "{pause}");
    }

    #[test]
    fn hyphen_dates() {
        let a = sanitize_for_speech("Scan (04-01-2026, outside).");
        assert!(a.contains("4, 1, twenty twenty six"), "{a}");
        let b = sanitize_for_speech("On 25-12-2024 and 12-25-2024.");
        assert!(
            b.matches("December twenty fifth twenty twenty four")
                .count()
                == 2,
            "{b}"
        );
        let phone = sanitize_for_speech("Call 555-123-4567.");
        assert!(phone.contains("5 5 5, 1 2 3, 4 5 6 7"), "{phone}");
    }

    #[test]
    fn hashtags_and_glued_words() {
        let h = sanitize_for_speech("Same pattern. #BlueSky and incident #7.");
        assert!(h.contains("hashtag Blue Sky"), "{h}");
        assert!(h.contains("number 7"), "{h}");
        let g = sanitize_for_speech(
            "in 10 minutes a dayNo scary tech. Frequency Hops & TriggerActivated Nodes",
        );
        assert!(g.contains("day No scary"), "{g}");
        assert!(g.contains("Trigger Activated"), "{g}");
        let brands = sanitize_for_speech("My iPhone, TinySA and TriField.");
        assert!(
            brands.contains("iPhone") && brands.contains("TinySA") && brands.contains("TriField"),
            "{brands}"
        );
    }

    #[test]
    fn plural_acronyms_spelled() {
        let out = sanitize_for_speech("Wrong Cell IDs, spoofed SSIDs, SUVs and USBs.");
        assert!(out.contains("eye dees"), "{out}");
        assert!(out.contains("ess-ess-eye-dees"), "{out}");
        assert!(
            out.contains("ess-you-vees") && out.contains("you-ess-bees"),
            "{out}"
        );
    }

    #[test]
    fn peak_to_peak_and_tesla_units() {
        let out = sanitize_for_speech("hit 3531.1 mVpp, 2800+ mVpp, and 50 nT.");
        assert!(
            out.contains("3531 point 1 millivolts peak to peak"),
            "{out}"
        );
        assert!(out.contains("plus millivolts peak to peak"), "{out}");
        assert!(out.contains("50 nanotesla"), "{out}");
    }

    #[test]
    fn glued_caps_and_stray_hyphens() {
        let g = sanitize_for_speech(
            "400 MHz band (TETRAadjacent, military/NATOadjacent) and a IMSIcatcher.",
        );
        assert!(
            g.contains("adjacent") && !g.contains("TETRAadjacent") && !g.contains("NATOadjacent"),
            "{g}"
        );
        assert!(g.contains("catcher") && !g.contains("IMSIcatcher"), "{g}");
        let h = sanitize_for_speech("mobile IMSI-catchers and Post-EDTA sauna, a well-known fact.");
        assert!(!h.contains(" -") && !h.contains("- "), "{h}");
        assert!(h.contains("well-known"), "{h}");
        let sar =
            sanitize_for_speech("Every SAR response and formal SARs. CITY OFFICE SAR EXTENSION");
        assert!(
            sar.contains("ess-eigh-ar response") && sar.contains("ess-eigh-ars"),
            "{sar}"
        );
        let ghz = sanitize_for_speech("logs are mainly sub-GHz UHF, in MHz.");
        assert!(
            ghz.contains("gigahertz") && ghz.contains("in megahertz"),
            "{ghz}"
        );
        let eir = sanitize_for_speech("City Council (EIR000123): referred.");
        assert!(
            eir.contains("ee-eye-ar zero zero zero one two three"),
            "{eir}"
        );
        let q = sanitize_for_speech("Really?. Yes!.");
        assert!(
            q.contains("Really?") && !q.contains("?.") && !q.contains("!."),
            "{q}"
        );
    }

    #[test]
    fn pronoun_i_is_not_a_numeral() {
        for (input, keep) in [
            ("that book I read", "book I read"),
            ("for my part I think so", "part I think"),
            ("Facebook I guess", "Facebook I guess"),
            ("an act civil enough", "act civil"),
        ] {
            let out = sanitize_for_speech(input);
            assert!(out.contains(keep), "{input} → {out}");
        }
        assert!(sanitize_for_speech("Chapter I begins.").contains("Chapter 1"));
        assert!(sanitize_for_speech("See Part IV now.").contains("Part 4"));
    }

    #[test]
    fn one_cent_digit_is_tenths() {
        let out = sanitize_for_speech("It was $1.5 then.");
        assert!(out.contains("50 cents"), "{out}");
    }

    #[test]
    fn years_with_oh() {
        assert_eq!(super::year_words(1905), "nineteen oh five");
        assert_eq!(super::year_words(1900), "nineteen hundred");
        assert_eq!(super::year_words(1805), "eighteen oh five");
        assert_eq!(super::year_words(2026), "twenty twenty six");
    }

    #[test]
    fn mega_and_milli_units_by_case() {
        let out = sanitize_for_speech("A 5 MW plant and a 5 mW laser.");
        assert!(
            out.contains("5 megawatts") && out.contains("5 milliwatts"),
            "{out}"
        );
    }

    #[test]
    fn abbreviations_match_case() {
        let out = sanitize_for_speech("Mr. Smith had an MS diagnosis.");
        assert!(
            out.contains("Mister Smith") && !out.contains("Miz"),
            "{out}"
        );
    }

    #[test]
    fn late_ordinals_spelled() {
        let out = sanitize_for_speech("the 26th and 29th entries");
        assert!(
            out.contains("twenty sixth") && out.contains("twenty ninth"),
            "{out}"
        );
    }
}
