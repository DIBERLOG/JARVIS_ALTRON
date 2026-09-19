//! Applying corrections, undoing them, and showing the difference.
//!
//! Two rules make this module safe to point at a user's text:
//!
//! * **Nothing is applied blind.** Every correction carries the text it expects to find
//!   at its range, and the caller passes the version of the text it checked. A
//!   correction whose range no longer holds that text is skipped and reported, never
//!   guessed at, so a keystroke typed while the panel was open cannot move an edit onto
//!   the wrong characters.
//! * **Undo is exact.** The journal stores the whole text that preceded each applied
//!   batch, so undo restores those exact bytes instead of trying to invert a range. It
//!   refuses when the text changed afterwards, because then the user's own edit would be
//!   thrown away together with the correction.
//!
//! Corrections are applied as one atomic batch: the interface sends everything the user
//! accepted, gets back the new text and the record of what happened, and stores that
//! text. The journal keeps the last [`MAX_JOURNAL_BATCHES`] batches in memory only — a
//! correction history is not written to disk, so nothing about the user's text outlives
//! the session.

use std::collections::VecDeque;

use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::error::AutocorrectError;
use super::model::{
    text_version, AppliedCorrection, Correction, CorrectionBatch, CorrectionOutcome, IssueReason,
    SkippedCorrection, TextRange, UndoOutcome,
};

/// How many applied batches the in-memory undo journal keeps.
pub const MAX_JOURNAL_BATCHES: usize = 20;
/// How many difference segments a preview renders before it summarises.
pub const MAX_DIFF_SEGMENTS: usize = 400;

/// The applied corrections of this session, newest last.
#[derive(Debug, Default)]
pub struct CorrectionJournal {
    batches: VecDeque<CorrectionBatch>,
    /// Whether the journal reached its cap at least once.
    dropped: usize,
}

impl CorrectionJournal {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records an applied batch. A batch that changed nothing is not recorded.
    pub fn record(&mut self, batch: CorrectionBatch) -> bool {
        if batch.applied.is_empty() || batch.before == batch.after {
            return false;
        }
        self.batches.push_back(batch);
        while self.batches.len() > MAX_JOURNAL_BATCHES {
            self.batches.pop_front();
            self.dropped += 1;
        }
        true
    }

    /// The batch an undo would revert.
    pub fn last(&self) -> Option<&CorrectionBatch> {
        self.batches.back()
    }

    /// Whether an undo is possible at all.
    pub fn can_undo(&self) -> bool {
        !self.batches.is_empty()
    }

    /// How many applied batches can still be undone.
    pub fn len(&self) -> usize {
        self.batches.len()
    }

    pub fn is_empty(&self) -> bool {
        self.batches.is_empty()
    }

    /// How many batches fell off the end of the journal.
    pub fn dropped(&self) -> usize {
        self.dropped
    }

    pub fn clear(&mut self) {
        self.batches.clear();
    }

    /// The words of the batch an undo would revert, for a label in the interface.
    pub fn last_words(&self) -> Vec<String> {
        self.last()
            .map(|batch| {
                batch
                    .applied
                    .iter()
                    .map(|correction| correction.after.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Applies a set of corrections to a text.
///
/// Returns the batch that was applied, including the full text before and after, so the
/// caller can show a preview, store the result, and offer undo. Refuses a stale text
/// outright; skips individual corrections that no longer match and reports them in
/// [`CorrectionBatch::skipped`].
pub fn apply_corrections(
    text: &str,
    corrections: &[Correction],
    expected_version: Option<&str>,
    journal: &mut CorrectionJournal,
) -> Result<CorrectionBatch, AutocorrectError> {
    let version_before = text_version(text);
    if let Some(expected) = expected_version {
        if expected != version_before {
            return Err(AutocorrectError::StaleText);
        }
    }

    let total_chars = text.chars().count();
    let mut ordered: Vec<&Correction> = corrections.iter().collect();
    ordered.sort_by_key(|correction| (correction.range.start, correction.range.end));

    let mut after = String::with_capacity(text.len());
    let mut byte_cursor = 0usize;
    let mut after_chars = 0usize;
    let mut applied: Vec<AppliedCorrection> = Vec::new();
    let mut skipped: Vec<SkippedCorrection> = Vec::new();
    let mut last_end: Option<usize> = None;

    for correction in ordered {
        let range = correction.range;
        let overlapping = last_end.is_some_and(|end| range.start < end);
        let slice = if !overlapping && range.end <= total_chars {
            range.slice(text).ok()
        } else {
            None
        };
        match slice {
            Some(original) if original == correction.original => {}
            Some(_) => {
                skipped.push(SkippedCorrection {
                    range,
                    original: correction.original.clone(),
                    outcome: CorrectionOutcome::Mismatched,
                });
                continue;
            }
            None => {
                skipped.push(SkippedCorrection {
                    range,
                    original: correction.original.clone(),
                    outcome: if overlapping {
                        CorrectionOutcome::Overlapped
                    } else {
                        CorrectionOutcome::Mismatched
                    },
                });
                continue;
            }
        }

        // Every range in the batch is a character range on the same text, so the byte
        // offsets are found the same way the tokenizer found them.
        let Some(bytes) = range.to_bytes(text) else {
            skipped.push(SkippedCorrection {
                range,
                original: correction.original.clone(),
                outcome: CorrectionOutcome::Mismatched,
            });
            continue;
        };
        after.push_str(&text[byte_cursor..bytes.start]);
        after_chars += text[byte_cursor..bytes.start].chars().count();
        let replacement = correction.replacement.clone();
        let replacement_chars = replacement.chars().count();
        after.push_str(&replacement);
        let range_after = TextRange::new(after_chars, after_chars + replacement_chars);
        after_chars += replacement_chars;
        byte_cursor = bytes.end;
        last_end = Some(range.end);

        applied.push(AppliedCorrection {
            id: Uuid::new_v4(),
            before: correction.original.clone(),
            after: replacement,
            range: range_after,
            version_before: version_before.clone(),
            version_after: String::new(),
            applied_at: Utc::now().to_rfc3339(),
            reason: correction.reason.unwrap_or(IssueReason::UnknownWord),
        });
    }
    after.push_str(&text[byte_cursor..]);

    let version_after = text_version(&after);
    for correction in &mut applied {
        correction.version_after = version_after.clone();
    }

    let batch = CorrectionBatch {
        before: text.to_string(),
        after,
        applied,
        skipped,
        version_before,
        version_after,
    };
    // A batch that changed nothing cannot be undone, so it is not journalled.
    journal.record(batch.clone());
    Ok(batch)
}

/// Reverts the most recent applied batch.
///
/// The text must still be exactly what that batch produced. Otherwise the user has
/// edited it since, and the undo is refused rather than reverted to a state that would
/// lose their work.
pub fn undo_last(
    text: &str,
    journal: &mut CorrectionJournal,
) -> Result<UndoOutcome, AutocorrectError> {
    let Some(batch) = journal.last() else {
        return Err(AutocorrectError::NotFound);
    };
    let version_before = text_version(text);
    if version_before != batch.version_after {
        return Err(AutocorrectError::UndoConflict);
    }
    let restored = batch.applied.clone();
    let restored_text = batch.before.clone();
    let version_after = batch.version_before.clone();
    journal.batches.pop_back();
    Ok(UndoOutcome {
        text: restored_text,
        restored,
        version_before,
        version_after,
        remaining: journal.len(),
    })
}

/// What a difference segment is.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffKind {
    /// Present in both texts, unchanged.
    Same,
    /// Only in the new text.
    Added,
    /// Only in the old text.
    Removed,
}

impl DiffKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Same => "same",
            Self::Added => "added",
            Self::Removed => "removed",
        }
    }
}

/// One run of the comparison between two texts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DiffSegment {
    pub kind: DiffKind,
    pub text: String,
}

/// The whole comparison, with counts the interface can show instead of the text.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct TextDiff {
    pub segments: Vec<DiffSegment>,
    pub added_words: usize,
    pub removed_words: usize,
    /// Whether the segment list was cut short; the counts are still exact.
    pub truncated: bool,
}

impl TextDiff {
    /// Whether the two texts differ at all.
    pub fn has_changes(&self) -> bool {
        self.added_words > 0 || self.removed_words > 0
    }
}

/// Splits a text into comparable units: one word with the spacing around it.
///
/// Joining every unit back together reproduces the original text exactly, which is what
/// lets a preview show the differences without re-rendering the text from pieces.
fn units(text: &str) -> Vec<&str> {
    let characters: Vec<(usize, char)> = text.char_indices().collect();
    let mut units = Vec::new();
    let mut index = 0usize;
    while index < characters.len() {
        let start = characters[index].0;
        while index < characters.len() && characters[index].1.is_whitespace() {
            index += 1;
        }
        while index < characters.len() && !characters[index].1.is_whitespace() {
            index += 1;
        }
        while index < characters.len() && characters[index].1.is_whitespace() {
            index += 1;
        }
        let end = if index < characters.len() {
            characters[index].0
        } else {
            text.len()
        };
        units.push(&text[start..end]);
    }
    units
}

/// Compares two texts word by word.
///
/// The comparison is on whole words, so a preview shows "this word became that word"
/// instead of a character-level smear, and the counts are word counts.
pub fn diff_texts(before: &str, after: &str) -> TextDiff {
    let left = units(before);
    let right = units(after);
    let (left_map, right_map) = seqdiff::diff(&left, &right);

    // Walk the two sequences together, emitting runs of the same kind.
    let mut segments: Vec<DiffSegment> = Vec::new();
    let mut added_words = 0usize;
    let mut removed_words = 0usize;
    let mut left_index = 0usize;
    let mut right_index = 0usize;

    let push = |kind: DiffKind, text: &str, segments: &mut Vec<DiffSegment>| {
        if text.is_empty() {
            return;
        }
        match segments.last_mut() {
            Some(last) if last.kind == kind => last.text.push_str(text),
            _ => segments.push(DiffSegment {
                kind,
                text: text.to_string(),
            }),
        }
    };

    while left_index < left.len() || right_index < right.len() {
        match (
            left_map.get(left_index).copied().flatten(),
            right_map.get(right_index).copied().flatten(),
        ) {
            (Some(matched), Some(_)) if matched == right_index => {
                push(
                    DiffKind::Same,
                    left.get(left_index).copied().unwrap_or(""),
                    &mut segments,
                );
                left_index += 1;
                right_index += 1;
            }
            // The left unit is unmatched: it is gone.
            (None, _) if left_index < left.len() => {
                push(
                    DiffKind::Removed,
                    left.get(left_index).copied().unwrap_or(""),
                    &mut segments,
                );
                removed_words += 1;
                left_index += 1;
            }
            // The right unit is unmatched: it is new.
            (_, None) if right_index < right.len() => {
                push(
                    DiffKind::Added,
                    right.get(right_index).copied().unwrap_or(""),
                    &mut segments,
                );
                added_words += 1;
                right_index += 1;
            }
            _ => {
                // Remaining units on either side; consume them in order.
                if left_index < left.len() {
                    push(
                        DiffKind::Removed,
                        left.get(left_index).copied().unwrap_or(""),
                        &mut segments,
                    );
                    removed_words += 1;
                    left_index += 1;
                } else if right_index < right.len() {
                    push(
                        DiffKind::Added,
                        right.get(right_index).copied().unwrap_or(""),
                        &mut segments,
                    );
                    added_words += 1;
                    right_index += 1;
                } else {
                    break;
                }
            }
        }
    }

    let truncated = segments.len() > MAX_DIFF_SEGMENTS;
    segments.truncate(MAX_DIFF_SEGMENTS);
    TextDiff {
        segments,
        added_words,
        removed_words,
        truncated,
    }
}

/// A compact human-readable summary of a difference, for a status line.
pub fn describe_diff(diff: &TextDiff) -> String {
    format!("+{}/-{}", diff.added_words, diff.removed_words)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn correction(range: (usize, usize), original: &str, replacement: &str) -> Correction {
        Correction {
            range: TextRange::new(range.0, range.1),
            original: original.to_string(),
            replacement: replacement.to_string(),
            reason: None,
        }
    }

    #[test]
    fn a_correction_replaces_exactly_its_range() {
        let mut journal = CorrectionJournal::new();
        let text = "привт мир";
        let batch = apply_corrections(
            text,
            &[correction((0, 5), "привт", "привет")],
            Some(&text_version(text)),
            &mut journal,
        )
        .unwrap();
        assert_eq!(batch.after, "привет мир");
        assert_eq!(batch.applied_count(), 1);
        assert_eq!(batch.skipped_count(), 0);
        assert_eq!(batch.applied[0].range, TextRange::new(0, 6));
        assert_eq!(batch.applied[0].after, "привет");
        assert_eq!(batch.version_after, text_version("привет мир"));
        assert!(journal.can_undo());
        assert_eq!(journal.len(), 1);
    }

    #[test]
    fn a_stale_text_is_refused_before_anything_is_touched() {
        let mut journal = CorrectionJournal::new();
        let text = "привт мир";
        let error = apply_corrections(
            text,
            &[correction((0, 5), "привт", "привет")],
            Some("0000000000000000"),
            &mut journal,
        )
        .unwrap_err();
        assert_eq!(error.code(), "stale_text");
        assert!(journal.is_empty());
    }

    #[test]
    fn a_correction_that_no_longer_matches_is_skipped_not_applied() {
        let mut journal = CorrectionJournal::new();
        let text = "привт мир";
        let batch = apply_corrections(
            text,
            &[
                correction((0, 5), "другое", "привет"),
                correction((6, 9), "мир", "мир!"),
            ],
            None,
            &mut journal,
        )
        .unwrap();
        assert_eq!(batch.applied_count(), 1);
        assert_eq!(batch.after, "привт мир!");
        assert_eq!(batch.skipped_count(), 1);
        assert_eq!(batch.skipped[0].outcome, CorrectionOutcome::Mismatched);
        assert_eq!(batch.skipped[0].original, "другое");
    }

    #[test]
    fn overlapping_corrections_are_applied_one_at_a_time() {
        let mut journal = CorrectionJournal::new();
        let text = "привт мир";
        let batch = apply_corrections(
            text,
            &[
                correction((0, 5), "привт", "привет"),
                correction((3, 8), "вт ми", "???"),
            ],
            None,
            &mut journal,
        )
        .unwrap();
        assert_eq!(batch.applied_count(), 1);
        assert_eq!(batch.skipped_count(), 1);
        assert_eq!(batch.skipped[0].outcome, CorrectionOutcome::Overlapped);
    }

    #[test]
    fn a_range_past_the_end_of_the_text_is_skipped() {
        let mut journal = CorrectionJournal::new();
        let batch = apply_corrections(
            "короткий",
            &[correction((100, 105), "x", "y")],
            None,
            &mut journal,
        )
        .unwrap();
        assert_eq!(batch.applied_count(), 0);
        assert_eq!(batch.skipped[0].outcome, CorrectionOutcome::Mismatched);
        assert_eq!(batch.after, "короткий");
        // Nothing changed, so nothing can be undone.
        assert!(!journal.can_undo());
    }

    #[test]
    fn several_corrections_are_applied_in_text_order() {
        let mut journal = CorrectionJournal::new();
        let text = "привт ,  мир";
        // Character positions: `привт` 0..5, ` ` 5..6, `,` 6..7, `  ` 7..9, `мир` 9..12.
        let batch = apply_corrections(
            text,
            &[
                // Sent out of order on purpose.
                correction((7, 9), "  ", " "),
                correction((5, 6), " ", ""),
                correction((0, 5), "привт", "привет"),
            ],
            Some(&text_version(text)),
            &mut journal,
        )
        .unwrap();
        assert_eq!(batch.after, "привет, мир");
        assert_eq!(batch.applied_count(), 3);
        assert_eq!(batch.skipped_count(), 0);
        // The recorded ranges describe the text after the batch.
        let replaced = &batch.applied[0];
        assert_eq!(replaced.range.slice(&batch.after).unwrap(), "привет");
        assert_eq!(batch.version_after, text_version("привет, мир"));
    }

    #[test]
    fn emoji_and_combining_marks_do_not_shift_a_correction() {
        let mut journal = CorrectionJournal::new();
        let text = "😀 привт 👋";
        let batch = apply_corrections(
            text,
            &[correction((2, 7), "привт", "привет")],
            None,
            &mut journal,
        )
        .unwrap();
        assert_eq!(batch.after, "😀 привет 👋");
        assert_eq!(
            batch.applied[0].range.slice(&batch.after).unwrap(),
            "привет"
        );
    }

    #[test]
    fn undo_restores_the_exact_previous_text() {
        let mut journal = CorrectionJournal::new();
        let text = "привт ,  мир";
        let batch = apply_corrections(
            text,
            &[correction((0, 5), "привт", "привет")],
            Some(&text_version(text)),
            &mut journal,
        )
        .unwrap();
        let outcome = undo_last(&batch.after, &mut journal).unwrap();
        assert_eq!(outcome.text, text);
        assert_eq!(outcome.version_after, batch.version_before);
        assert_eq!(outcome.restored.len(), 1);
        assert_eq!(outcome.restored[0].before, "привт");
        assert_eq!(outcome.remaining, 0);
        assert!(!journal.can_undo());
    }

    #[test]
    fn undo_refuses_when_the_text_changed_afterwards() {
        let mut journal = CorrectionJournal::new();
        let text = "привт мир";
        let batch = apply_corrections(
            text,
            &[correction((0, 5), "привт", "привет")],
            None,
            &mut journal,
        )
        .unwrap();
        let edited = format!("{}!", batch.after);
        let error = undo_last(&edited, &mut journal).unwrap_err();
        assert_eq!(error.code(), "undo_conflict");
        // The correction is still undoable once the text matches again.
        assert!(undo_last(&batch.after, &mut journal).is_ok());
        assert_eq!(
            undo_last(&batch.after, &mut journal).unwrap_err(),
            AutocorrectError::NotFound
        );
    }

    #[test]
    fn the_journal_keeps_only_the_newest_batches() {
        let mut journal = CorrectionJournal::new();
        for index in 0..MAX_JOURNAL_BATCHES + 3 {
            // Every batch is applied to a distinct text, so each one is journalled.
            let text = format!("привт{index}");
            apply_corrections(
                &text,
                &[correction((0, 5), "привт", "привет")],
                None,
                &mut journal,
            )
            .unwrap();
        }
        assert_eq!(journal.len(), MAX_JOURNAL_BATCHES);
        assert_eq!(journal.dropped(), 3);
        // The oldest batch is gone, the newest is still undoable.
        assert!(journal.last().is_some());
    }

    #[test]
    fn an_empty_batch_is_not_journalled() {
        let mut journal = CorrectionJournal::new();
        let batch = apply_corrections("текст", &[], None, &mut journal).unwrap();
        assert!(batch.applied.is_empty());
        assert!(!journal.can_undo());
        // A correction that changes nothing is not recorded either.
        apply_corrections(
            "текст",
            &[correction((0, 5), "текст", "текст")],
            None,
            &mut journal,
        )
        .unwrap();
        assert!(!journal.can_undo());
        assert_eq!(journal.last_words(), Vec::<String>::new());
    }

    #[test]
    fn the_journal_reports_the_words_of_its_last_batch() {
        let mut journal = CorrectionJournal::new();
        apply_corrections(
            "привт мир",
            &[correction((0, 5), "привт", "привет")],
            None,
            &mut journal,
        )
        .unwrap();
        assert_eq!(journal.last_words(), vec!["привет".to_string()]);
        journal.clear();
        assert!(journal.is_empty());
    }

    #[test]
    fn a_difference_is_reported_word_by_word() {
        let diff = diff_texts("привет мир", "привет большой мир");
        assert!(diff.has_changes());
        assert_eq!(diff.added_words, 1);
        assert_eq!(diff.removed_words, 0);
        let added: Vec<&str> = diff
            .segments
            .iter()
            .filter(|segment| segment.kind == DiffKind::Added)
            .map(|segment| segment.text.trim())
            .collect();
        assert_eq!(added, vec!["большой"]);

        let removed = diff_texts("привет большой мир", "привет мир");
        assert_eq!(removed.removed_words, 1);
        assert_eq!(removed.added_words, 0);

        let same = diff_texts("привет мир", "привет мир");
        assert!(!same.has_changes());
        assert_eq!(same.segments.len(), 1);
        assert_eq!(same.segments[0].kind, DiffKind::Same);
    }

    #[test]
    fn a_preview_can_be_rebuilt_from_its_segments() {
        let before = "привет мир\nвторой ряд";
        let after = "привет большой мир\nвторой ряд";
        let diff = diff_texts(before, after);
        let rebuilt: String = diff
            .segments
            .iter()
            .filter(|segment| segment.kind != DiffKind::Removed)
            .map(|segment| segment.text.as_str())
            .collect();
        assert_eq!(rebuilt, after);
        let rebuilt_before: String = diff
            .segments
            .iter()
            .filter(|segment| segment.kind != DiffKind::Added)
            .map(|segment| segment.text.as_str())
            .collect();
        assert_eq!(rebuilt_before, before);
        assert_eq!(describe_diff(&diff), "+1/-0");
    }

    #[test]
    fn an_empty_difference_is_harmless() {
        let diff = diff_texts("", "");
        assert!(!diff.has_changes());
        assert!(diff.segments.is_empty());
        let diff = diff_texts("", "новый текст");
        assert_eq!(diff.added_words, 2);
        let diff = diff_texts("был текст", "");
        assert_eq!(diff.removed_words, 2);
    }

    #[test]
    fn comparison_units_rebuild_both_texts_exactly() {
        for text in [
            "",
            " ",
            "привет",
            "  привет   мир  ",
            "первый\nвторой\tтретий",
            "😀 привт 👋 мир",
            "e\u{0301} текст",
        ] {
            let rebuilt: String = units(text).concat();
            assert_eq!(rebuilt, text, "units must rebuild {text:?}");
        }
    }
}
