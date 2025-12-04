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

/// Result of a state transition
#[derive(Debug, Clone, uniffi::Record)]
pub struct StateTransition {
    /// The new state after the transition
    pub new_state: WordCheckState,
    /// Whether the UI should advance to the next word
    pub should_advance_word: bool,
    /// Suggested animation/delay duration in ms (None if no animation needed)
    pub animation_duration_ms: Option<u32>,
}

/// Internal mutable state
#[derive(Debug)]
struct Inner {
    validator: Arc<WordValidator>,
    state: WordCheckState,
    word_number: u8,
    is_correct: Option<bool>,
    config: WordVerifyAnimationConfig,
}

/// State machine for word verification flow
///
/// UI sends events (select_word, animation_complete, etc.) and receives
/// StateTransition results that tell it what state to render and what
/// animations to play.
#[derive(Debug, Clone, uniffi::Object)]
pub struct WordVerifyStateMachine(Arc<RwLock<Inner>>);

#[uniffi::export]
impl WordVerifyStateMachine {
    /// Create a new state machine with the given validator
    #[uniffi::constructor]
    pub fn new(validator: Arc<WordValidator>, starting_word_number: u8) -> Self {
        Self(Arc::new(RwLock::new(Inner {
            validator,
            state: WordCheckState::None,
            word_number: starting_word_number,
            is_correct: None,
            config: WordVerifyAnimationConfig::default(),
        })))
    }

    /// Get the current state
    pub fn state(&self) -> WordCheckState {
        self.0.read().state.clone()
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

    /// User tapped a word - start the checking animation
    ///
    /// Returns a transition with Checking state and the animation duration.
    /// If already animating, returns no-change.
    pub fn select_word(&self, word: String) -> StateTransition {
        let mut inner = self.0.write();

        if !matches!(inner.state, WordCheckState::None) {
            // already animating, ignore
            return StateTransition {
                new_state: inner.state.clone(),
                should_advance_word: false,
                animation_duration_ms: None,
            };
        }

        let is_correct = inner.validator.is_word_correct(word.clone(), inner.word_number);
        inner.is_correct = Some(is_correct);
        inner.state = WordCheckState::Checking { word };

        let duration = if is_correct {
            inner.config.move_duration_ms_correct
        } else {
            inner.config.move_duration_ms_incorrect
        };

        StateTransition {
            new_state: inner.state.clone(),
            should_advance_word: false,
            animation_duration_ms: Some(duration),
        }
    }

    /// Animation to target complete - transition to Correct or Incorrect
    ///
    /// Returns a transition with the result state and dwell duration.
    pub fn animation_complete(&self) -> StateTransition {
        let mut inner = self.0.write();

        let word = match &inner.state {
            WordCheckState::Checking { word } => word.clone(),
            _ => {
                return StateTransition {
                    new_state: inner.state.clone(),
                    should_advance_word: false,
                    animation_duration_ms: None,
                };
            }
        };

        let is_correct = inner.is_correct.unwrap_or(false);

        inner.state = if is_correct {
            WordCheckState::Correct { word }
        } else {
            WordCheckState::Incorrect { word }
        };

        let dwell = if is_correct {
            inner.config.dwell_duration_ms_correct
        } else {
            inner.config.dwell_duration_ms_incorrect
        };

        StateTransition {
            new_state: inner.state.clone(),
            should_advance_word: false,
            animation_duration_ms: Some(dwell),
        }
    }

    /// Dwell time complete - advance word or start return animation
    ///
    /// If correct: transitions to None and signals to advance word.
    /// If incorrect: transitions to Returning state.
    pub fn dwell_complete(&self) -> StateTransition {
        let mut inner = self.0.write();

        match &inner.state {
            WordCheckState::Correct { .. } => {
                inner.state = WordCheckState::None;
                inner.is_correct = None;
                inner.word_number += 1;

                StateTransition {
                    new_state: inner.state.clone(),
                    should_advance_word: true,
                    animation_duration_ms: None,
                }
            }
            WordCheckState::Incorrect { word } => {
                inner.state = WordCheckState::Returning { word: word.clone() };

                StateTransition {
                    new_state: inner.state.clone(),
                    should_advance_word: false,
                    animation_duration_ms: Some(inner.config.move_duration_ms_incorrect),
                }
            }
            _ => StateTransition {
                new_state: inner.state.clone(),
                should_advance_word: false,
                animation_duration_ms: None,
            },
        }
    }

    /// Return animation complete (after incorrect) - back to None
    pub fn return_complete(&self) -> StateTransition {
        let mut inner = self.0.write();
        inner.state = WordCheckState::None;
        inner.is_correct = None;

        StateTransition {
            new_state: inner.state.clone(),
            should_advance_word: false,
            animation_duration_ms: None,
        }
    }

    /// Check if all words have been verified
    pub fn is_complete(&self) -> bool {
        let inner = self.0.read();
        inner.validator.is_complete(inner.word_number)
    }

    /// Reset to a specific word number (useful for going back)
    pub fn reset_to_word(&self, word_number: u8) {
        let mut inner = self.0.write();
        inner.state = WordCheckState::None;
        inner.is_correct = None;
        inner.word_number = word_number;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_validator() -> Arc<WordValidator> {
        Arc::new(WordValidator::preview(true, None))
    }

    #[test]
    fn test_initial_state() {
        let validator = create_test_validator();
        let sm = WordVerifyStateMachine::new(validator, 1);

        assert_eq!(sm.state(), WordCheckState::None);
        assert_eq!(sm.word_number(), 1);
        assert!(!sm.is_complete());
    }

    #[test]
    fn test_select_correct_word() {
        let validator = create_test_validator();
        let sm = WordVerifyStateMachine::new(validator.clone(), 1);

        // get the correct word
        let possible = sm.possible_words();
        let correct_word =
            possible.iter().find(|w| validator.is_word_correct(w.to_string(), 1)).unwrap().clone();

        let transition = sm.select_word(correct_word.clone());

        assert!(matches!(transition.new_state, WordCheckState::Checking { .. }));
        assert!(!transition.should_advance_word);
        assert!(transition.animation_duration_ms.is_some());
    }

    #[test]
    fn test_correct_word_flow() {
        let validator = create_test_validator();
        let sm = WordVerifyStateMachine::new(validator.clone(), 1);

        // get the correct word
        let possible = sm.possible_words();
        let correct_word =
            possible.iter().find(|w| validator.is_word_correct(w.to_string(), 1)).unwrap().clone();

        // select word
        sm.select_word(correct_word);
        assert!(matches!(sm.state(), WordCheckState::Checking { .. }));

        // animation complete
        let transition = sm.animation_complete();
        assert!(matches!(transition.new_state, WordCheckState::Correct { .. }));

        // dwell complete
        let transition = sm.dwell_complete();
        assert!(matches!(transition.new_state, WordCheckState::None));
        assert!(transition.should_advance_word);
        assert_eq!(sm.word_number(), 2);
    }

    #[test]
    fn test_incorrect_word_flow() {
        let validator = create_test_validator();
        let sm = WordVerifyStateMachine::new(validator.clone(), 1);

        // select an incorrect word
        sm.select_word("wrongword".to_string());
        assert!(matches!(sm.state(), WordCheckState::Checking { .. }));

        // animation complete
        let transition = sm.animation_complete();
        assert!(matches!(transition.new_state, WordCheckState::Incorrect { .. }));

        // dwell complete - should transition to returning
        let transition = sm.dwell_complete();
        assert!(matches!(transition.new_state, WordCheckState::Returning { .. }));
        assert!(!transition.should_advance_word);

        // return complete
        let transition = sm.return_complete();
        assert!(matches!(transition.new_state, WordCheckState::None));
        assert_eq!(sm.word_number(), 1); // should not advance
    }

    #[test]
    fn test_ignore_select_while_animating() {
        let validator = create_test_validator();
        let sm = WordVerifyStateMachine::new(validator, 1);

        sm.select_word("word1".to_string());
        let original_state = sm.state();

        // try to select another word while animating
        let transition = sm.select_word("word2".to_string());

        // should return current state, no change
        assert_eq!(transition.new_state, original_state);
    }
}
