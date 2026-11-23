//! Running counters for one session.

/// Counters reported after every planned step.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TypingProgress {
    /// Characters in the (normalized) source text.
    pub total_chars: usize,
    /// Source characters already handled, including skipped ones.
    pub typed_chars: usize,
    pub typo_count: usize,
    pub omission_count: usize,
    pub transpose_count: usize,
    /// Key pressed twice (human model only).
    pub double_press_count: usize,
    /// Shift timing slips: wrong letter case (human model only).
    pub case_slip_count: usize,
    /// Words fumbled, erased and retyped (human model only).
    pub word_redo_count: usize,
    /// Mistakes that were noticed and fixed.
    pub correction_count: usize,
}
