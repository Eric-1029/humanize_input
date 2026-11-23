use std::sync::Arc;
use std::time::Duration;

use super::*;
use crate::editor::{MAX_SAFE_HOLD, VirtualEditor};

const SAMPLE: &str = "The Quick brown fox, said HELLO to the lazy dog. Then it left!\n\
    fn main() { println!(\"x = {}\", 42 * 7); }\n\n\tIndented: a_b-c+d=e [f] <g> ~h| 'i' `j`?";

fn layout() -> Arc<Layout> {
    Arc::new(Layout::us_qwerty())
}

fn no_errors(model: HumanModel) -> HumanModel {
    HumanModel {
        typo_percent: 0.0,
        omission_percent: 0.0,
        transpose_percent: 0.0,
        double_press_percent: 0.0,
        case_slip_percent: 0.0,
        word_redo_percent: 0.0,
        ..model
    }
}

fn sloppy(model: HumanModel) -> HumanModel {
    HumanModel {
        typo_percent: 8.0,
        omission_percent: 5.0,
        transpose_percent: 5.0,
        double_press_percent: 5.0,
        case_slip_percent: 30.0,
        word_redo_percent: 25.0,
        repair_percent: 100.0,
        ..model
    }
}

struct Run {
    editor: VirtualEditor,
    actions: Vec<KeyAction>,
    progress: TypingProgress,
}

fn run(text: &str, model: HumanModel, seed: u64) -> Run {
    let layout = layout();
    let actions: Vec<KeyAction> = HumanPlanner::with_seed(text, model, Arc::clone(&layout), seed).collect();
    let mut editor = VirtualEditor::new(layout);
    editor.apply_all(actions.iter().copied());
    let progress = actions
        .iter()
        .rev()
        .find_map(|a| match a {
            KeyAction::Progress(p) => Some(*p),
            _ => None,
        })
        .unwrap_or_default();
    Run { editor, actions, progress }
}

fn assert_clean(r: &Run, context: &str) {
    assert!(r.editor.violations().is_empty(), "{context}: {:?}", r.editor.violations());
    assert!(r.editor.held().is_empty(), "{context}: keys left down: {:?}", r.editor.held());
    assert!(r.editor.longest_hold() <= MAX_SAFE_HOLD, "{context}");
}

#[test]
fn without_mistakes_every_preset_types_text_verbatim() {
    for preset in Preset::ALL {
        for seed in 0..20 {
            let r = run(SAMPLE, no_errors(preset.model()), seed);
            let ctx = format!("{preset:?} seed {seed}");
            assert_eq!(r.editor.text(), SAMPLE, "{ctx}");
            assert_clean(&r, &ctx);
            assert_eq!(r.progress.typed_chars, SAMPLE.chars().count(), "{ctx}");
            assert!(!r.actions.contains(&KeyAction::Press(KeyCode::BACKSPACE)), "{ctx}");
        }
    }
}

#[test]
fn full_repair_reproduces_text_with_every_mistake_kind() {
    let mut totals = TypingProgress::default();
    for preset in Preset::ALL {
        for seed in 0..60 {
            let r = run(SAMPLE, sloppy(preset.model()), seed);
            let ctx = format!("{preset:?} seed {seed}");
            assert_eq!(r.editor.text(), SAMPLE, "{ctx}");
            assert_clean(&r, &ctx);
            let p = r.progress;
            totals.typo_count += p.typo_count;
            totals.omission_count += p.omission_count;
            totals.transpose_count += p.transpose_count;
            totals.double_press_count += p.double_press_count;
            totals.case_slip_count += p.case_slip_count;
            totals.word_redo_count += p.word_redo_count;
        }
    }
    // Every mistake kind actually happened somewhere.
    for (name, n) in [
        ("typo", totals.typo_count),
        ("omission", totals.omission_count),
        ("transpose", totals.transpose_count),
        ("double", totals.double_press_count),
        ("case", totals.case_slip_count),
        ("redo", totals.word_redo_count),
    ] {
        assert!(n > 0, "no {name} mistakes generated");
    }
}

#[test]
fn backspace_word_delete_style_also_reproduces_text() {
    let model = HumanModel { word_delete: WordDelete::Backspaces, ..sloppy(Preset::Average.model()) };
    for seed in 0..40 {
        let r = run(SAMPLE, model, seed);
        assert_eq!(r.editor.text(), SAMPLE, "seed {seed}");
        assert_clean(&r, "backspaces");
        assert!(!r.actions.contains(&KeyAction::Press(KeyCode::LEFT_CTRL)));
    }
}

#[test]
fn word_redo_uses_ctrl_backspace_on_eligible_words_only() {
    let model = HumanModel {
        word_redo_percent: 100.0,
        word_delete: WordDelete::CtrlBackspace,
        ..no_errors(Preset::Average.model())
    };
    let r = run("the quick brown foxes, jumped over", model, 3);
    assert_eq!(r.editor.text(), "the quick brown foxes, jumped over");
    assert_clean(&r, "redo");
    // quick, brown, foxes, jumped, over (first word and short words excluded).
    assert_eq!(r.progress.word_redo_count, 5);
    let ctrl_presses = r.actions.iter().filter(|a| **a == KeyAction::Press(KeyCode::LEFT_CTRL)).count();
    assert_eq!(ctrl_presses, 5);
}

#[test]
fn same_seed_same_actions() {
    let model = sloppy(Preset::Average.model());
    let a: Vec<_> = HumanPlanner::with_seed(SAMPLE, model, layout(), 9).collect();
    let b: Vec<_> = HumanPlanner::with_seed(SAMPLE, model, layout(), 9).collect();
    let c: Vec<_> = HumanPlanner::with_seed(SAMPLE, model, layout(), 10).collect();
    assert_eq!(a, b);
    assert_ne!(a, c);
}

#[test]
fn unrepaired_mistakes_stay_in_the_text() {
    let model = HumanModel { typo_percent: 30.0, repair_percent: 0.0, ..no_errors(Preset::Average.model()) };
    let text = "keyboards are hard";
    let r = run(text, model, 5);
    assert_ne!(r.editor.text(), text);
    assert_eq!(r.editor.text().len(), text.len());
    assert!(r.progress.typo_count > 0);
    assert_eq!(r.progress.correction_count, 0);
    assert_clean(&r, "unrepaired");
}

#[test]
fn skipped_characters_are_noticed_after_the_next_one() {
    let model =
        HumanModel { omission_percent: 100.0, repair_percent: 100.0, ..no_errors(Preset::Average.model()) };
    let text = "abcdefgh";
    let r = run(text, model, 1);
    assert_eq!(r.editor.text(), text);
    let backspaces = r.actions.iter().filter(|a| **a == KeyAction::Press(KeyCode::BACKSPACE)).count();
    // Each skip is followed by ≥ 1 more char that must be erased, except a
    // skip of the very last character.
    assert!(backspaces + 1 >= r.progress.omission_count, "{backspaces} < {}", r.progress.omission_count);
}

#[test]
fn character_presses_are_never_instant() {
    let r = run(SAMPLE, sloppy(Preset::Professional.model()), 2);
    let mut since_last: Option<Duration> = None;
    for action in &r.actions {
        match action {
            KeyAction::Wait(d) => since_last = since_last.map(|s| s + *d),
            KeyAction::Press(k) if !k.is_modifier() => {
                if let Some(gap) = since_last {
                    assert!(gap >= Duration::from_millis(25), "presses {gap:?} apart");
                }
                since_last = Some(Duration::ZERO);
            }
            _ => {}
        }
    }
}

#[test]
fn fast_typists_roll_over_keys() {
    let r = run(SAMPLE, no_errors(Preset::Professional.model()), 4);
    assert!(r.editor.rollovers() > 0);
    assert_clean(&r, "rollover");
}

#[test]
fn shift_comes_from_the_other_hand_and_stays_down_across_capitals() {
    let model =
        HumanModel { word_pause_ms: 0.0, long_pause_percent: 0.0, ..no_errors(Preset::Professional.model()) };
    // 'a' is a left-hand key → right Shift.
    let r = run("A", model, 0);
    assert_eq!(
        r.actions.iter().find(|a| matches!(a, KeyAction::Press(_))),
        Some(&KeyAction::Press(KeyCode::RIGHT_SHIFT))
    );

    let r = run("HELLO", model, 0);
    let shift_presses = r
        .actions
        .iter()
        .filter(|a| matches!(a, KeyAction::Press(KeyCode::LEFT_SHIFT | KeyCode::RIGHT_SHIFT)))
        .count();
    assert_eq!(r.editor.text(), "HELLO");
    assert_eq!(shift_presses, 1);
}

fn mean_elapsed(text: &str, model: HumanModel) -> Duration {
    (0..6).map(|seed| run(text, model, seed).editor.elapsed()).sum::<Duration>() / 6
}

#[test]
fn presets_are_ordered_by_speed_and_accuracy() {
    let text = SAMPLE.repeat(3);
    let times: Vec<Duration> = Preset::ALL.iter().map(|p| mean_elapsed(&text, p.model())).collect();
    assert!(times.windows(2).all(|w| w[0] > w[1]), "{times:?}");

    let mistakes = |p: Preset| {
        (0..10)
            .map(|seed| {
                let q = run(&text, p.model(), seed).progress;
                q.typo_count + q.omission_count + q.transpose_count + q.double_press_count + q.case_slip_count
            })
            .sum::<usize>()
    };
    assert!(mistakes(Preset::Novice) > mistakes(Preset::Professional));
}

#[test]
fn macro_speed_and_fatigue_change_duration() {
    let base = no_errors(Preset::Average.model());
    let text = SAMPLE.repeat(2);
    let slow = mean_elapsed(&text, base.with_macros(Macros { speed: 0, ..Macros::default() }));
    let fast = mean_elapsed(&text, base.with_macros(Macros { speed: 100, ..Macros::default() }));
    assert!(slow > fast * 3, "{slow:?} vs {fast:?}");

    let long = "lorem ipsum dolor sit amet ".repeat(120);
    let rested = HumanModel { fatigue_slowdown_percent: 0.0, ..base };
    let tired = HumanModel { fatigue_slowdown_percent: 20.0, ..base };
    assert!(mean_elapsed(&long, tired) > mean_elapsed(&long, rested));
}

#[test]
fn empty_text_yields_nothing() {
    assert_eq!(HumanPlanner::with_seed("", HumanModel::default(), layout(), 0).count(), 0);
}
