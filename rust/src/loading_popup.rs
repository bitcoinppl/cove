//! Conditional loading popup helper for async operations

use std::future::Future;

use tokio::time::{Duration, Instant, sleep};

use crate::app::reconcile::{Update, Updater};

const LOADING_POPUP_DELAY_MS: u64 = 125;
const MINIMUM_POPUP_DISPLAY_MS: u64 = 400;

/// Runs an async operation with conditional loading popup
/// Shows popup only if operation takes >125ms, keeps visible for min 400ms
pub async fn with_loading_popup<F, T, E>(operation: F) -> Result<T, E>
where
    F: Future<Output = Result<T, E>>,
{
    use tokio::pin;
    pin!(operation);

    let mut popup_shown_at: Option<Instant> = None;

    // biased checks operation completion first, avoiding popup for fast operations
    let result = tokio::select! {
        biased;

        result = &mut operation => result,

        _ = sleep(Duration::from_millis(LOADING_POPUP_DELAY_MS)) => {
            Updater::send_update(Update::ShowLoadingPopup);
            popup_shown_at = Some(Instant::now());
            operation.await
        }
    };

    if let Some(shown_at) = popup_shown_at {
        let elapsed = shown_at.elapsed();
        let min_display = Duration::from_millis(MINIMUM_POPUP_DISPLAY_MS);

        if elapsed < min_display {
            sleep(min_display - elapsed).await;
        }
        Updater::send_update(Update::HideLoadingPopup);
    }

    result
}
