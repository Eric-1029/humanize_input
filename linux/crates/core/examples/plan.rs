//! Prints the action sequence for a text without sending any keys.
//!
//! ```sh
//! cargo run -p humanize-core --example plan -- "Hello, world" [seed]
//! ```

use std::sync::Arc;
use std::time::Duration;

use humanize_core::{Action, KeyboardGeometry, Planner, TypingSettings};

fn main() {
    let mut args = std::env::args().skip(1);
    let text = args.next().unwrap_or_else(|| "The quick brown fox jumps over the lazy dog.".into());
    let seed: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(1);

    let planner =
        Planner::with_seed(&text, TypingSettings::default(), Arc::new(KeyboardGeometry::us_qwerty()), seed);
    let mut elapsed = Duration::ZERO;
    let mut buffer = String::new();

    for action in planner {
        match action {
            Action::Type(c) => {
                buffer.push(c);
                println!("{:>8.3}s  type  {c:?}", elapsed.as_secs_f64());
            }
            Action::Backspace => {
                buffer.pop();
                println!("{:>8.3}s  back", elapsed.as_secs_f64());
            }
            Action::Wait(d) => elapsed += d,
            Action::Progress(p) => println!(
                "          progress {}/{}  typo {}  omit {}  swap {}  fixed {}",
                p.typed_chars,
                p.total_chars,
                p.typo_count,
                p.omission_count,
                p.transpose_count,
                p.correction_count
            ),
        }
    }

    println!("\nresult: {buffer:?}");
    println!("total:  {:.2}s", elapsed.as_secs_f64());
}
