//! Command-based undo/redo history.
//!
//! Legacy gInk keeps eight whole-document snapshots in a ring buffer
//! (`src/Root.cs`: `UndoStrokes[8]`, `UndoDepth` capped at 7). The rewrite
//! records invertible edit commands instead: the document stores each stroke
//! once, and a history entry only borrows the effect of an edit by cloning the
//! strokes it added or removed. Nothing is cloned per frame or per pointer
//! sample.
//!
//! Semantics preserved from the legacy baseline (§5.4):
//! - one current state plus at most [`MAX_UNDO_STATES`] (7) undo states;
//! - a new edit discards the redo tail;
//! - erase is an explicit history command (legacy D-004 folded erases and must
//!   not be copied);
//! - closing the annotation is an edit, so undo after reopening restores the
//!   pre-close strokes (same process, no disk persistence).

use std::collections::VecDeque;

use crate::ink::model::{Stroke, StrokeId};

/// Maximum number of undo states (legacy `UndoStrokes.Length - 1`).
pub const MAX_UNDO_STATES: usize = 7;

/// A stroke inserted by an edit, with the index used to restore its z-order.
#[derive(Debug)]
pub(crate) struct AddedStroke {
    pub index: usize,
    pub id: StrokeId,
    pub stroke: Stroke,
}

/// A stroke removed by an edit, with its original z-order index.
#[derive(Debug)]
pub(crate) struct RemovedStroke {
    pub index: usize,
    pub id: StrokeId,
    pub stroke: Stroke,
}

/// One invertible document edit: `added` and `removed` are the forward effects.
/// Undo removes `added` and re-inserts `removed`; redo does the opposite.
#[derive(Debug)]
pub(crate) struct HistoryEntry {
    pub added: Option<AddedStroke>,
    pub removed: Vec<RemovedStroke>,
}

/// Bounded undo/redo stack with a cursor (`position` = number of applied
/// entries in the retained window).
#[derive(Debug, Default)]
pub(crate) struct History {
    entries: VecDeque<HistoryEntry>,
    position: usize,
}

impl History {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a new applied edit, discarding any redo tail and enforcing the
    /// seven-state window.
    pub fn record(&mut self, entry: HistoryEntry) {
        self.entries.truncate(self.position);
        self.entries.push_back(entry);
        self.position += 1;
        while self.entries.len() > MAX_UNDO_STATES {
            self.entries.pop_front();
            self.position -= 1;
        }
    }

    /// Discards the redo tail without recording an edit. Used when closing an
    /// already-empty annotation, which still invalidates redo in the legacy
    /// exit path (`SaveUndoStrokes` clears `RedoDepth`).
    pub fn discard_redo(&mut self) {
        self.entries.truncate(self.position);
    }

    /// The entry an undo would invert.
    pub fn undo_entry(&self) -> Option<&HistoryEntry> {
        self.position
            .checked_sub(1)
            .and_then(|index| self.entries.get(index))
    }

    /// The entry a redo would replay.
    pub fn redo_entry(&self) -> Option<&HistoryEntry> {
        self.entries.get(self.position)
    }

    pub fn finish_undo(&mut self) {
        debug_assert!(self.position > 0);
        self.position -= 1;
    }

    pub fn finish_redo(&mut self) {
        debug_assert!(self.position < self.entries.len());
        self.position += 1;
    }

    pub fn undo_depth(&self) -> usize {
        self.position
    }

    pub fn redo_depth(&self) -> usize {
        self.entries.len() - self.position
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_entry() -> HistoryEntry {
        HistoryEntry {
            added: None,
            removed: Vec::new(),
        }
    }

    #[test]
    fn window_keeps_seven_entries_and_redo_tail_is_truncated() {
        let mut history = History::new();
        for _ in 0..8 {
            history.record(empty_entry());
        }
        assert_eq!(history.undo_depth(), MAX_UNDO_STATES);
        assert_eq!(history.redo_depth(), 0);

        history.finish_undo();
        history.finish_undo();
        assert_eq!(history.undo_depth(), 5);
        assert_eq!(history.redo_depth(), 2);

        history.record(empty_entry());
        assert_eq!(history.undo_depth(), 6);
        assert_eq!(history.redo_depth(), 0);
        assert!(history.redo_entry().is_none());
    }
}
