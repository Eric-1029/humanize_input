//! Every adjustable value of the two typing models, with names, one-line
//! explanations, ranges and units, so the parameter page is built from data.

use humanize_core::{HumanModel, TypingSettings};

pub struct Param {
    pub zh: &'static str,
    pub en: &'static str,
    pub hint_zh: &'static str,
    pub hint_en: &'static str,
    pub min: f64,
    pub max: f64,
    pub step: f64,
    pub digits: u32,
    pub unit: &'static str,
    pub get: fn(&HumanModel) -> f64,
    pub set: fn(&mut HumanModel, f64),
}

pub struct Group {
    pub zh: &'static str,
    pub en: &'static str,
    pub params: &'static [Param],
}

macro_rules! p {
    ($field:ident, $zh:literal, $en:literal, $hzh:literal, $hen:literal, $min:expr, $max:expr, $step:expr, $digits:expr, $unit:literal) => {
        Param {
            zh: $zh,
            en: $en,
            hint_zh: $hzh,
            hint_en: $hen,
            min: $min,
            max: $max,
            step: $step,
            digits: $digits,
            unit: $unit,
            get: |m| m.$field,
            set: |m, v| m.$field = v,
        }
    };
}

pub static HUMAN: &[Group] = &[
    Group {
        zh: "节奏",
        en: "Rhythm",
        params: &[
            p!(
                wpm,
                "速度",
                "Speed",
                "连续打字时的速度，按每分钟单词数计（1 词 = 5 个字符）。",
                "Speed while typing steadily, in words (5 characters) per minute.",
                5.0,
                250.0,
                1.0,
                0,
                "WPM"
            ),
            p!(
                interval_sigma,
                "节奏起伏",
                "Irregularity",
                "按键间隔的离散程度；越大越不规律。",
                "Spread of key intervals; higher is less regular.",
                0.0,
                1.2,
                0.01,
                2,
                ""
            ),
            p!(
                same_finger_factor,
                "同指连按",
                "Same finger",
                "同一根手指连按两个不同键时，间隔乘以这个数。",
                "Interval multiplier when one finger presses two different keys.",
                0.3,
                3.0,
                0.01,
                2,
                "×"
            ),
            p!(
                same_hand_factor,
                "同手连按",
                "Same hand",
                "同一只手、不同手指连按时的间隔倍数。",
                "Interval multiplier for two keys on the same hand.",
                0.3,
                3.0,
                0.01,
                2,
                "×"
            ),
            p!(
                alternate_hand_factor,
                "左右手交替",
                "Alternating hands",
                "左右手交替时的间隔倍数，通常小于 1。",
                "Interval multiplier when hands alternate; usually below 1.",
                0.3,
                3.0,
                0.01,
                2,
                "×"
            ),
            p!(
                common_bigram_factor,
                "常见字母组合",
                "Common letter pairs",
                "th、er 这类常见组合的间隔倍数。",
                "Interval multiplier for frequent pairs such as th and er.",
                0.3,
                3.0,
                0.01,
                2,
                "×"
            ),
            p!(
                familiar_word_factor,
                "常用词",
                "Common words",
                "the、and 这类常用词内部的间隔倍数。",
                "Interval multiplier inside very common words such as the and and.",
                0.3,
                3.0,
                0.01,
                2,
                "×"
            ),
            p!(
                dwell_ms,
                "按住时长",
                "Key hold",
                "每个键按下到抬起的中位时长。",
                "Median time a key stays down.",
                30.0,
                200.0,
                1.0,
                0,
                "ms"
            ),
            p!(
                dwell_sigma,
                "按住时长起伏",
                "Hold irregularity",
                "按住时长的离散程度。",
                "Spread of the key hold time.",
                0.0,
                0.8,
                0.01,
                2,
                ""
            ),
            p!(
                modifier_lead_ms,
                "Shift 提前量",
                "Shift lead",
                "Shift 比要配合的键提前按下的时间。",
                "How long Shift goes down before the key it modifies.",
                15.0,
                250.0,
                1.0,
                0,
                "ms"
            ),
        ],
    },
    Group {
        zh: "停顿",
        en: "Pauses",
        params: &[
            p!(
                word_pause_ms,
                "词前停顿",
                "Before each word",
                "开始打每个词之前多停的时间。",
                "Extra pause before starting each word.",
                0.0,
                2000.0,
                5.0,
                0,
                "ms"
            ),
            p!(
                punctuation_pause_ms,
                "逗号后停顿",
                "After commas",
                "打完 , ; : 之后多停的时间。",
                "Extra pause after , ; and :.",
                0.0,
                5000.0,
                10.0,
                0,
                "ms"
            ),
            p!(
                sentence_pause_ms,
                "句末停顿",
                "After sentences",
                "打完 . ! ? 和空格之后多停的时间。",
                "Extra pause after . ! or ? and a space.",
                0.0,
                10000.0,
                10.0,
                0,
                "ms"
            ),
            p!(
                paragraph_pause_ms,
                "段落停顿",
                "After paragraphs",
                "空行之后多停的时间；单个换行按 60% 计。",
                "Extra pause after a blank line; a single line break gets 60 %.",
                0.0,
                20000.0,
                50.0,
                0,
                "ms"
            ),
            p!(
                long_pause_percent,
                "长停顿概率",
                "Long-stop chance",
                "每个词前停下来想一想的概率。",
                "Chance per word of stopping to think.",
                0.0,
                100.0,
                0.1,
                1,
                "%"
            ),
            p!(
                long_pause_ms,
                "长停顿时长",
                "Long-stop length",
                "长停顿的中位时长。",
                "Median length of a long stop.",
                0.0,
                60000.0,
                100.0,
                0,
                "ms"
            ),
        ],
    },
    Group {
        zh: "疲劳",
        en: "Fatigue",
        params: &[
            p!(
                fatigue_slowdown_percent,
                "变慢",
                "Slowdown",
                "每打 1000 个字符慢下来的百分比，最多 30%。",
                "Slowdown per 1000 typed characters, up to 30 %.",
                0.0,
                100.0,
                0.5,
                1,
                "%"
            ),
            p!(
                fatigue_error_percent,
                "错误增多",
                "More mistakes",
                "每打 1000 个字符错误率上升的百分比，最多 50%。",
                "Rise in mistakes per 1000 typed characters, up to 50 %.",
                0.0,
                100.0,
                0.5,
                1,
                "%"
            ),
        ],
    },
    Group {
        zh: "错误",
        en: "Mistakes",
        params: &[
            p!(
                typo_percent,
                "错字",
                "Wrong key",
                "每个字符按到相邻键的概率。数字不会打错。",
                "Chance per character of hitting an adjacent key. Digits are never mistyped.",
                0.0,
                100.0,
                0.1,
                1,
                "%"
            ),
            p!(
                omission_percent,
                "漏字",
                "Skipped character",
                "每个字符被漏掉的概率。",
                "Chance per character of skipping it.",
                0.0,
                100.0,
                0.1,
                1,
                "%"
            ),
            p!(
                transpose_percent,
                "颠倒",
                "Swapped pair",
                "和下一个字符顺序颠倒的概率。",
                "Chance of swapping a character with the next one.",
                0.0,
                100.0,
                0.1,
                1,
                "%"
            ),
            p!(
                double_press_percent,
                "多按一次",
                "Double press",
                "一个键被按了两次的概率。",
                "Chance of pressing a key twice.",
                0.0,
                100.0,
                0.1,
                1,
                "%"
            ),
            p!(
                case_slip_percent,
                "大小写失误",
                "Shift slip",
                "大写字母前后 Shift 没按准的概率。",
                "Chance per capital letter of mistiming Shift.",
                0.0,
                100.0,
                0.1,
                1,
                "%"
            ),
            p!(
                word_redo_percent,
                "整词重打",
                "Word redone",
                "4 个字母以上的词打乱后删掉重打的概率。",
                "Chance per word of 4+ letters of fumbling it and starting over.",
                0.0,
                100.0,
                0.1,
                1,
                "%"
            ),
        ],
    },
    Group {
        zh: "修正",
        en: "Corrections",
        params: &[
            p!(
                repair_percent,
                "修正率",
                "Fix rate",
                "错误被发现并改正的比例；没改的会留在文字里。",
                "Share of mistakes that get fixed; the rest stay in the text.",
                0.0,
                100.0,
                0.5,
                1,
                "%"
            ),
            p!(
                notice_delay_ms,
                "发现延迟",
                "Noticing time",
                "打错后多久发现。",
                "How long until a mistake is noticed.",
                0.0,
                10000.0,
                10.0,
                0,
                "ms"
            ),
            p!(
                lookahead_percent,
                "继续打一两个字",
                "Typing on",
                "发现错误之前又多打 1–2 个字符的概率。",
                "Chance of typing 1–2 more characters before noticing.",
                0.0,
                100.0,
                1.0,
                0,
                "%"
            ),
            p!(
                backspace_interval_ms,
                "退格间隔",
                "Backspace interval",
                "连续退格时两次之间的中位时间。",
                "Median time between repeated backspaces.",
                30.0,
                1000.0,
                5.0,
                0,
                "ms"
            ),
        ],
    },
];

pub struct ClassicParam {
    pub zh: &'static str,
    pub en: &'static str,
    pub max: u32,
    pub unit: &'static str,
    pub get: fn(&TypingSettings) -> u32,
    pub set: fn(&mut TypingSettings, u32),
}

macro_rules! c {
    ($field:ident, $zh:literal, $en:literal, $max:expr, $unit:literal) => {
        ClassicParam { zh: $zh, en: $en, max: $max, unit: $unit, get: |s| s.$field, set: |s, v| s.$field = v }
    };
}

/// The original model's eight values (lead-in lives in Settings).
pub static CLASSIC: &[ClassicParam] = &[
    c!(base_delay_ms, "基础延迟", "Base delay", 2000, "ms"),
    c!(jitter_percent, "抖动", "Jitter", 100, "%"),
    c!(typo_rate_percent, "错字率", "Typo rate", 100, "%"),
    c!(omission_rate_percent, "漏写率", "Omission rate", 100, "%"),
    c!(transpose_rate_percent, "颠倒率", "Transpose rate", 100, "%"),
    c!(repair_rate_percent, "修复率", "Repair rate", 100, "%"),
    c!(error_detect_delay_ms, "错误发现延迟", "Error detect delay", 10000, "ms"),
    c!(backspace_delay_ms, "回删延迟", "Backspace delay", 2000, "ms"),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_parameter_round_trips_and_defaults_are_in_range() {
        let base = HumanModel::default();
        let mut seen = 0;
        for group in HUMAN {
            for p in group.params {
                let v = (p.get)(&base);
                assert!((p.min..=p.max).contains(&v), "{} = {v}", p.en);
                let mut m = base;
                (p.set)(&mut m, p.min);
                assert_eq!((p.get)(&m), p.min, "{}", p.en);
                seen += 1;
            }
        }
        // All numeric fields of HumanModel (everything but word_delete and lead_in_ms).
        assert_eq!(seen, 28);
        assert_eq!(CLASSIC.len(), 8);
    }
}
