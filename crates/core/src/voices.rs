use serde::{Deserialize, Serialize};

/// Language or accent group for the voice dropdown in the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum Language {
    EnglishBritish,
    #[default]
    EnglishAmerican,
    Spanish,
    French,
    Hindi,
    Italian,
    Japanese,
    Portuguese,
    Chinese,
}

impl Language {
    /// Order in the UI: American, British, then the other languages A–Z by label.
    pub const ALL: [Language; 9] = [
        Self::EnglishAmerican,
        Self::EnglishBritish,
        Self::Chinese,
        Self::French,
        Self::Hindi,
        Self::Italian,
        Self::Japanese,
        Self::Portuguese,
        Self::Spanish,
    ];

    /// Human-readable name for dropdowns.
    pub fn label(self) -> &'static str {
        match self {
            Self::EnglishBritish => "English (British)",
            Self::EnglishAmerican => "English (American)",
            Self::Spanish => "Spanish",
            Self::French => "French",
            Self::Hindi => "Hindi",
            Self::Italian => "Italian",
            Self::Japanese => "Japanese",
            Self::Portuguese => "Portuguese (Brazil)",
            Self::Chinese => "Chinese (Mandarin)",
        }
    }

    /// Default voice id when the user picks this language.
    pub fn default_voice(self) -> &'static str {
        match self {
            Self::EnglishBritish => "bm_george",
            Self::EnglishAmerican => "am_adam",
            Self::Spanish => "em_alex",
            Self::French => "ff_siwis",
            Self::Hindi => "hm_omega",
            Self::Italian => "im_nicola",
            Self::Japanese => "jm_kumo",
            Self::Portuguese => "pm_alex",
            Self::Chinese => "zm_yunjian",
        }
    }
}

impl std::fmt::Display for Language {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

#[derive(Debug, Clone)]
pub struct VoiceInfo {
    pub id: String,
    pub language: Language,
    pub gender: &'static str,
    pub notes: &'static str,
}

fn v(id: &str, language: Language, gender: &'static str, notes: &'static str) -> VoiceInfo {
    VoiceInfo {
        id: id.into(),
        language,
        gender,
        notes,
    }
}

/// Full list of voices offered by the GUI and command-line tool.
pub(crate) fn all_voices() -> Vec<VoiceInfo> {
    let mut out = Vec::new();
    out.extend(english_british_voices());
    out.extend(english_american_voices());
    out.extend(spanish_voices());
    out.extend(french_voices());
    out.extend(hindi_voices());
    out.extend(italian_voices());
    out.extend(japanese_voices());
    out.extend(portuguese_voices());
    out.extend(chinese_voices());
    out
}

/// Voices for one language choice in the dropdown.
pub fn voices_for_language(lang: Language) -> Vec<VoiceInfo> {
    all_voices()
        .into_iter()
        .filter(|v| v.language == lang)
        .collect()
}

fn english_british_voices() -> Vec<VoiceInfo> {
    vec![
        v(
            "bm_george",
            Language::EnglishBritish,
            "male",
            "British male — default",
        ),
        v("bm_lewis", Language::EnglishBritish, "male", "British male"),
        v(
            "bm_daniel",
            Language::EnglishBritish,
            "male",
            "British male",
        ),
        v("bm_fable", Language::EnglishBritish, "male", "British male"),
        v(
            "bf_emma",
            Language::EnglishBritish,
            "female",
            "British female",
        ),
        v(
            "bf_isabella",
            Language::EnglishBritish,
            "female",
            "British female",
        ),
        v(
            "bf_alice",
            Language::EnglishBritish,
            "female",
            "British female",
        ),
        v(
            "bf_lily",
            Language::EnglishBritish,
            "female",
            "British female",
        ),
    ]
}

fn english_american_voices() -> Vec<VoiceInfo> {
    vec![
        v(
            "am_adam",
            Language::EnglishAmerican,
            "male",
            "American male — default",
        ),
        v(
            "am_fenrir",
            Language::EnglishAmerican,
            "male",
            "American male",
        ),
        v(
            "am_michael",
            Language::EnglishAmerican,
            "male",
            "American male",
        ),
        v(
            "am_puck",
            Language::EnglishAmerican,
            "male",
            "American male",
        ),
        v(
            "am_liam",
            Language::EnglishAmerican,
            "male",
            "American male",
        ),
        v(
            "am_onyx",
            Language::EnglishAmerican,
            "male",
            "American male",
        ),
        v(
            "am_echo",
            Language::EnglishAmerican,
            "male",
            "American male",
        ),
        v(
            "am_eric",
            Language::EnglishAmerican,
            "male",
            "American male",
        ),
        v(
            "af_heart",
            Language::EnglishAmerican,
            "female",
            "American female — top grade",
        ),
        v(
            "af_bella",
            Language::EnglishAmerican,
            "female",
            "American female",
        ),
        v(
            "af_nicole",
            Language::EnglishAmerican,
            "female",
            "American female",
        ),
        v(
            "af_sarah",
            Language::EnglishAmerican,
            "female",
            "American female",
        ),
        v(
            "af_sky",
            Language::EnglishAmerican,
            "female",
            "American female",
        ),
    ]
}

fn spanish_voices() -> Vec<VoiceInfo> {
    vec![
        v("em_alex", Language::Spanish, "male", "Spanish male"),
        v("em_santa", Language::Spanish, "male", "Spanish male"),
        v("ef_dora", Language::Spanish, "female", "Spanish female"),
    ]
}

fn french_voices() -> Vec<VoiceInfo> {
    vec![v("ff_siwis", Language::French, "female", "French female")]
}

fn hindi_voices() -> Vec<VoiceInfo> {
    vec![
        v("hm_omega", Language::Hindi, "male", "Hindi male"),
        v("hm_psi", Language::Hindi, "male", "Hindi male"),
        v("hf_alpha", Language::Hindi, "female", "Hindi female"),
        v("hf_beta", Language::Hindi, "female", "Hindi female"),
    ]
}

fn italian_voices() -> Vec<VoiceInfo> {
    vec![
        v("im_nicola", Language::Italian, "male", "Italian male"),
        v("if_sara", Language::Italian, "female", "Italian female"),
    ]
}

fn japanese_voices() -> Vec<VoiceInfo> {
    vec![
        v("jm_kumo", Language::Japanese, "male", "Japanese male"),
        v("jf_alpha", Language::Japanese, "female", "Japanese female"),
        v(
            "jf_gongitsune",
            Language::Japanese,
            "female",
            "Japanese female",
        ),
        v("jf_nezumi", Language::Japanese, "female", "Japanese female"),
        v(
            "jf_tebukuro",
            Language::Japanese,
            "female",
            "Japanese female",
        ),
    ]
}

fn portuguese_voices() -> Vec<VoiceInfo> {
    vec![
        v(
            "pm_alex",
            Language::Portuguese,
            "male",
            "Brazilian Portuguese male",
        ),
        v(
            "pm_santa",
            Language::Portuguese,
            "male",
            "Brazilian Portuguese male",
        ),
        v(
            "pf_dora",
            Language::Portuguese,
            "female",
            "Brazilian Portuguese female",
        ),
    ]
}

fn chinese_voices() -> Vec<VoiceInfo> {
    vec![
        v("zm_yunjian", Language::Chinese, "male", "Mandarin male"),
        v("zm_yunxi", Language::Chinese, "male", "Mandarin male"),
        v("zm_yunxia", Language::Chinese, "male", "Mandarin male"),
        v("zm_yunyang", Language::Chinese, "male", "Mandarin male"),
        v("zf_xiaobei", Language::Chinese, "female", "Mandarin female"),
        v("zf_xiaoni", Language::Chinese, "female", "Mandarin female"),
        v(
            "zf_xiaoxiao",
            Language::Chinese,
            "female",
            "Mandarin female",
        ),
        v("zf_xiaoyi", Language::Chinese, "female", "Mandarin female"),
    ]
}
