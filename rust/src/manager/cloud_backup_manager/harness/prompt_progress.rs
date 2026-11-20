//! Invariant 11: answering a prompt makes progress
//!
//! A prompt choice that goes on, rather than leaving through Discard or a dismissal, must not end
//! on the screen it was chosen on with nothing new: no notice issued on its control, no changed
//! state and no new choice. That screen asks the user for the same answer again, which is a loop

use super::invariants::{Invariant, Violation};
use crate::manager::cloud_backup_manager::CloudBackupRootPrompt;
use crate::manager::cloud_backup_manager::admission::UserIntent;
use crate::manager::cloud_backup_manager::screen::{
    CloudBackupOutcomeTarget, CloudBackupScreen, CloudBackupScreenAction, CloudBackupScreenDelta,
};

/// The prompt answer waiting for the work it started to end
#[derive(Debug, Default)]
pub(crate) struct PromptProgress(Option<Answer>);

#[derive(Debug)]
struct Answer {
    intent: UserIntent,
    /// The control the answer was sent from, where a notice for it is drawn
    control: CloudBackupOutcomeTarget,
    /// The screen the platform showed when the user answered, without its notice
    shown: CloudBackupScreen,
    /// Whether a notice was issued on `control` after the answer, even one that reads like the
    /// notice before
    noticed: bool,
}

impl PromptProgress {
    /// The test dispatched `intent` while the platform showed `shown`
    ///
    /// Any later dispatch replaces the answer waiting, since the user moved on before it ended
    pub(crate) fn dispatched(&mut self, intent: &UserIntent, shown: Option<&CloudBackupScreen>) {
        self.0 = None;

        // Discard and the prompts' dismissals leave the prompt, so only Use Existing, Try Again
        // and Create New are answers that go on
        let UserIntent::AcceptEnablePrompt(choice) = intent else { return };
        let Some(shown) = shown.filter(|shown| shown.root_prompt != CloudBackupRootPrompt::None)
        else {
            return;
        };

        self.0 = Some(Answer {
            intent: intent.clone(),
            control: CloudBackupOutcomeTarget::Action(
                CloudBackupScreenAction::accepting_enable_prompt(*choice),
            ),
            shown: without_notice(shown),
            noticed: false,
        });
    }

    /// 11: once nothing the answer started is live, the published screen differs from the one it
    /// was answered on, or a notice was issued on the answer's control
    ///
    /// Notices are left out of the comparison: one cleared, or issued on another control, doesn't
    /// tell the user why the same prompt is back
    ///
    /// `sent` holds the deltas sent since the last check; with work still live the answer waits
    pub(crate) fn check(
        &mut self,
        sent: &[CloudBackupScreenDelta],
        published: &CloudBackupScreen,
        live: bool,
    ) -> Result<(), Violation> {
        let Some(answer) = self.0.as_mut() else { return Ok(()) };
        answer.noticed |= sent.iter().any(|delta| issues_notice_on(delta, &answer.control));
        if live {
            return Ok(());
        }

        let Some(answer) = self.0.take() else { return Ok(()) };
        if answer.noticed || without_notice(published) != answer.shown {
            return Ok(());
        }

        Err(Violation::new(
            Invariant::PromptProgress,
            format!(
                "{:?} brought back {:?} unchanged, with no notice issued on its control",
                answer.intent, published.root_prompt
            ),
        ))
    }
}

fn issues_notice_on(delta: &CloudBackupScreenDelta, control: &CloudBackupOutcomeTarget) -> bool {
    delta
        .last_outcome
        .as_ref()
        .and_then(|update| update.outcome.as_ref())
        .is_some_and(|notice| notice.target == *control)
}

fn without_notice(screen: &CloudBackupScreen) -> CloudBackupScreen {
    CloudBackupScreen { last_outcome: None, ..screen.clone() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::cloud_backup_manager::screen::{
        CloudBackupLastOutcome, CloudBackupLastOutcomeUpdate, CloudBackupOutcome,
        CloudBackupOutcomeTarget, CloudBackupScreenAction, cloud_backup_screen_fixtures,
    };
    use crate::manager::cloud_backup_manager::{
        CloudBackupEnableContext, CloudBackupEnablePromptChoice,
    };

    fn existing_backup_prompt() -> CloudBackupScreen {
        let mut screen = cloud_backup_screen_fixtures()
            .into_iter()
            .find(|fixture| fixture.name.ends_with(":disabled"))
            .expect("the disabled fixture")
            .screen;
        screen.root_prompt = CloudBackupRootPrompt::ExistingBackupFound(
            CloudBackupEnableContext::settings_manual(),
            None,
        );
        screen
    }

    fn use_existing() -> UserIntent {
        UserIntent::AcceptEnablePrompt(CloudBackupEnablePromptChoice::UseExisting)
    }

    /// A notice on `action`'s control
    fn notice_on(action: CloudBackupScreenAction) -> CloudBackupScreenDelta {
        CloudBackupScreenDelta {
            revision: 1,
            body: None,
            settings_row: None,
            heal_notice: None,
            last_outcome: Some(CloudBackupLastOutcomeUpdate {
                outcome: Some(CloudBackupLastOutcome {
                    target: CloudBackupOutcomeTarget::Action(action),
                    outcome: CloudBackupOutcome::Failed { message: "opened nothing".into() },
                }),
            }),
            root_prompt: None,
            actions: None,
        }
    }

    /// Use Existing that ends on the same prompt with nothing new breaks 11, even with a notice
    /// on another control, and the same answer passes once a notice on Use Existing was issued or
    /// the screen changed
    #[test]
    fn an_answer_that_brings_back_the_same_prompt_breaks_11() {
        let shown = existing_backup_prompt();
        let mut progress = PromptProgress::default();

        progress.dispatched(&use_existing(), Some(&shown));
        assert!(progress.check(&[], &shown, true).is_ok(), "live work may still end it");
        let violation = progress.check(&[], &shown, false).expect_err("the same prompt again");
        assert_eq!(violation.invariant, Invariant::PromptProgress, "{}", violation.detail);

        let elsewhere = notice_on(CloudBackupScreenAction::AcceptEnablePromptCreateNew);
        let mut with_elsewhere = shown.clone();
        with_elsewhere.last_outcome =
            elsewhere.last_outcome.clone().and_then(|update| update.outcome);
        progress.dispatched(&use_existing(), Some(&shown));
        let violation = progress
            .check(&[elsewhere], &with_elsewhere, false)
            .expect_err("a notice on Create New says nothing about Use Existing");
        assert_eq!(violation.invariant, Invariant::PromptProgress, "{}", violation.detail);

        let failed = notice_on(CloudBackupScreenAction::AcceptEnablePromptUseExisting);
        progress.dispatched(&use_existing(), Some(&shown));
        assert!(progress.check(&[failed], &shown, false).is_ok(), "a notice was issued");

        let mut moved = shown.clone();
        moved.root_prompt = CloudBackupRootPrompt::None;
        progress.dispatched(&use_existing(), Some(&shown));
        assert!(progress.check(&[], &moved, false).is_ok(), "the prompt closed");
    }
}
