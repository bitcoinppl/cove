use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use act_zero::{Actor, ActorResult, AddrLike as _, call};
use futures::FutureExt as _;
use tokio::sync::{Notify, oneshot};

use super::{
    CoordinatorPhase, DestructiveClaim, DestructiveIntent, ORDINARY_CLOSE_RETRY_BACKOFF,
    OrdinaryCloseOutcome, RegistrationState, ResumedRegistrationState, ShutdownDeadlineTier,
    WalletActorKind, WalletLifecycleCoordinator, WalletLifecycleFailure,
    WalletManagerLifecycleToken, cache_clear_after_ordinary_close, terminal_shutdown,
    test_support::enter_wallet_deletion,
};

#[derive(Debug)]
struct TestActor {
    release: Arc<Notify>,
}

#[async_trait::async_trait]
impl Actor for TestActor {
    async fn error(&mut self, _error: act_zero::ActorError) -> bool {
        false
    }
}

impl TestActor {
    async fn fail(&mut self) -> ActorResult<()> {
        Err("expected failure".into())
    }

    async fn probe(&mut self) -> ActorResult<()> {
        Ok(act_zero::Produces::Value(()))
    }

    async fn block(&mut self, started: oneshot::Sender<()>) -> ActorResult<()> {
        let _ = started.send(());
        self.release.notified().await;
        Ok(act_zero::Produces::Value(()))
    }
}

fn spawn_actor(release: Arc<Notify>) -> act_zero::Addr<TestActor> {
    crate::test_support::ensure_tokio_runtime();
    cove_tokio::task::spawn_actor(TestActor { release })
}

fn register_preview_wallet(
    coordinator: &'static WalletLifecycleCoordinator,
) -> (
    crate::wallet::metadata::WalletId,
    u64,
    crate::manager::wallet_manager::RustWalletManager,
    super::WalletActorRegistration,
) {
    crate::test_support::ensure_tokio_runtime();
    crate::test_support::init_test_keychain();
    let manager = crate::manager::wallet_manager::RustWalletManager::preview_new_wallet();
    let wallet_id = manager.id.clone();
    let construction =
        coordinator.begin_construction(wallet_id.clone()).expect("construction starts");
    let (registration, _) = construction.register(wallet_id.clone(), manager.actor.clone(), None);
    let registration_id = registration._lease.registration_id;

    (wallet_id, registration_id, manager, registration)
}

fn mark_ordinary_closing(coordinator: &WalletLifecycleCoordinator, registration_id: u64) {
    let mut data = coordinator.data.lock();
    let actors = data.actors.get_mut(&registration_id).expect("registration exists");
    actors.state = RegistrationState::OrdinaryClosing;
    actors.lifecycle.mark_closing();
}

#[tokio::test]
async fn ordinary_failed_call_does_not_terminate_actor() {
    let actor = spawn_actor(Arc::new(Notify::new()));

    assert!(call!(actor.fail()).await.is_err());
    call!(actor.probe()).await.expect("actor remains active");
}

#[tokio::test]
async fn successful_terminal_request_awaits_termination() {
    let actor = spawn_actor(Arc::new(Notify::new()));
    let result = terminal_shutdown(
        actor.clone(),
        tokio::time::Instant::now() + std::time::Duration::from_secs(1),
        |_actor| Box::pin(async { Ok(()) }),
    )
    .await;

    assert_eq!(result, Ok(()));
    assert!(actor.termination().now_or_never().is_some());
}

#[tokio::test]
async fn already_terminated_actor_is_idempotent_success() {
    let actor = spawn_actor(Arc::new(Notify::new()));
    let quiesce_calls = Arc::new(AtomicUsize::new(0));
    let first_calls = quiesce_calls.clone();
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(1);

    terminal_shutdown(actor.clone(), deadline, move |_actor| {
        first_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(()) })
    })
    .await
    .expect("first terminal request succeeds");

    let second_calls = quiesce_calls.clone();
    terminal_shutdown(actor, deadline, move |_actor| {
        second_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(()) })
    })
    .await
    .expect("terminated actor is already safe");

    assert_eq!(quiesce_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn failed_terminal_request_leaves_actor_active() {
    let actor = spawn_actor(Arc::new(Notify::new()));
    let result = terminal_shutdown(
        actor.clone(),
        tokio::time::Instant::now() + std::time::Duration::from_secs(1),
        |_actor| Box::pin(async { Err("blocked".to_string()) }),
    )
    .await;

    assert_eq!(result, Err(()));
    call!(actor.probe()).await.expect("actor remains active");
}

#[tokio::test]
async fn cancelled_queued_terminal_request_cannot_terminate_actor_later() {
    let release = Arc::new(Notify::new());
    let actor = spawn_actor(release.clone());
    let (started_sender, started_receiver) = oneshot::channel();
    actor.send_mut(Box::new(move |actor| {
        Box::pin(async move {
            let _ = actor.block(started_sender).await;
            false
        })
    }));
    started_receiver.await.expect("blocking call starts");

    let result = terminal_shutdown(
        actor.clone(),
        tokio::time::Instant::now() + std::time::Duration::from_millis(10),
        |_actor| Box::pin(async { Ok(()) }),
    )
    .await;
    assert_eq!(result, Err(()));

    release.notify_one();
    call!(actor.probe()).await.expect("actor remains active");
}

#[tokio::test]
async fn cancelled_running_terminal_request_cannot_terminate_actor_later() {
    let actor = spawn_actor(Arc::new(Notify::new()));
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(10);

    let result = terminal_shutdown(actor.clone(), deadline, |_actor| {
        Box::pin(async {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            Ok(())
        })
    })
    .await;

    assert_eq!(result, Err(()));
    call!(actor.probe()).await.expect("actor remains active");
}

#[tokio::test]
async fn non_cooperative_terminal_request_still_obeys_deadline() {
    let actor = spawn_actor(Arc::new(Notify::new()));
    let release = Arc::new(AtomicBool::new(false));
    let release_for_request = release.clone();
    let release_after_delay = release.clone();
    let releaser = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        release_after_delay.store(true, Ordering::Release);
    });
    let started_at = std::time::Instant::now();

    let result = terminal_shutdown(
        actor.clone(),
        tokio::time::Instant::now() + Duration::from_millis(20),
        move |_actor| {
            Box::pin(async move {
                while !release_for_request.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(5));
                }

                Ok(())
            })
        },
    )
    .await;
    let elapsed = started_at.elapsed();

    releaser.join().expect("deadline test releaser joins");

    assert_eq!(result, Err(()));
    assert!(elapsed < Duration::from_millis(250), "terminal shutdown exceeded its deadline");
    call!(actor.probe()).await.expect("actor remains active");
}

#[test]
fn same_wallet_construction_returns_typed_closing_state_without_waiting() {
    let coordinator: &'static WalletLifecycleCoordinator =
        Box::leak(Box::new(WalletLifecycleCoordinator::default()));
    let (wallet_id, registration_id, manager, registration) = register_preview_wallet(coordinator);
    mark_ordinary_closing(coordinator, registration_id);

    let started = std::time::Instant::now();
    assert_eq!(
        coordinator.begin_construction(wallet_id.clone()).err(),
        Some(WalletLifecycleFailure::ManagerClosing { wallet_id })
    );
    assert!(
        started.elapsed() < std::time::Duration::from_millis(100),
        "synchronous construction must not wait for actor shutdown"
    );

    coordinator.data.lock().actors.remove(&registration_id);
    std::mem::forget(registration);
    drop(manager);
}

#[tokio::test]
async fn retryable_ordinary_close_is_retried_without_a_new_construction() {
    let coordinator: &'static WalletLifecycleCoordinator =
        Box::leak(Box::new(WalletLifecycleCoordinator::default()));
    let (wallet_id, registration_id, manager, registration) = register_preview_wallet(coordinator);
    mark_ordinary_closing(coordinator, registration_id);

    coordinator.finish_ordinary_close(registration_id, OrdinaryCloseOutcome::Retryable);
    assert_eq!(
        coordinator.data.lock().actors.get(&registration_id).map(|actors| actors.state),
        Some(RegistrationState::OrdinaryClosePending)
    );

    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let notified = coordinator.changed.notified();
            if !coordinator.data.lock().actors.contains_key(&registration_id) {
                break;
            }

            notified.await;
        }
    })
    .await
    .expect("the coordinator-owned retry closes the actors");

    assert!(coordinator.begin_construction(wallet_id).is_ok());

    std::mem::forget(registration);
    drop(manager);
}

#[tokio::test]
async fn ordinary_close_retry_waits_for_failed_destructive_phase_to_release() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    crate::test_support::init_test_keychain();
    crate::database::test_support::init_test_database();

    let coordinator: &'static WalletLifecycleCoordinator =
        Box::leak(Box::new(WalletLifecycleCoordinator::default()));
    let (wallet_id, registration_id, manager, registration) = register_preview_wallet(coordinator);
    {
        let mut data = coordinator.data.lock();
        let actors = data.actors.get_mut(&registration_id).expect("registration exists");
        actors.state = RegistrationState::OrdinaryClosePending;
        actors.lifecycle.mark_closing();
    }

    coordinator.arm_ordinary_close_retry(registration_id);
    let retry_id = coordinator
        .data
        .lock()
        .actors
        .get(&registration_id)
        .and_then(|actors| actors.ordinary_close_retry)
        .expect("ordinary close retry is armed");

    enter_wallet_deletion(coordinator, wallet_id.clone(), ShutdownDeadlineTier::Initial, None)
        .expect("deletion preparation starts");
    let DestructiveClaim::Claimed(targets) = coordinator
        .try_claim_destructive_registrations(Some(&wallet_id))
        .expect("destructive shutdown claims the pending registration")
    else {
        panic!("destructive shutdown must claim the registration")
    };
    assert_eq!(
        coordinator.restore_after_failed_destructive_quiesce(registration_id),
        Some(ResumedRegistrationState::OrdinaryClosePending)
    );
    tokio::time::sleep(ORDINARY_CLOSE_RETRY_BACKOFF + Duration::from_millis(100)).await;
    {
        let data = coordinator.data.lock();
        let actors = data.actors.get(&registration_id).expect("registration remains pending");
        assert_eq!(actors.state, RegistrationState::OrdinaryClosePending);
        assert_eq!(actors.ordinary_close_retry, Some(retry_id));
        assert_eq!(actors.next_ordinary_close_retry_id, 1);
    }

    drop(targets);
    coordinator.release_failed_preparation();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let notified = coordinator.changed.notified();
            if !coordinator.data.lock().actors.contains_key(&registration_id) {
                break;
            }

            notified.await;
        }
    })
    .await
    .expect("the retained retry closes after destructive ownership releases");

    assert!(coordinator.begin_construction(wallet_id).is_ok());

    std::mem::forget(registration);
    drop(manager);
}

async fn wait_for_registration_removal(
    coordinator: &'static WalletLifecycleCoordinator,
    registration_id: u64,
) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let notified = coordinator.changed.notified();
            if !coordinator.data.lock().actors.contains_key(&registration_id) {
                break;
            }

            notified.await;
        }
    })
    .await
    .expect("ordinary close retry removes the registration");
}

#[tokio::test]
async fn lease_drop_before_destructive_claim_survives_blocked_shutdown() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    crate::test_support::ensure_tokio_runtime();
    crate::test_support::init_test_keychain();
    crate::database::test_support::init_test_database();

    let coordinator: &'static WalletLifecycleCoordinator =
        Box::leak(Box::new(WalletLifecycleCoordinator::default()));
    let (wallet_id, registration_id, manager, registration) = register_preview_wallet(coordinator);
    let lifecycle = coordinator
        .data
        .lock()
        .actors
        .get(&registration_id)
        .expect("registration exists")
        .lifecycle
        .clone();
    enter_wallet_deletion(coordinator, wallet_id.clone(), ShutdownDeadlineTier::Initial, None)
        .expect("deletion preparation starts");

    coordinator.close_registration(registration_id);
    std::mem::forget(registration);
    {
        let data = coordinator.data.lock();
        let actors = data.actors.get(&registration_id).expect("registration remains owned");
        assert_eq!(actors.state, RegistrationState::OrdinaryClosePending);
        assert_eq!(actors.ordinary_close_retry, Some(0));
        assert_eq!(actors.next_ordinary_close_retry_id, 1);
    }
    assert!(lifecycle.ensure_active().is_err());

    tokio::time::sleep(ORDINARY_CLOSE_RETRY_BACKOFF + Duration::from_millis(100)).await;
    {
        let data = coordinator.data.lock();
        let actors = data.actors.get(&registration_id).expect("registration remains owned");
        assert_eq!(actors.state, RegistrationState::OrdinaryClosePending);
        assert_eq!(actors.ordinary_close_retry, Some(0));
    }

    let DestructiveClaim::Claimed(targets) = coordinator
        .try_claim_destructive_registrations(Some(&wallet_id))
        .expect("destructive shutdown claims the registration")
    else {
        panic!("destructive shutdown must claim the registration");
    };
    assert_eq!(
        coordinator.data.lock().actors.get(&registration_id).map(|actors| actors.state),
        Some(RegistrationState::DestructiveClosing {
            resume: ResumedRegistrationState::OrdinaryClosePending,
        })
    );
    drop(targets);

    assert_eq!(
        coordinator.restore_after_failed_destructive_quiesce(registration_id),
        Some(ResumedRegistrationState::OrdinaryClosePending)
    );
    assert!(lifecycle.ensure_active().is_err());
    let blocked = coordinator.finish_blocked_attempt(
        DestructiveIntent::Delete(wallet_id.clone()),
        vec![WalletActorKind::Wallet],
        ShutdownDeadlineTier::Initial,
        None,
    );
    assert!(matches!(blocked, WalletLifecycleFailure::ShutdownBlocked { .. }));

    wait_for_registration_removal(coordinator, registration_id).await;
    let construction = coordinator
        .begin_construction(wallet_id)
        .expect("wallet reconstruction follows ordinary close");
    drop(construction);
    drop(manager);
}

#[tokio::test]
async fn lease_drop_after_destructive_claim_updates_failed_resume_target() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    crate::test_support::ensure_tokio_runtime();
    crate::test_support::init_test_keychain();
    crate::database::test_support::init_test_database();

    let coordinator: &'static WalletLifecycleCoordinator =
        Box::leak(Box::new(WalletLifecycleCoordinator::default()));
    let (wallet_id, registration_id, manager, registration) = register_preview_wallet(coordinator);
    let lifecycle = coordinator
        .data
        .lock()
        .actors
        .get(&registration_id)
        .expect("registration exists")
        .lifecycle
        .clone();
    enter_wallet_deletion(coordinator, wallet_id.clone(), ShutdownDeadlineTier::Initial, None)
        .expect("deletion preparation starts");
    let DestructiveClaim::Claimed(targets) = coordinator
        .try_claim_destructive_registrations(Some(&wallet_id))
        .expect("destructive shutdown claims the registration")
    else {
        panic!("destructive shutdown must claim the registration");
    };
    assert_eq!(
        coordinator.data.lock().actors.get(&registration_id).map(|actors| actors.state),
        Some(RegistrationState::DestructiveClosing { resume: ResumedRegistrationState::Active })
    );

    coordinator.close_registration(registration_id);
    std::mem::forget(registration);
    assert_eq!(
        coordinator.data.lock().actors.get(&registration_id).map(|actors| (
            actors.state,
            actors.ordinary_close_retry,
            actors.next_ordinary_close_retry_id
        )),
        Some((
            RegistrationState::DestructiveClosing {
                resume: ResumedRegistrationState::OrdinaryClosePending,
            },
            Some(0),
            1,
        ))
    );

    tokio::time::sleep(ORDINARY_CLOSE_RETRY_BACKOFF + Duration::from_millis(100)).await;
    assert_eq!(
        coordinator.data.lock().actors.get(&registration_id).map(|actors| actors.state),
        Some(RegistrationState::DestructiveClosing {
            resume: ResumedRegistrationState::OrdinaryClosePending,
        })
    );
    drop(targets);

    assert_eq!(
        coordinator.restore_after_failed_destructive_quiesce(registration_id),
        Some(ResumedRegistrationState::OrdinaryClosePending)
    );
    assert!(lifecycle.ensure_active().is_err());
    let blocked = coordinator.finish_blocked_attempt(
        DestructiveIntent::Delete(wallet_id.clone()),
        vec![WalletActorKind::Wallet],
        ShutdownDeadlineTier::Initial,
        None,
    );
    assert!(matches!(blocked, WalletLifecycleFailure::ShutdownBlocked { .. }));

    wait_for_registration_removal(coordinator, registration_id).await;
    let construction = coordinator
        .begin_construction(wallet_id)
        .expect("wallet reconstruction follows ordinary close");
    drop(construction);
    drop(manager);
}

#[tokio::test]
async fn lease_drop_after_failed_restore_marks_lifecycle_closing() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    crate::test_support::ensure_tokio_runtime();
    crate::test_support::init_test_keychain();
    crate::database::test_support::init_test_database();

    let coordinator: &'static WalletLifecycleCoordinator =
        Box::leak(Box::new(WalletLifecycleCoordinator::default()));
    let (wallet_id, registration_id, manager, registration) = register_preview_wallet(coordinator);
    let lifecycle = coordinator
        .data
        .lock()
        .actors
        .get(&registration_id)
        .expect("registration exists")
        .lifecycle
        .clone();
    enter_wallet_deletion(coordinator, wallet_id.clone(), ShutdownDeadlineTier::Initial, None)
        .expect("deletion preparation starts");
    let DestructiveClaim::Claimed(targets) = coordinator
        .try_claim_destructive_registrations(Some(&wallet_id))
        .expect("destructive shutdown claims the registration")
    else {
        panic!("destructive shutdown must claim the registration");
    };

    assert_eq!(
        coordinator.restore_after_failed_destructive_quiesce(registration_id),
        Some(ResumedRegistrationState::Active)
    );
    assert_eq!(
        coordinator.data.lock().actors.get(&registration_id).map(|actors| actors.state),
        Some(RegistrationState::Active)
    );
    assert!(lifecycle.ensure_active().is_ok());

    coordinator.close_registration(registration_id);
    std::mem::forget(registration);
    assert_eq!(
        coordinator.data.lock().actors.get(&registration_id).map(|actors| actors.state),
        Some(RegistrationState::OrdinaryClosePending)
    );
    assert!(lifecycle.ensure_active().is_err());

    drop(targets);
    let blocked = coordinator.finish_blocked_attempt(
        DestructiveIntent::Delete(wallet_id.clone()),
        vec![WalletActorKind::Wallet],
        ShutdownDeadlineTier::Initial,
        None,
    );
    assert!(matches!(blocked, WalletLifecycleFailure::ShutdownBlocked { .. }));

    wait_for_registration_removal(coordinator, registration_id).await;
    let construction = coordinator
        .begin_construction(wallet_id)
        .expect("wallet reconstruction follows ordinary close");
    drop(construction);
    drop(manager);
}

#[test]
fn successful_ordinary_close_does_not_clear_a_same_wallet_replacement() {
    let wallet_id = crate::wallet::metadata::WalletId::preview_new_random();

    assert_eq!(
        cache_clear_after_ordinary_close(OrdinaryCloseOutcome::Closed, &wallet_id),
        None,
        "ordinary close was requested by the cache owner and must not send a stale clear"
    );
    assert_eq!(
        cache_clear_after_ordinary_close(OrdinaryCloseOutcome::RecoveryRequired, &wallet_id,),
        Some(wallet_id),
        "recovery-required state still invalidates the unsafe cached manager"
    );
}

#[test]
fn unrecoverable_ordinary_close_requires_process_restart() {
    let coordinator: &'static WalletLifecycleCoordinator =
        Box::leak(Box::new(WalletLifecycleCoordinator::default()));
    let (wallet_id, registration_id, manager, registration) = register_preview_wallet(coordinator);
    mark_ordinary_closing(coordinator, registration_id);

    coordinator.finish_ordinary_close(registration_id, OrdinaryCloseOutcome::RecoveryRequired);

    assert_eq!(
        coordinator.begin_construction(wallet_id.clone()).err(),
        Some(WalletLifecycleFailure::ManagerRecoveryRequired { wallet_id })
    );

    coordinator.data.lock().actors.remove(&registration_id);
    std::mem::forget(registration);
    drop(manager);
}

#[test]
fn destructive_preparation_keeps_normal_close_persistence_available() {
    let coordinator: &'static WalletLifecycleCoordinator =
        Box::leak(Box::new(WalletLifecycleCoordinator::default()));
    let (wallet_id, registration_id, manager, registration) = register_preview_wallet(coordinator);
    mark_ordinary_closing(coordinator, registration_id);

    let (result_sender, result_receiver) = std::sync::mpsc::sync_channel(1);
    let deletion_wallet_id = wallet_id.clone();
    std::thread::spawn(move || {
        let result = cove_tokio::try_block_on(coordinator.prepare_wallet_deletion(
            deletion_wallet_id,
            ShutdownDeadlineTier::Initial,
            None,
        ))
        .expect("runtime bridge is available");
        result_sender.send(result.is_ok()).expect("send preparation result");
    });

    let reservation_started = (0..100).any(|_| {
        if coordinator.data.lock().pending_preparation.is_some() {
            return true;
        }

        std::thread::sleep(std::time::Duration::from_millis(1));
        false
    });
    assert!(reservation_started, "destructive preparation reserves ownership");
    assert_eq!(coordinator.data.lock().phase, CoordinatorPhase::Available);
    assert!(matches!(
        coordinator.reserve_preparation(
            DestructiveIntent::FullWipe,
            ShutdownDeadlineTier::Initial,
            None,
        ),
        Err(WalletLifecycleFailure::CoordinatorBusy)
    ));
    assert!(matches!(
        coordinator.begin_recovery_cleanup(),
        Err(WalletLifecycleFailure::CoordinatorBusy)
    ));

    let persistence = coordinator
        .begin_persistence_operation(wallet_id.clone())
        .expect("ordinary close persistence remains available");
    drop(persistence);

    coordinator.finish_ordinary_close(registration_id, OrdinaryCloseOutcome::Retryable);
    assert!(
        result_receiver
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("destructive preparation finishes after ordinary close")
    );

    std::mem::forget(registration);
    drop(manager);
}

#[test]
fn ordinary_close_defers_to_claimed_destructive_phase() {
    let coordinator: &'static WalletLifecycleCoordinator =
        Box::leak(Box::new(WalletLifecycleCoordinator::default()));
    let (wallet_id, registration_id, manager, registration) = register_preview_wallet(coordinator);
    enter_wallet_deletion(coordinator, wallet_id.clone(), ShutdownDeadlineTier::Initial, None)
        .expect("deletion preparation starts");

    coordinator.close_registration(registration_id);
    assert_eq!(
        coordinator.data.lock().actors.get(&registration_id).map(|actors| actors.state),
        Some(RegistrationState::OrdinaryClosePending)
    );

    let super::DestructiveClaim::Claimed(targets) = coordinator
        .try_claim_destructive_registrations(Some(&wallet_id))
        .expect("destructive shutdown claims the actors")
    else {
        panic!("destructive shutdown must not wait for ordinary close");
    };
    assert_eq!(targets.len(), 1);
    assert!(matches!(
        coordinator.data.lock().actors.get(&registration_id).map(|actors| actors.state),
        Some(RegistrationState::DestructiveClosing { .. })
    ));

    coordinator.release_failed_preparation();
    coordinator.data.lock().actors.remove(&registration_id);
    drop(targets);
    std::mem::forget(registration);
    drop(manager);
}

#[test]
fn deadline_tiers_escalate_from_five_to_twenty_seconds() {
    assert_eq!(ShutdownDeadlineTier::Initial.duration(), std::time::Duration::from_secs(5));
    assert_eq!(ShutdownDeadlineTier::Retry.duration(), std::time::Duration::from_secs(20));
}

#[test]
fn cancelled_attempt_cannot_authorize_retry() {
    let coordinator: &'static WalletLifecycleCoordinator =
        Box::leak(Box::new(WalletLifecycleCoordinator::default()));
    let wallet_id = crate::wallet::metadata::WalletId::preview_new_random();
    let intent = DestructiveIntent::Delete(wallet_id.clone());
    let blocked = coordinator.finish_blocked_attempt(
        intent.clone(),
        vec![],
        ShutdownDeadlineTier::Initial,
        None,
    );
    let WalletLifecycleFailure::ShutdownBlocked { attempt_id, .. } = blocked else {
        panic!("expected blocked attempt");
    };

    coordinator.cancel_attempt(&attempt_id);

    assert_eq!(
        enter_wallet_deletion(
            coordinator,
            wallet_id,
            ShutdownDeadlineTier::Retry,
            Some(&attempt_id),
        ),
        Err(WalletLifecycleFailure::CoordinatorBusy)
    );
    assert_eq!(coordinator.data.lock().phase, CoordinatorPhase::Available);
}

#[test]
fn recovery_cleanup_blocks_new_wallet_work_until_release() {
    let coordinator = Box::leak(Box::new(WalletLifecycleCoordinator::default()));
    let cleanup = coordinator.begin_recovery_cleanup().expect("cleanup is reserved");
    let wallet_id = crate::wallet::metadata::WalletId::preview_new_random();

    assert!(matches!(
        coordinator.begin_construction(wallet_id.clone()),
        Err(WalletLifecycleFailure::CoordinatorBusy)
    ));
    assert!(matches!(
        coordinator.begin_persistence_operation(wallet_id.clone()),
        Err(WalletLifecycleFailure::CoordinatorBusy)
    ));

    drop(cleanup);

    assert!(coordinator.begin_persistence_operation(wallet_id).is_ok());
}

#[test]
fn same_wallet_constructions_are_serialized() {
    let coordinator: &'static WalletLifecycleCoordinator =
        Box::leak(Box::new(WalletLifecycleCoordinator::default()));
    let wallet_id = crate::wallet::metadata::WalletId::preview_new_random();
    let first =
        coordinator.begin_construction(wallet_id.clone()).expect("first construction starts");

    assert!(matches!(
        coordinator.begin_construction(wallet_id.clone()),
        Err(WalletLifecycleFailure::ConstructionInProgress { wallet_id: id }) if id == wallet_id
    ));

    drop(first);
    assert!(coordinator.begin_construction(wallet_id).is_ok());
}

#[tokio::test]
async fn cancelled_preparation_finishes_in_the_coordinator_owned_task() {
    crate::test_support::ensure_tokio_runtime();
    let coordinator: &'static WalletLifecycleCoordinator =
        Box::leak(Box::new(WalletLifecycleCoordinator::default()));
    let wallet_id = crate::wallet::metadata::WalletId::preview_new_random();
    let persistence =
        coordinator.begin_persistence_operation(wallet_id.clone()).expect("operation starts");
    let task = tokio::spawn(async move {
        coordinator.prepare_wallet_deletion(wallet_id, ShutdownDeadlineTier::Initial, None).await
    });

    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let notified = coordinator.changed.notified();
            if coordinator.data.lock().phase != CoordinatorPhase::Available {
                break;
            }

            notified.await;
        }
    })
    .await
    .expect("preparation reserves ownership");

    task.abort();
    drop(persistence);

    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        loop {
            let notified = coordinator.changed.notified();
            let done = {
                let data = coordinator.data.lock();
                data.phase == CoordinatorPhase::Available && data.pending_preparation.is_none()
            };
            if done {
                break;
            }

            notified.await;
        }
    })
    .await
    .expect("coordinator-owned preparation releases after cancellation");
}

#[tokio::test]
async fn cancelled_preparation_converges_a_real_destructive_registration() {
    let _guard = crate::test_support::global_state_test_lock().lock().await;
    crate::test_support::ensure_tokio_runtime();
    crate::test_support::init_test_keychain();
    crate::database::test_support::init_test_database();

    let coordinator: &'static WalletLifecycleCoordinator =
        Box::leak(Box::new(WalletLifecycleCoordinator::default()));
    let (wallet_id, registration_id, manager, registration) = register_preview_wallet(coordinator);
    let release_blocked_call = Arc::new(Notify::new());
    let release_blocked_call_for_actor = release_blocked_call.clone();
    let (blocked_call_started_sender, blocked_call_started_receiver) = oneshot::channel();
    manager.actor.send_mut(Box::new(move |_actor| {
        Box::pin(async move {
            let _ = blocked_call_started_sender.send(());
            release_blocked_call_for_actor.notified().await;
            false
        })
    }));
    blocked_call_started_receiver.await.expect("blocking wallet call starts");

    let preparation = tokio::spawn(async move {
        coordinator.prepare_wallet_deletion(wallet_id, ShutdownDeadlineTier::Initial, None).await
    });

    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let entered_destructive_closing = {
                let data = coordinator.data.lock();
                matches!(
                    data.actors.get(&registration_id).map(|actors| actors.state),
                    Some(RegistrationState::DestructiveClosing { .. })
                )
            };
            if entered_destructive_closing {
                break;
            }

            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("real registration enters destructive closing");

    preparation.abort();
    assert!(matches!(
        coordinator.data.lock().actors.get(&registration_id).map(|actors| actors.state),
        Some(RegistrationState::DestructiveClosing { .. })
    ));

    release_blocked_call.notify_one();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let notified = coordinator.changed.notified();
            let done = {
                let data = coordinator.data.lock();
                data.phase == CoordinatorPhase::Available
                    && !data.actors.contains_key(&registration_id)
            };
            if done {
                break;
            }

            notified.await;
        }
    })
    .await
    .expect("detached preparation converges phase and registration");

    std::mem::forget(registration);
    drop(manager);
}

#[test]
fn preparation_allows_owned_nested_work_but_rejects_new_external_work() {
    let coordinator: &'static WalletLifecycleCoordinator =
        Box::leak(Box::new(WalletLifecycleCoordinator::default()));
    let wallet_id = crate::wallet::metadata::WalletId::preview_new_random();
    let construction =
        coordinator.begin_construction(wallet_id.clone()).expect("construction starts");
    enter_wallet_deletion(coordinator, wallet_id.clone(), ShutdownDeadlineTier::Initial, None)
        .expect("deletion preparation starts");

    let nested = coordinator
        .begin_persistence_operation(wallet_id.clone())
        .expect("owned nested write can finish");
    let external_coordinator = coordinator;
    let external = std::thread::spawn(move || {
        matches!(
            external_coordinator.begin_persistence_operation(wallet_id),
            Err(WalletLifecycleFailure::CoordinatorBusy)
        )
    })
    .join()
    .expect("external operation thread joins");

    assert!(external);

    drop(nested);
    drop(construction);
    coordinator.release_failed_preparation();
}

#[test]
fn terminal_payjoin_authority_allows_only_its_wallet_and_phase() {
    let coordinator: &'static WalletLifecycleCoordinator =
        Box::leak(Box::new(WalletLifecycleCoordinator::default()));
    let wallet_id = crate::wallet::metadata::WalletId::preview_new_random();
    let other_wallet_id = crate::wallet::metadata::WalletId::preview_new_random();
    enter_wallet_deletion(coordinator, wallet_id.clone(), ShutdownDeadlineTier::Initial, None)
        .expect("deletion preparation starts");
    let authority = coordinator.terminal_payjoin_authority(wallet_id.clone());

    let persistence = authority.begin(&wallet_id).expect("terminal fallback write is allowed");
    assert!(matches!(
        authority.begin(&other_wallet_id),
        Err(WalletLifecycleFailure::CoordinatorBusy)
    ));
    assert!(matches!(
        coordinator.begin_persistence_operation(wallet_id.clone()),
        Err(WalletLifecycleFailure::CoordinatorBusy)
    ));
    drop(persistence);

    coordinator.release_failed_preparation();
    assert!(matches!(authority.begin(&wallet_id), Err(WalletLifecycleFailure::CoordinatorBusy)));
}

#[test]
fn stale_label_manager_rejects_storage_access() {
    let id = crate::wallet::metadata::WalletId::preview_new_random();
    let db = crate::database::wallet_data::WalletDataDb::new_in_memory(id)
        .expect("in-memory wallet data opens");
    let lifecycle = WalletManagerLifecycleToken::new();
    let labels =
        crate::label_manager::LabelManager::new_with_db(db).with_lifecycle(lifecycle.clone());

    assert!(!labels.has_labels().expect("active manager reads labels"));

    lifecycle.mark_closing();

    assert!(matches!(
        labels.has_labels(),
        Err(crate::label_manager::LabelManagerError::ManagerClosed)
    ));
}
