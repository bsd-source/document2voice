//! Heteronyms espeak-ng gets wrong (read/read, wind/wind, subject/subject …).
//!
//! espeak-ng has no part-of-speech context, so it reads some heteronyms with the
//! wrong sense. A POS tagger (the averaged perceptron from misaki-rs, data vendored in
//! `data/pos-tagger/`) decides verb vs noun, and only the word/sense pairs espeak was
//! measured to get wrong are rewritten — to espeak's own phoneme input `[[…]]`, or
//! plain "red" for past-tense "read". `heteronym_audit_espeak` re-runs that audit.

use std::sync::OnceLock;

/// Context that makes a noun/verb heteronym a **verb** even when the POS tagger
/// says noun/adjective (it slips on imperatives and after plural subjects):
/// - followed by an object ("Wind the clock", "Moderate the debate"), not after a
///   determiner ("the wind the…");
/// - after a plural noun / pronoun subject and followed by an adverb, preposition,
///   punctuation or the end ("Prices increase.", "Metals contract in cold") —
///   compounds like "the sales contract was signed" stay nouns.
fn verb_by_context(prev: &str, prev_tag: &str, next: &str, next_tag: &str) -> bool {
    const OBJECT_START: &[&str] = &[
        "the", "a", "an", "him", "her", "them", "it", "us", "me", "this", "that", "these", "those",
        "my", "your", "his", "their", "our", "its", "some", "all", "every",
    ];
    const DETERMINERS: &[&str] = &[
        "the", "a", "an", "this", "that", "these", "those", "my", "your", "his", "her", "their",
        "our", "its", "every", "each", "no", "any", "some",
    ];
    if DETERMINERS.contains(&prev) {
        return false;
    }
    let object_follows = OBJECT_START.contains(&next);
    let plural_subject = prev_tag == "NNS" || matches!(prev, "they" | "we" | "you" | "i");
    let clause_ends = next.is_empty()
        || next_tag.starts_with("RB")
        || next_tag == "IN"
        || next.chars().all(|c| c.is_ascii_punctuation());
    object_follows || (plural_subject && clause_ends)
}

/// "minute" = tiny after a linking verb or intensifier ("was minute", "very minute").
fn minute_is_adjective(prev: &str) -> bool {
    matches!(
        prev,
        "was"
            | "is"
            | "are"
            | "were"
            | "be"
            | "been"
            | "seems"
            | "seemed"
            | "looked"
            | "very"
            | "so"
            | "too"
            | "quite"
            | "rather"
            | "extremely"
            | "most"
    )
}

/// Is "read" past tense (/ɹɛd/)? The word before it settles most cases — the POS
/// tagger slips on imperatives ("Please/NNP read/VBD this") — then the tag.
fn read_is_past(prev: &str, tag: &str) -> bool {
    const PRESENT_BEFORE: &[&str] = &[
        "will",
        "would",
        "can",
        "could",
        "shall",
        "should",
        "may",
        "might",
        "must",
        "to",
        "please",
        "let",
        "let's",
        "lets",
        "do",
        "does",
        "did",
        "don't",
        "didn't",
        "doesn't",
        "won't",
        "can't",
        "couldn't",
        "wouldn't",
        "shouldn't",
        "i'll",
        "you'll",
        "we'll",
        "they'll",
        "he'll",
        "she'll",
        "cannot",
        "and",
    ];
    const PAST_BEFORE: &[&str] = &[
        "have", "has", "had", "having", "been", "was", "were", "is", "are", "am", "be", "being",
        "i've", "you've", "we've", "they've", "already", "just", "never", "once", "well", "widely",
        "recently",
    ];
    if PRESENT_BEFORE.contains(&prev) {
        false
    } else {
        PAST_BEFORE.contains(&prev) || matches!(tag, "VBD" | "VBN")
    }
}

/// Rewrite the heteronyms espeak-ng mispronounces (see module docs), using
/// misaki's POS tagger (vendored data, loaded on first use).
pub(crate) fn respell_heteronyms_for_espeak(text: &str) -> String {
    use misaki_rs::tagger::PerceptronTagger;
    #[derive(Clone, Copy, PartialEq)]
    enum Fix {
        Verb,
        Noun,
    }
    // (word, which sense espeak gets wrong, espeak mnemonic for that sense)
    const TABLE: &[(&str, Fix, &str)] = &[
        ("contract", Fix::Verb, "k@ntr'akt"),
        ("progress", Fix::Verb, "pr@gr'Es"),
        ("subject", Fix::Verb, "s@bdZ'Ekt"),
        ("conflict", Fix::Verb, "k@nfl'Ikt"),
        ("desert", Fix::Verb, "dI#z'3:t"),
        ("insult", Fix::Verb, "Ins'Vlt"),
        ("increase", Fix::Verb, "INkr'i:s"),
        ("excuse", Fix::Verb, "Ekskj'u:z"),
        ("close", Fix::Verb, "kl'oUz"),
        ("wind", Fix::Verb, "w'aInd"),
        ("separate", Fix::Verb, "s'Ep3r,eIt"),
        ("estimate", Fix::Verb, "'EstI#m,eIt"),
        ("moderate", Fix::Verb, "m'0d3r,eIt"),
        ("produce", Fix::Noun, "pr'oUdu:s"),
        ("import", Fix::Noun, "'Impo@t"),
        ("associate", Fix::Noun, "a#s'oUsI2;@t"),
    ];
    const VERB_BEFORE: &[&str] = &[
        "to", "will", "would", "can", "could", "should", "must", "may", "might", "please", "let's",
        "don't", "didn't", "doesn't", "won't", "can't", "they", "we", "i", "you",
    ];
    static TAGGER: OnceLock<PerceptronTagger> = OnceLock::new();

    let lower = text.to_lowercase();
    let mentions = |w: &str| {
        lower
            .split(|c: char| !c.is_alphabetic())
            .any(|x| x == w || x.strip_suffix('s') == Some(w))
    };
    let any = mentions("read")
        || mentions("wound")
        || mentions("minute")
        || mentions("does")
        || TABLE.iter().any(|(w, _, _)| mentions(w));
    if !any {
        return text.to_string();
    }
    let tagger = TAGGER.get_or_init(|| {
        PerceptronTagger::new(
            include_str!("../data/pos-tagger/weights.json"),
            include_str!("../data/pos-tagger/classes.txt"),
            include_str!("../data/pos-tagger/tags.json"),
        )
    });
    // Tokens: words (letters + apostrophes) and single punctuation marks, with spans.
    let mut spans: Vec<(usize, usize)> = Vec::new();
    let mut it = text.char_indices().peekable();
    while let Some((i, c)) = it.next() {
        if c.is_alphabetic() {
            let mut end = i + c.len_utf8();
            while let Some(&(j, d)) = it.peek() {
                if d.is_alphabetic() || d == '\'' || d == '’' {
                    end = j + d.len_utf8();
                    it.next();
                } else {
                    break;
                }
            }
            spans.push((i, end));
        } else if !c.is_whitespace() {
            spans.push((i, i + c.len_utf8()));
        }
    }
    let words: Vec<&str> = spans.iter().map(|&(a, b)| &text[a..b]).collect();
    let tags = tagger.tag(&words);
    let mut out = String::with_capacity(text.len() + 32);
    let mut last = 0;
    for (k, &(a, b)) in spans.iter().enumerate() {
        let w = words[k].to_lowercase();
        let in_table = |x: &str| TABLE.iter().any(|(t, _, _)| *t == x);
        let candidate = matches!(w.as_str(), "read" | "wound" | "minute" | "does")
            || in_table(&w)
            || w.strip_suffix('s').is_some_and(in_table);
        if !candidate {
            continue;
        }
        let prev_idx = (0..k)
            .rev()
            .find(|&j| words[j].chars().any(char::is_alphabetic));
        let prev = prev_idx
            .map(|j| words[j].to_lowercase())
            .unwrap_or_default();
        let tag = tags.get(k).map_or("", |t| t.tag.as_str());
        let prev_tag = prev_idx
            .and_then(|j| tags.get(j))
            .map_or("", |t| t.tag.as_str());
        let next = words
            .get(k + 1)
            .map(|w| w.to_lowercase())
            .unwrap_or_default();
        let next_tag = tags.get(k + 1).map_or("", |t| t.tag.as_str());
        let is_verb = tag.starts_with("VB")
            || VERB_BEFORE.contains(&prev.as_str())
            || verb_by_context(&prev, prev_tag, &next, next_tag);
        let is_noun = !is_verb && (tag.starts_with("NN") || tag.starts_with("JJ"));
        let replacement: Option<String> = match w.as_str() {
            "read" => read_is_past(&prev, tag).then(|| {
                if words[k].starts_with('R') {
                    "Red"
                } else {
                    "red"
                }
                .to_string()
            }),
            "wound" => matches!(tag, "VBD" | "VBN").then(|| "[[w'aUnd]]".to_string()),
            "minute" => (tag.starts_with("JJ") || minute_is_adjective(&prev))
                .then(|| "[[maIn'ju:t]]".to_string()),
            "does" => matches!(
                prev.as_str(),
                "the" | "these" | "those" | "two" | "three" | "many" | "some" | "several"
            )
            .then(|| "[[d'oUz]]".to_string()),
            _ => {
                let (entry, plural) = match TABLE.iter().find(|(x, _, _)| *x == w) {
                    Some(e) => (Some(e), false),
                    None => (
                        w.strip_suffix('s')
                            .and_then(|st| TABLE.iter().find(|(x, _, _)| *x == st)),
                        true,
                    ),
                };
                entry.and_then(|&(_, fix, mn)| {
                    let wanted = match fix {
                        Fix::Verb => is_verb,
                        Fix::Noun => is_noun,
                    };
                    wanted.then(|| {
                        format!(
                            "[[{}]]",
                            if plural {
                                mnemonic_add_s(mn)
                            } else {
                                mn.to_string()
                            }
                        )
                    })
                })
            }
        };
        if let Some(r) = replacement {
            out.push_str(&text[last..a]);
            out.push_str(&r);
            last = b;
        }
    }
    out.push_str(&text[last..]);
    out
}

/// espeak mnemonic "-s": `I#z` after sibilants, `s` after voiceless sounds, else `z`.
fn mnemonic_add_s(mn: &str) -> String {
    let tail: String = mn
        .chars()
        .rev()
        .take(2)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    let last = mn.chars().last().unwrap_or('a');
    let suffix = if matches!(last, 's' | 'z' | 'S' | 'Z') || tail == "tS" || tail == "dZ" {
        "I#z"
    } else if matches!(last, 'p' | 't' | 'k' | 'f' | 'T') {
        "s"
    } else {
        "z"
    };
    format!("{mn}{suffix}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn espeak_path_respells_past_read() {
        let t =
            respell_heteronyms_for_espeak("I read the book yesterday, and I will read it again.");
        assert_eq!(t, "I red the book yesterday, and I will read it again.");
        let t = respell_heteronyms_for_espeak("Please read this. I have read it. Read on!");
        assert_eq!(t, "Please read this. I have red it. Read on!");
        assert_eq!(
            respell_heteronyms_for_espeak("Nothing here."),
            "Nothing here."
        );
    }

    /// The audit behind the table: run real espeak-ng on verb and noun uses of these
    /// heteronyms (after our respelling) and require the right stressed syllable.
    /// Skipped when espeak-ng is not installed.
    #[test]
    fn heteronym_audit_espeak() {
        let ipa = |s: &str| -> Option<String> {
            let mut c = std::process::Command::new("espeak-ng")
                .args(["-q", "--ipa", "-v", "en-us"])
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .spawn()
                .ok()?;
            c.stdin
                .take()?
                .write_all(format!("{s}\n").as_bytes())
                .ok()?;
            Some(String::from_utf8_lossy(&c.wait_with_output().ok()?.stdout).replace('\n', " "))
        };
        if ipa("test").is_none() {
            eprintln!("espeak-ng not installed; skipping audit");
            return;
        }
        // (verb sentence, must contain, noun/adj sentence, must contain)
        const CASES: &[(&str, &str, &str, &str)] = &[
            (
                "They will record it.",
                "kˈɔːɹd",
                "He broke the record.",
                "ɹˈɛkɚd",
            ),
            (
                "I object to that.",
                "dʒˈɛkt",
                "That object is red.",
                "ˈɑːbdʒ",
            ),
            (
                "They present the prize.",
                "zˈɛnt",
                "A present for you.",
                "pɹˈɛzənt",
            ),
            ("They permit it.", "mˈɪt", "You need a permit.", "pˈɜːmɪt"),
            (
                "They conduct tests.",
                "dˈʌkt",
                "His conduct was bad.",
                "kˈɑːndʌkt",
            ),
            (
                "Metals contract in cold.",
                "tɹˈækt",
                "Sign the contract.",
                "kˈɑːntɹækt",
            ),
            (
                "They project growth.",
                "dʒˈɛkt",
                "The project failed.",
                "pɹˈɑːdʒ",
            ),
            (
                "Farms produce food.",
                "dˈuːs",
                "Fresh produce is cheap.",
                "pɹˈoʊd",
            ),
            (
                "Things progress slowly.",
                "ɡɹˈɛs",
                "We made progress.",
                "pɹˈɑːɡ",
            ),
            (
                "They rebel against it.",
                "bˈɛl",
                "He was a rebel.",
                "ɹˈɛbəl",
            ),
            (
                "They subject him to tests.",
                "dʒˈɛkt",
                "The subject was hard.",
                "sˈʌbdʒ",
            ),
            ("I suspect him.", "spˈɛkt", "The suspect fled.", "sˈʌspɛkt"),
            (
                "Reports conflict.",
                "flˈɪkt",
                "The conflict ended.",
                "kˈɑːnflɪkt",
            ),
            (
                "They contest the will.",
                "tˈɛst",
                "She won the contest.",
                "kˈɑːntɛst",
            ),
            (
                "Soldiers desert the army.",
                "zˈɜːt",
                "The desert is hot.",
                "dˈɛzɚt",
            ),
            (
                "Do not insult him.",
                "sˈʌlt",
                "That was an insult.",
                "ˈɪnsʌlt",
            ),
            (
                "They protest the law.",
                "tˈɛst",
                "The protest grew.",
                "pɹˈoʊt",
            ),
            (
                "They export oil.",
                "pˈɔːɹt",
                "Oil is an export.",
                "ˈɛkspɔːɹt",
            ),
            (
                "They import cars.",
                "pˈɔːɹt",
                "Cars are an import.",
                "ˈɪmpɔːɹt",
            ),
            ("Prices increase.", "kɹˈiːs", "A big increase.", "ˈɪŋkɹiːs"),
            (
                "They refuse to go.",
                "fjˈuːz",
                "Take out the refuse.",
                "ɹˈɛfjuːs",
            ),
            (
                "Please excuse him.",
                "kjˈuːz",
                "That is no excuse.",
                "kjˈuːs",
            ),
            ("Do not abuse it.", "bjˈuːz", "The abuse stopped.", "bjˈuːs"),
            ("I use it.", "jˈuːz", "What is the use?", "jˈuːs"),
            (
                "Please close the door.",
                "klˈoʊz",
                "The shop is close.",
                "klˈoʊs",
            ),
            (
                "They house the refugees.",
                "hˈaʊz",
                "The house is big.",
                "hˈaʊs",
            ),
            ("Wind the clock.", "wˈaɪnd", "The wind blew.", "wˈɪnd"),
            (
                "He wound the rope.",
                "wˈaʊnd",
                "The wound healed.",
                "wˈuːnd",
            ),
            ("They live here.", "lˈɪv", "A live concert.", "lˈaɪv"),
            ("Do not tear it.", "tˈɛɹ", "A tear fell.", "tˈɪɹ"),
            ("He lives here.", "lˈɪvz", "Cats have nine lives.", "lˈaɪvz"),
            ("Separate the eggs.", "ˌeɪt", "Two separate rooms.", "pɹət"),
            ("Estimate the cost.", "ˌeɪt", "A rough estimate.", "mət"),
            ("They graduate soon.", "eɪt", "She is a graduate.", "ət"),
            ("Moderate the debate.", "ˌeɪt", "A moderate amount.", "ɹət"),
            ("Associate the words.", "eɪt", "A business associate.", "ət"),
            // Known limitation: "a minute detail" (tiny) vs "a minute walk" needs meaning,
            // not grammar; only an adjective tag switches it ("It was minute.").
            (
                "Wait a minute.",
                "mˈɪnɪt",
                "The change was minute.",
                "maɪnjˈuːt",
            ),
            ("Does he know?", "dˈʌz", "The does ran off.", "dˈoʊz"),
            ("I read it yesterday.", "ɹˈɛd", "I will read it.", "ɹˈiːd"),
            (
                "It subjects him to tests.",
                "dʒˈɛkts",
                "The subjects were hard.",
                "sˈʌbdʒ",
            ),
        ];
        let mut wrong = Vec::new();
        for (vs, vk, ns, nk) in CASES {
            for (sent, key) in [(vs, vk), (ns, nk)] {
                let got = ipa(&respell_heteronyms_for_espeak(sent)).unwrap_or_default();
                if !got.contains(key) {
                    wrong.push(format!("{sent:30} want {key:10} got {}", got.trim()));
                }
            }
        }
        assert!(
            wrong.is_empty(),
            "{} wrong:\n{}",
            wrong.len(),
            wrong.join("\n")
        );
    }
}
