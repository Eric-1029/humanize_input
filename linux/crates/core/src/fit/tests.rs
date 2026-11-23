use super::*;
use crate::human::{Preset, WordDelete};

/// About 1,300 characters of ordinary prose with capitals and punctuation.
pub(crate) const PROMPT: &str = "The morning train was late again, so Maria opened her laptop and started on the report. \
She had promised the team a first draft by Friday, and the numbers from March still did not add up. \
Every few minutes a new message arrived: questions about the budget, a reminder about the meeting, \
and a photo of somebody's lunch. She muted the chat and kept typing.\n\
By the time the train reached the city, the summary was done. The figures for April looked better than expected; \
sales in the north had grown by almost a third, while costs stayed flat. Maria wrote a short note to explain the \
difference and saved the file twice, just to be safe.\n\
At the office, the coffee machine was broken. Somebody had taped a sign to it that read Out of Order, \
which nobody seemed to believe. A small queue formed anyway, and people talked about the weekend, \
the weather, and the new project that would start next month. Maria smiled, took a cup of tea instead, \
and walked to her desk to read the comments from her manager. Most of them were kind. One of them asked \
for a chart, which meant another hour of work, but she did not mind. The report was nearly finished, \
and the rest of the day looked calm.";

fn layout() -> Arc<Layout> {
    Arc::new(Layout::us_qwerty())
}

fn sample(model: HumanModel, seed: u64) -> Vec<SampleEvent> {
    simulate(PROMPT, model, &layout(), seed)
}

fn within(field: &str, got: f64, want: f64, tolerance: f64) {
    let err = (got - want).abs() / want.abs().max(1e-9);
    assert!(err <= tolerance, "{field}: fitted {got:.3}, true {want:.3} ({:.0}% off)", err * 100.0);
}

#[test]
fn recovers_rhythm_and_key_timing_of_a_known_typist() {
    // A typist unlike the base model, so calibration has to move values.
    let truth = HumanModel {
        wpm: 68.0,
        interval_sigma: 0.30,
        dwell_ms: 118.0,
        dwell_sigma: 0.20,
        modifier_lead_ms: 75.0,
        word_pause_ms: 90.0,
        sentence_pause_ms: 700.0,
        punctuation_pause_ms: 260.0,
        long_pause_percent: 0.0,
        backspace_interval_ms: 120.0,
        fatigue_slowdown_percent: 0.0,
        fatigue_error_percent: 0.0,
        ..Preset::Average.model()
    };
    let base = Preset::Average.model();
    let report = fit(PROMPT, &sample(truth, 7), &layout(), base);
    let get = |f: &str| report.item(f).unwrap_or_else(|| panic!("{f} not measured"));
    for (field, want, tol) in [
        ("wpm", truth.wpm, 0.12),
        ("dwell_ms", truth.dwell_ms, 0.10),
        ("dwell_sigma", truth.dwell_sigma, 0.30),
        ("modifier_lead_ms", truth.modifier_lead_ms, 0.25),
        ("word_pause_ms", truth.word_pause_ms, 0.45),
        ("sentence_pause_ms", truth.sentence_pause_ms, 0.40),
        ("punctuation_pause_ms", truth.punctuation_pause_ms, 0.45),
    ] {
        let item = get(field);
        assert!(item.reliable, "{field} unreliable with {} samples", item.samples);
        within(field, item.value, want, tol);
    }
    within("interval_sigma", get("interval_sigma").value, truth.interval_sigma, 0.35);
    assert!(report.rollover_percent >= 0.0 && report.key_presses > 1000);
}

#[test]
fn recovers_mistake_rates_and_corrections() {
    let truth = HumanModel {
        typo_percent: 3.0,
        omission_percent: 1.5,
        transpose_percent: 1.0,
        double_press_percent: 1.0,
        case_slip_percent: 8.0,
        word_redo_percent: 0.0,
        repair_percent: 85.0,
        notice_delay_ms: 600.0,
        backspace_interval_ms: 130.0,
        word_delete: WordDelete::Backspaces,
        fatigue_slowdown_percent: 0.0,
        fatigue_error_percent: 0.0,
        ..Preset::Average.model()
    };
    // Pool several runs: one sample holds only a few dozen mistakes.
    let (mut sums, mut n) = (HashMap::<&str, f64>::new(), 0.0);
    for seed in 0..6 {
        let report = fit(PROMPT, &sample(truth, seed), &layout(), Preset::Average.model());
        for item in &report.items {
            *sums.entry(item.field).or_default() += item.value;
        }
        n += 1.0;
    }
    let avg = |f: &str| sums[f] / n;
    within("typo_percent", avg("typo_percent"), truth.typo_percent, 0.35);
    within("omission_percent", avg("omission_percent"), truth.omission_percent, 0.45);
    within("transpose_percent", avg("transpose_percent"), truth.transpose_percent, 0.5);
    within("double_press_percent", avg("double_press_percent"), truth.double_press_percent, 0.5);
    within("repair_percent", avg("repair_percent"), truth.repair_percent, 0.12);
    within("notice_delay_ms", avg("notice_delay_ms"), truth.notice_delay_ms, 0.35);
    within("backspace_interval_ms", avg("backspace_interval_ms"), truth.backspace_interval_ms, 0.25);
}

#[test]
fn clean_sample_reports_no_mistakes_and_full_accuracy() {
    let truth = HumanModel {
        typo_percent: 0.0,
        omission_percent: 0.0,
        transpose_percent: 0.0,
        double_press_percent: 0.0,
        case_slip_percent: 0.0,
        word_redo_percent: 0.0,
        ..Preset::Proficient.model()
    };
    let report = fit(PROMPT, &sample(truth, 3), &layout(), Preset::Average.model());
    assert_eq!(report.mistakes, 0);
    assert_eq!(report.accuracy, 100.0);
    assert_eq!(report.item("typo_percent").unwrap().value, 0.0);
}

#[test]
fn short_samples_are_marked_unreliable_and_leave_the_base_alone() {
    let short = &PROMPT[..60];
    let events = simulate(short, Preset::Novice.model(), &layout(), 1);
    let base = Preset::Average.model();
    let report = fit(short, &events, &layout(), base);
    // Per-character mistake rates need a longer sample.
    let rates = [
        "typo_percent",
        "omission_percent",
        "transpose_percent",
        "double_press_percent",
        "case_slip_percent",
        "word_redo_percent",
    ];
    assert!(report.items.iter().filter(|i| rates.contains(&i.field)).all(|i| !i.reliable));
    assert_eq!(report.model.typo_percent, base.typo_percent);
}

#[test]
fn classifies_each_kind_of_fixed_mistake() {
    let e: Vec<char> = "abcdef".chars().collect();
    let c = |kept: &str, wrong: &str| {
        let kept: Vec<char> = kept.chars().collect();
        let wrong: Vec<char> = wrong.chars().collect();
        classify(&kept, &wrong, &e[kept.len()..]).unwrap()
    };
    assert_eq!(c("ab", "x"), (Mistake::Typo, 0));
    assert_eq!(c("ab", "xde"), (Mistake::Typo, 2));
    assert_eq!(c("ab", "de"), (Mistake::Omission, 2));
    assert_eq!(c("ab", "dc"), (Mistake::Transpose, 0));
    assert_eq!(c("ab", "bc"), (Mistake::Double, 1));
    assert_eq!(c("ab", "C"), (Mistake::Case, 0));
}

#[test]
fn unrepaired_mistakes_are_aligned() {
    let p: Vec<char> = "the quick fox".chars().collect();
    let t: Vec<char> = "teh quikc foox".chars().collect();
    let counts = unrepaired(&p, &t);
    assert_eq!(counts.get("transpose"), Some(&2));
    assert_eq!(counts.get("double"), Some(&1));
}
