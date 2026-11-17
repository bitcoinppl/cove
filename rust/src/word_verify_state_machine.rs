//! State machine for word verification flow
//!
//! Provides a shared state machine that both iOS and Android use for
//! the recovery word verification animation flow.

use std::sync::Arc;

use parking_lot::RwLock;

use crate::word_validator::WordValidator;

/// The current state of the word verification check
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Enum)]
pub enum WordCheckState {
    /// No word is being checked
    None,
    /// User tapped a word, animating to target
    Checking { word: String },
    /// Word was correct, showing green
    Correct { word: String },
    /// Word was incorrect, showing red
    Incorrect { word: String },
    /// Returning to origin after incorrect
    Returning { word: String },
}

/// Animation timing configuration
#[derive(Debug, Clone, uniffi::Record)]
pub struct WordVerifyAnimationConfig {
    /// Duration (ms) for chip to travel to target when correct
    pub move_duration_ms_correct: u32,
    /// Duration (ms) for chip to travel to target when incorrect
    pub move_duration_ms_incorrect: u32,
    /// How long (ms) chip stays at target after arriving (correct)
    pub dwell_duration_ms_correct: u32,
    /// How long (ms) chip stays at target after arriving (incorrect)
    pub dwell_duration_ms_incorrect: u32,
}

impl Default for WordVerifyAnimationConfig {
    fn default() -> Self {
        Self {
            move_duration_ms_correct: 300,
            move_duration_ms_incorrect: 400,
            dwell_duration_ms_correct: 1000,
            dwell_duration_ms_incorrect: 1000,
        }
    }
}

/// Identifies one pick of a word, from the tap until its chip is back at rest
///
/// Animation callbacks carry the id of the pick that started them, so a callback that outlives
/// its pick, such as the flight of a cancelled chip finishing after a new pick began, is
/// recognised as stale and ignored instead of acting on the newer pick
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, derive_more::From, derive_more::Into)]
pub struct WordCheckId(u64);

uniffi::custom_newtype!(WordCheckId, u64);

/// Result of a state transition
#[derive(Debug, Clone, uniffi::Record)]
pub struct StateTransition {
    /// The pick this transition belongs to, passed back with the next event for it
    pub check_id: WordCheckId,
    /// The new state after the transition
    pub new_state: WordCheckState,
    /// Whether the UI should advance to the next word
    pub should_advance_word: bool,
    /// Suggested animation/delay duration in ms (None if no animation needed)
    pub animation_duration_ms: Option<u32>,
}

/// Where the picked chip is in its round trip
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CheckPhase {
    /// Flying to the answer slot
    Flying,
    /// Showing green or red in the slot
    Dwelling,
    /// Flying back after a wrong pick or a cancel
    Returning,
}

/// The pick being checked, which exists from the tap until its chip is back at rest
#[derive(Debug)]
struct ActiveCheck {
    id: WordCheckId,
    word: String,
    is_correct: bool,
    phase: CheckPhase,
}

impl ActiveCheck {
    fn state(&self) -> WordCheckState {
        let word = self.word.clone();

        match self.phase {
            CheckPhase::Flying => WordCheckState::Checking { word },
            CheckPhase::Dwelling if self.is_correct => WordCheckState::Correct { word },
            CheckPhase::Dwelling => WordCheckState::Incorrect { word },
            CheckPhase::Returning => WordCheckState::Returning { word },
        }
    }
}

/// Internal mutable state
#[derive(Debug)]
struct Inner {
    validator: Arc<WordValidator>,
    check: Option<ActiveCheck>,
    next_check_id: u64,
    word_number: u8,
    config: WordVerifyAnimationConfig,
}

impl Inner {
    fn state(&self) -> WordCheckState {
        self.check.as_ref().map_or(WordCheckState::None, ActiveCheck::state)
    }

    /// The active check when it is `id` and in `phase`, or None for an event that is stale or
    /// out of order
    fn check_in(&mut self, id: WordCheckId, phase: CheckPhase) -> Option<&mut ActiveCheck> {
        self.check.as_mut().filter(|check| check.id == id && check.phase == phase)
    }

    fn transition(
        &self,
        id: WordCheckId,
        should_advance_word: bool,
        animation_duration_ms: Option<u32>,
    ) -> StateTransition {
        StateTransition {
            check_id: id,
            new_state: self.state(),
            should_advance_word,
            animation_duration_ms,
        }
    }
}

/// State machine for word verification flow
///
/// The UI sends events and draws the state each returned transition carries. A pick starts a
/// check with its own id; the events that move it along (`animation_complete`,
/// `dwell_complete`, `cancel_check`, `return_complete`) take that id and return None when they
/// are stale or out of order, so the UI drops them instead of drawing a state the machine is not
/// in. One check runs at a time, and a pick made while one runs returns None too
#[derive(Debug, Clone, uniffi::Object)]
pub struct WordVerifyStateMachine(Arc<RwLock<Inner>>);

#[uniffi::export]
impl WordVerifyStateMachine {
    /// Create a new state machine with the given validator
    #[uniffi::constructor]
    pub fn new(validator: Arc<WordValidator>, starting_word_number: u8) -> Self {
        Self(Arc::new(RwLock::new(Inner {
            validator,
            check: None,
            next_check_id: 0,
            word_number: starting_word_number,
            config: WordVerifyAnimationConfig::default(),
        })))
    }

    /// Get the current state
    pub fn state(&self) -> WordCheckState {
        self.0.read().state()
    }

    /// Get the current word number being verified (1-indexed)
    pub fn word_number(&self) -> u8 {
        self.0.read().word_number
    }

    /// Get possible words for the current word number
    pub fn possible_words(&self) -> Vec<String> {
        let inner = self.0.read();
        inner.validator.possible_words(inner.word_number)
    }

    /// Get the animation configuration
    pub fn config(&self) -> WordVerifyAnimationConfig {
        self.0.read().config.clone()
    }

    /// User tapped a word: start a check, with the chip flying to the answer slot
    ///
    /// Returns None while another check runs or once every word is verified, and the tap does
    /// nothing
    pub fn select_word(&self, word: String) -> Option<StateTransition> {
        let mut inner = self.0.write();
        if inner.check.is_some() || inner.validator.is_complete(inner.word_number) {
            return None;
        }

        let id = WordCheckId(inner.next_check_id);
        inner.next_check_id += 1;

        let is_correct = inner.validator.is_word_correct(word.clone(), inner.word_number);
        inner.check = Some(ActiveCheck { id, word, is_correct, phase: CheckPhase::Flying });

        let duration = if is_correct {
            inner.config.move_duration_ms_correct
        } else {
            inner.config.move_duration_ms_incorrect
        };

        Some(inner.transition(id, false, Some(duration)))
    }

    /// The chip reached the answer slot: show it as correct or incorrect
    ///
    /// Returns the transition with the dwell duration, or None unless check `id` is flying
    pub fn animation_complete(&self, id: WordCheckId) -> Option<StateTransition> {
        let mut inner = self.0.write();
        let is_correct = {
            let check = inner.check_in(id, CheckPhase::Flying)?;
            check.phase = CheckPhase::Dwelling;
            check.is_correct
        };

        let dwell = if is_correct {
            inner.config.dwell_duration_ms_correct
        } else {
            inner.config.dwell_duration_ms_incorrect
        };

        Some(inner.transition(id, false, Some(dwell)))
    }

    /// User tapped the chip while it flies to the slot: send it back without checking it
    ///
    /// Returns a Returning transition, or None unless a check is flying
    pub fn cancel_check(&self) -> Option<StateTransition> {
        let mut inner = self.0.write();
        let id = {
            let check = inner.check.as_mut().filter(|check| check.phase == CheckPhase::Flying)?;
            check.phase = CheckPhase::Returning;
            check.id
        };

        Some(inner.transition(id, false, Some(inner.config.move_duration_ms_incorrect)))
    }

    /// Dwell time complete: advance on a correct word or send a wrong one back
    ///
    /// A correct check ends here, moving to the next word with `should_advance_word` set; a
    /// wrong one starts Returning. Returns None unless check `id` is dwelling
    pub fn dwell_complete(&self, id: WordCheckId) -> Option<StateTransition> {
        let mut inner = self.0.write();
        let check = inner.check_in(id, CheckPhase::Dwelling)?;

        if check.is_correct {
            inner.check = None;
            inner.word_number += 1;

            return Some(inner.transition(id, true, None));
        }

        check.phase = CheckPhase::Returning;
        Some(inner.transition(id, false, Some(inner.config.move_duration_ms_incorrect)))
    }

    /// Return animation complete: the chip is back in its place and another word can be picked
    ///
    /// Returns None unless check `id` is returning
    pub fn return_complete(&self, id: WordCheckId) -> Option<StateTransition> {
        let mut inner = self.0.write();
        inner.check_in(id, CheckPhase::Returning)?;
        inner.check = None;

        Some(inner.transition(id, false, None))
    }

    /// Check if all words have been verified
    pub fn is_complete(&self) -> bool {
        let inner = self.0.read();
        inner.validator.is_complete(inner.word_number)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A machine on word 1 of a 12-word preview wallet, with that word's correct and a wrong pick
    fn machine() -> (WordVerifyStateMachine, Arc<WordValidator>) {
        let validator = Arc::new(WordValidator::preview(true, None));
        (WordVerifyStateMachine::new(validator.clone(), 1), validator)
    }

    fn correct_word(sm: &WordVerifyStateMachine, validator: &WordValidator) -> String {
        let word_number = sm.word_number();
        sm.possible_words()
            .into_iter()
            .find(|word| validator.is_word_correct(word.clone(), word_number))
            .expect("the correct word is offered")
    }

    fn wrong_word(sm: &WordVerifyStateMachine, validator: &WordValidator) -> String {
        let word_number = sm.word_number();
        sm.possible_words()
            .into_iter()
            .find(|word| !validator.is_word_correct(word.clone(), word_number))
            .expect("a wrong word is offered")
    }

    /// Runs a correct pick for the current word through to the advance
    fn verify_current_word(sm: &WordVerifyStateMachine, validator: &WordValidator) {
        let id = sm.select_word(correct_word(sm, validator)).unwrap().check_id;
        sm.animation_complete(id).unwrap();
        assert!(sm.dwell_complete(id).unwrap().should_advance_word);
    }

    #[test]
    fn test_initial_state() {
        let (sm, _) = machine();

        assert_eq!(sm.state(), WordCheckState::None);
        assert!(!sm.is_complete());
    }

    #[test]
    fn test_correct_word_flow() {
        let (sm, validator) = machine();
        let word = correct_word(&sm, &validator);

        let selected = sm.select_word(word.clone()).unwrap();
        assert_eq!(selected.new_state, WordCheckState::Checking { word: word.clone() });
        assert!(!selected.should_advance_word);
        assert!(selected.animation_duration_ms.is_some());

        let id = selected.check_id;
        let landed = sm.animation_complete(id).unwrap();
        assert_eq!(landed.new_state, WordCheckState::Correct { word });

        let dwelled = sm.dwell_complete(id).unwrap();
        assert_eq!(dwelled.new_state, WordCheckState::None);
        assert!(dwelled.should_advance_word);
        assert_eq!(sm.word_number(), 2);
    }

    #[test]
    fn test_incorrect_word_flow() {
        let (sm, validator) = machine();
        let word = wrong_word(&sm, &validator);

        let id = sm.select_word(word.clone()).unwrap().check_id;
        let landed = sm.animation_complete(id).unwrap();
        assert_eq!(landed.new_state, WordCheckState::Incorrect { word: word.clone() });

        let dwelled = sm.dwell_complete(id).unwrap();
        assert_eq!(dwelled.new_state, WordCheckState::Returning { word });
        assert!(!dwelled.should_advance_word);

        let returned = sm.return_complete(id).unwrap();
        assert_eq!(returned.new_state, WordCheckState::None);
        assert_eq!(sm.word_number(), 1);
    }

    #[test]
    fn test_ignore_select_while_checking() {
        let (sm, validator) = machine();
        let wrong = wrong_word(&sm, &validator);
        sm.select_word(wrong.clone()).unwrap();

        assert!(sm.select_word(correct_word(&sm, &validator)).is_none());
        assert_eq!(sm.state(), WordCheckState::Checking { word: wrong });
    }

    #[test]
    fn test_cancel_check_returns_the_chip_without_checking_it() {
        let (sm, validator) = machine();
        let word = correct_word(&sm, &validator);
        let id = sm.select_word(word.clone()).unwrap().check_id;

        let cancelled = sm.cancel_check().unwrap();
        assert_eq!(cancelled.check_id, id);
        assert_eq!(cancelled.new_state, WordCheckState::Returning { word });
        assert!(!cancelled.should_advance_word);
        assert!(cancelled.animation_duration_ms.is_some());

        // the flight the cancel interrupted still reports landing, and must not check the word
        assert!(sm.animation_complete(id).is_none());

        assert_eq!(sm.return_complete(id).unwrap().new_state, WordCheckState::None);
        assert_eq!(sm.word_number(), 1);
    }

    #[test]
    fn test_cancel_check_only_while_flying() {
        let (sm, validator) = machine();
        assert!(sm.cancel_check().is_none());

        let id = sm.select_word(wrong_word(&sm, &validator)).unwrap().check_id;
        sm.animation_complete(id).unwrap();
        assert!(sm.cancel_check().is_none());

        sm.dwell_complete(id).unwrap();
        assert!(sm.cancel_check().is_none());
    }

    /// The iOS repro: a wrong pick cancelled mid-flight, then the correct word picked before the
    /// cancelled flight's completion fires; that completion must not check the new pick
    #[test]
    fn test_stale_flight_completion_cannot_check_a_later_pick() {
        let (sm, validator) = machine();
        let wrong = sm.select_word(wrong_word(&sm, &validator)).unwrap().check_id;
        sm.cancel_check().unwrap();
        sm.return_complete(wrong).unwrap();

        let word = correct_word(&sm, &validator);
        let picked = sm.select_word(word.clone()).unwrap().check_id;
        assert_ne!(picked, wrong);

        assert!(sm.animation_complete(wrong).is_none());
        assert_eq!(sm.state(), WordCheckState::Checking { word: word.clone() });

        assert_eq!(
            sm.animation_complete(picked).unwrap().new_state,
            WordCheckState::Correct { word }
        );
        assert!(sm.dwell_complete(picked).unwrap().should_advance_word);
        assert_eq!(sm.word_number(), 2);
    }

    /// A late dwell or return completion from an earlier wrong pick must not advance or reset a
    /// later correct pick, which on the last word moved the machine past it
    #[test]
    fn test_stale_dwell_and_return_completions_are_ignored() {
        let (sm, validator) = machine();
        for _ in 1..12 {
            verify_current_word(&sm, &validator);
        }

        assert_eq!(sm.word_number(), 12);

        let wrong = sm.select_word(wrong_word(&sm, &validator)).unwrap().check_id;
        sm.animation_complete(wrong).unwrap();
        sm.dwell_complete(wrong).unwrap();
        sm.return_complete(wrong).unwrap();

        let picked = sm.select_word(correct_word(&sm, &validator)).unwrap().check_id;
        assert!(sm.return_complete(wrong).is_none());
        sm.animation_complete(picked).unwrap();

        assert!(sm.dwell_complete(wrong).is_none());
        assert!(sm.return_complete(wrong).is_none());
        assert_eq!(sm.word_number(), 12);
        assert!(matches!(sm.state(), WordCheckState::Correct { .. }));

        assert!(sm.dwell_complete(picked).unwrap().should_advance_word);
        assert!(sm.is_complete());
    }

    #[test]
    fn test_events_out_of_order_are_ignored() {
        let (sm, validator) = machine();
        let id = sm.select_word(wrong_word(&sm, &validator)).unwrap().check_id;

        assert!(sm.dwell_complete(id).is_none());
        assert!(sm.return_complete(id).is_none());

        sm.animation_complete(id).unwrap();
        assert!(sm.animation_complete(id).is_none());
        assert!(sm.return_complete(id).is_none());
    }

    #[test]
    fn test_no_pick_once_every_word_is_verified() {
        let (sm, validator) = machine();
        for _ in 0..12 {
            verify_current_word(&sm, &validator);
        }

        assert_eq!(sm.word_number(), 13);
        assert!(sm.is_complete());
        assert!(sm.select_word("abandon".to_string()).is_none());
        assert_eq!(sm.state(), WordCheckState::None);
    }
}
