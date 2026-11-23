//! Prints the key events for a text without sending anything.
//!
//! ```sh
//! cargo run -p humanize-core --example plan -- [novice|average|proficient|professional|classic] "Hello, World" [seed]
//! ```

use std::sync::Arc;

use humanize_core::{
    ClassicKeys, HumanPlanner, KeyAction, Layout, Planner, Preset, TypingProgress, TypingSettings,
    VirtualEditor,
};

fn main() {
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_else(|| "average".into());
    let text = args.next().unwrap_or_else(|| "The quick brown Fox jumps over the lazy dog.".into());
    let seed: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1);

    let layout = Arc::new(Layout::us_qwerty());
    let actions: Vec<KeyAction> = match mode.as_str() {
        "classic" => {
            let planner = Planner::with_seed(
                &text,
                TypingSettings::default(),
                Arc::new(layout.classic_geometry()),
                seed,
            );
            ClassicKeys::new(planner, Arc::clone(&layout)).collect()
        }
        name => {
            let preset = match name {
                "novice" => Preset::Novice,
                "proficient" => Preset::Proficient,
                "professional" => Preset::Professional,
                _ => Preset::Average,
            };
            HumanPlanner::with_seed(&text, preset.model(), Arc::clone(&layout), seed).collect()
        }
    };

    let mut editor = VirtualEditor::new(Arc::clone(&layout));
    let mut last = TypingProgress::default();
    for action in &actions {
        editor.apply(*action);
        let t = editor.elapsed().as_secs_f64();
        match action {
            KeyAction::Press(k) => println!("{t:>8.3}s  down {:<3} {:?}", k.0, editor.text()),
            KeyAction::Release(k) => println!("{t:>8.3}s  up   {:<3}", k.0),
            KeyAction::Progress(p) => last = *p,
            KeyAction::Wait(_) => {}
        }
    }

    let p = last;
    println!("\nresult:   {:?}", editor.text());
    println!(
        "time:     {:.2}s  ({} key presses, {} rollovers)",
        editor.elapsed().as_secs_f64(),
        editor.presses(),
        editor.rollovers()
    );
    println!(
        "mistakes: typo {}  omit {}  swap {}  double {}  case {}  redo {}  fixed {}",
        p.typo_count,
        p.omission_count,
        p.transpose_count,
        p.double_press_count,
        p.case_slip_count,
        p.word_redo_count,
        p.correction_count
    );
    if !editor.violations().is_empty() {
        println!("violations: {:?}", editor.violations());
    }
}
