#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloudBackupOperationKind {
    Enable,
    Restore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum CloudBackupOperationGate {
    #[default]
    Idle,
    Running {
        operation_id: u64,
        kind: CloudBackupOperationKind,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloudBackupOperationEvent {
    TryStart { operation_id: u64, kind: CloudBackupOperationKind },
    Complete { operation_id: u64 },
    Fail { operation_id: u64 },
    Cancel { kind: CloudBackupOperationKind },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CloudBackupOperationDecision {
    Start { operation_id: u64, kind: CloudBackupOperationKind },
    Ignore { active_kind: CloudBackupOperationKind },
    Finish { kind: CloudBackupOperationKind },
    Cancel { operation_id: u64, kind: CloudBackupOperationKind },
    Stale,
}

impl CloudBackupOperationGate {
    pub(crate) fn apply(
        &mut self,
        event: CloudBackupOperationEvent,
    ) -> CloudBackupOperationDecision {
        match (*self, event) {
            (Self::Idle, CloudBackupOperationEvent::TryStart { operation_id, kind }) => {
                *self = Self::Running { operation_id, kind };
                CloudBackupOperationDecision::Start { operation_id, kind }
            }
            (Self::Running { kind, .. }, CloudBackupOperationEvent::TryStart { .. }) => {
                CloudBackupOperationDecision::Ignore { active_kind: kind }
            }
            (
                Self::Running { operation_id, kind },
                CloudBackupOperationEvent::Complete { operation_id: current_operation_id }
                | CloudBackupOperationEvent::Fail { operation_id: current_operation_id },
            ) if operation_id == current_operation_id => {
                let active_kind = kind;
                *self = Self::Idle;
                CloudBackupOperationDecision::Finish { kind: active_kind }
            }
            (
                Self::Running { operation_id, kind },
                CloudBackupOperationEvent::Cancel { kind: cancel_kind },
            ) if kind == cancel_kind => {
                let current_operation_id = operation_id;
                let active_kind = kind;
                *self = Self::Idle;
                CloudBackupOperationDecision::Cancel {
                    operation_id: current_operation_id,
                    kind: active_kind,
                }
            }
            (_, CloudBackupOperationEvent::Complete { .. })
            | (_, CloudBackupOperationEvent::Fail { .. })
            | (_, CloudBackupOperationEvent::Cancel { .. }) => CloudBackupOperationDecision::Stale,
        }
    }

    pub(crate) fn is_current(&self, operation_id: u64) -> bool {
        matches!(
            self,
            Self::Running {
                operation_id: current_operation_id,
                ..
            } if *current_operation_id == operation_id
        )
    }

}

#[derive(Debug, Default)]
pub(crate) struct CloudBackupOperationController {
    gate: CloudBackupOperationGate,
    next_operation_id: u64,
}

impl CloudBackupOperationController {
    pub(crate) fn try_start(
        &mut self,
        kind: CloudBackupOperationKind,
    ) -> CloudBackupOperationDecision {
        let operation_id = self.next_operation_id + 1;
        let decision = self.gate.apply(CloudBackupOperationEvent::TryStart { operation_id, kind });
        if matches!(decision, CloudBackupOperationDecision::Start { .. }) {
            self.next_operation_id = operation_id;
        }

        decision
    }

    pub(crate) fn complete(&mut self, operation_id: u64) -> CloudBackupOperationDecision {
        self.gate.apply(CloudBackupOperationEvent::Complete { operation_id })
    }

    pub(crate) fn fail(&mut self, operation_id: u64) -> CloudBackupOperationDecision {
        self.gate.apply(CloudBackupOperationEvent::Fail { operation_id })
    }

    pub(crate) fn cancel(
        &mut self,
        kind: CloudBackupOperationKind,
    ) -> CloudBackupOperationDecision {
        self.gate.apply(CloudBackupOperationEvent::Cancel { kind })
    }

    pub(crate) fn is_current(&self, operation_id: u64) -> bool {
        self.gate.is_current(operation_id)
    }

}

#[cfg(test)]
mod tests {
    use super::{
        CloudBackupOperationController, CloudBackupOperationDecision, CloudBackupOperationGate,
        CloudBackupOperationKind,
    };

    impl CloudBackupOperationGate {
        fn active_kind(&self) -> Option<CloudBackupOperationKind> {
            match self {
                Self::Idle => None,
                Self::Running { kind, .. } => Some(*kind),
            }
        }
    }

    impl CloudBackupOperationController {
        fn active_kind(&self) -> Option<CloudBackupOperationKind> {
            self.gate.active_kind()
        }
    }

    #[test]
    fn gate_starts_when_idle() {
        let mut controller = CloudBackupOperationController::default();

        let decision = controller.try_start(CloudBackupOperationKind::Enable);

        assert_eq!(
            decision,
            CloudBackupOperationDecision::Start {
                operation_id: 1,
                kind: CloudBackupOperationKind::Enable,
            }
        );
        assert_eq!(controller.active_kind(), Some(CloudBackupOperationKind::Enable));
        assert!(controller.is_current(1));
    }

    #[test]
    fn gate_ignores_start_while_running() {
        let mut controller = CloudBackupOperationController::default();
        controller.try_start(CloudBackupOperationKind::Enable);

        let decision = controller.try_start(CloudBackupOperationKind::Restore);

        assert_eq!(
            decision,
            CloudBackupOperationDecision::Ignore { active_kind: CloudBackupOperationKind::Enable }
        );
        assert_eq!(controller.active_kind(), Some(CloudBackupOperationKind::Enable));
    }

    #[test]
    fn gate_finishes_current_operation() {
        let mut controller = CloudBackupOperationController::default();
        controller.try_start(CloudBackupOperationKind::Restore);

        let decision = controller.complete(1);

        assert_eq!(
            decision,
            CloudBackupOperationDecision::Finish { kind: CloudBackupOperationKind::Restore }
        );
        assert_eq!(controller.active_kind(), None);
    }

    #[test]
    fn gate_rejects_stale_completion() {
        let mut controller = CloudBackupOperationController::default();
        controller.try_start(CloudBackupOperationKind::Restore);

        let decision = controller.complete(9);

        assert_eq!(decision, CloudBackupOperationDecision::Stale);
        assert_eq!(controller.active_kind(), Some(CloudBackupOperationKind::Restore));
    }

    #[test]
    fn gate_cancels_matching_operation_kind() {
        let mut controller = CloudBackupOperationController::default();
        controller.try_start(CloudBackupOperationKind::Restore);

        let decision = controller.cancel(CloudBackupOperationKind::Restore);

        assert_eq!(
            decision,
            CloudBackupOperationDecision::Cancel {
                operation_id: 1,
                kind: CloudBackupOperationKind::Restore,
            }
        );
        assert_eq!(controller.active_kind(), None);
    }

    #[test]
    fn gate_ignores_cancel_for_other_kind() {
        let mut controller = CloudBackupOperationController::default();
        controller.try_start(CloudBackupOperationKind::Enable);

        let decision = controller.cancel(CloudBackupOperationKind::Restore);

        assert_eq!(decision, CloudBackupOperationDecision::Stale);
        assert_eq!(controller.active_kind(), Some(CloudBackupOperationKind::Enable));
    }
}
