//! Searches seeds for outputs whose length change cannot be explained by
//! unrepaired omissions/double presses (a sign of a planning bug).
use humanize_core::{HumanModel, HumanPlanner, KeyAction, Layout, Preset, VirtualEditor, WordDelete};
use std::sync::Arc;

fn main() {
    let text = std::fs::read_to_string(std::env::args().nth(1).unwrap()).unwrap();
    let layout = Arc::new(Layout::us_qwerty());
    let mut bad = 0;
    for wd in [WordDelete::Backspaces, WordDelete::CtrlBackspace] {
        for seed in 0..20000u64 {
            let model = HumanModel { word_delete: wd, ..Preset::Average.model() };
            let actions: Vec<KeyAction> =
                HumanPlanner::with_seed(&text, model, Arc::clone(&layout), seed).collect();
            let p = actions
                .iter()
                .rev()
                .find_map(|a| if let KeyAction::Progress(p) = a { Some(*p) } else { None })
                .unwrap();
            let mut e = VirtualEditor::new(Arc::clone(&layout));
            e.apply_all(actions.iter().copied());
            let diff = e.text().chars().count() as i64 - text.chars().count() as i64;
            if diff < -(p.omission_count as i64)
                || diff > p.double_press_count as i64
                || !e.violations().is_empty()
            {
                bad += 1;
                if bad <= 5 {
                    println!(
                        "{wd:?} seed {seed}: diff {diff} omissions {} doubles {} redo {}\n  {:?}",
                        p.omission_count,
                        p.double_press_count,
                        p.word_redo_count,
                        e.text()
                    );
                }
            }
        }
    }
    println!("suspicious: {bad} / 40000");
}
