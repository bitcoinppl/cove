//! Whether a payment to a Payjoin request offers the receiver a Payjoin

use cove_device::device::{BuildChannel, Device};
use cove_types::address::PayJoinParams;

use crate::{
    database::Database, manager::wallet_manager::payjoin::PayjoinIntent,
    router::UnsignedPaymentMode,
};

/// How this build ships Payjoin
///
/// Whether Payjoin is on by default, an opt-in beta or internal only is still being decided, so
/// that decision is this one value
pub(crate) const PAYJOIN_ROLLOUT: PayjoinRollout = PayjoinRollout::OnByDefault;

/// The ways Payjoin can ship
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // only one rollout is chosen at a time
pub(crate) enum PayjoinRollout {
    /// Every build offers Payjoin, on until the user switches it off in Settings
    OnByDefault,

    /// Every build offers Payjoin as a beta, off until the user switches it on in Settings
    OptInBeta,

    /// Only debug and beta builds offer Payjoin, on until the user switches it off there
    InternalBuildsOnly,
}

impl PayjoinRollout {
    /// The setting this rollout gives a build on `channel`, given the user's saved choice
    const fn setting(self, channel: BuildChannel, choice: Option<PayjoinChoice>) -> PayjoinSetting {
        let (enabled_by_default, beta) = match self {
            Self::OnByDefault => (true, false),
            Self::OptInBeta => (false, true),
            Self::InternalBuildsOnly => match channel {
                BuildChannel::Debug | BuildChannel::Beta => (true, false),
                BuildChannel::Release => return PayjoinSetting::Unavailable,
            },
        };

        let enabled = match choice {
            Some(choice) => matches!(choice, PayjoinChoice::On),
            None => enabled_by_default,
        };

        PayjoinSetting::Available { enabled, beta }
    }

    /// Whether this rollout depends on which kind of build is running
    const fn needs_build_channel(self) -> bool {
        matches!(self, Self::InternalBuildsOnly)
    }
}

/// The user's own Payjoin choice from Settings, saved in the global config
///
/// Only an explicit choice is saved, so a user who never touched the switch follows the
/// rollout's default even after the rollout changes
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display, strum::EnumString)]
#[strum(serialize_all = "snake_case")]
pub(crate) enum PayjoinChoice {
    On,
    Off,
}

impl From<bool> for PayjoinChoice {
    fn from(enabled: bool) -> Self {
        if enabled { Self::On } else { Self::Off }
    }
}

/// Payjoin as the Settings screen shows it
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq, uniffi::Enum)]
pub enum PayjoinSetting {
    /// This build doesn't offer Payjoin, so Settings has no Payjoin switch and every payment is
    /// sent as an ordinary one
    Unavailable,

    /// Settings shows a Payjoin switch, labelled beta when `beta` is set
    Available { enabled: bool, beta: bool },
}

impl PayjoinSetting {
    /// The setting for this build and the user's saved choice
    pub(crate) fn current() -> Self {
        Self::for_choice(Database::global().global_config.payjoin_choice())
    }

    /// The setting for this build, given the user's saved `choice`
    pub(crate) fn for_choice(choice: Option<PayjoinChoice>) -> Self {
        // only an internal-builds rollout asks the platform, so the other rollouts need no
        // device and a settings read can't fail on it
        let channel = if PAYJOIN_ROLLOUT.needs_build_channel() {
            Device::global().build_channel()
        } else {
            BuildChannel::Release
        };

        PAYJOIN_ROLLOUT.setting(channel, choice)
    }

    pub(crate) const fn is_enabled(self) -> bool {
        matches!(self, Self::Available { enabled: true, .. })
    }

    /// How a confirmed payment to a request carrying `payjoin` is sent
    ///
    /// Only a v2 endpoint is ever parsed into `PayJoinParams`, so a BIP 78 request is always
    /// paid as an ordinary payment
    pub(crate) fn payment_mode(self, payjoin: Option<&PayJoinParams>) -> UnsignedPaymentMode {
        match payjoin {
            Some(payjoin) if self.is_enabled() => UnsignedPaymentMode::Payjoin {
                intent: PayjoinIntent::new(payjoin.endpoint.clone()),
            },
            _ => UnsignedPaymentMode::Standard,
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::database::{Database, global_config::GlobalConfigKey};

    /// Saves the user's Payjoin choice for one test, and forgets it again when dropped, even if
    /// the test panics, so later tests get the rollout's default
    pub(crate) struct PayjoinChoiceGuard;

    impl PayjoinChoiceGuard {
        pub(crate) fn set(enabled: bool) -> Self {
            Database::global()
                .global_config
                .set_payjoin_enabled(enabled)
                .expect("the Payjoin choice saves");

            Self
        }
    }

    impl Drop for PayjoinChoiceGuard {
        fn drop(&mut self) {
            let _ = Database::global().global_config.delete(GlobalConfigKey::PayjoinChoice);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHANNELS: [BuildChannel; 3] =
        [BuildChannel::Debug, BuildChannel::Beta, BuildChannel::Release];

    #[test]
    fn each_rollout_sets_the_default_and_respects_an_explicit_choice() {
        for channel in CHANNELS {
            let on_by_default = PayjoinRollout::OnByDefault;
            assert_eq!(
                on_by_default.setting(channel, None),
                PayjoinSetting::Available { enabled: true, beta: false }
            );
            assert_eq!(
                on_by_default.setting(channel, Some(PayjoinChoice::Off)),
                PayjoinSetting::Available { enabled: false, beta: false }
            );

            let opt_in = PayjoinRollout::OptInBeta;
            assert_eq!(
                opt_in.setting(channel, None),
                PayjoinSetting::Available { enabled: false, beta: true }
            );
            assert_eq!(
                opt_in.setting(channel, Some(PayjoinChoice::On)),
                PayjoinSetting::Available { enabled: true, beta: true }
            );
        }

        let internal = PayjoinRollout::InternalBuildsOnly;
        for channel in [BuildChannel::Debug, BuildChannel::Beta] {
            assert_eq!(
                internal.setting(channel, None),
                PayjoinSetting::Available { enabled: true, beta: false }
            );
        }

        assert_eq!(
            internal.setting(BuildChannel::Release, Some(PayjoinChoice::On)),
            PayjoinSetting::Unavailable,
            "a release build never offers Payjoin, whatever an earlier build saved"
        );
    }

    #[test]
    fn only_an_enabled_setting_pays_a_payjoin_request_with_a_payjoin() {
        let request = PayJoinParams { endpoint: "https://payjo.in/example".to_string() };
        let enabled = PayjoinSetting::Available { enabled: true, beta: false };
        let disabled = PayjoinSetting::Available { enabled: false, beta: false };

        let UnsignedPaymentMode::Payjoin { intent } = enabled.payment_mode(Some(&request)) else {
            panic!("an enabled setting pays a payjoin request with a payjoin");
        };
        assert_eq!(intent.endpoint, request.endpoint);

        assert_eq!(enabled.payment_mode(None), UnsignedPaymentMode::Standard);
        assert_eq!(disabled.payment_mode(Some(&request)), UnsignedPaymentMode::Standard);
        assert_eq!(
            PayjoinSetting::Unavailable.payment_mode(Some(&request)),
            UnsignedPaymentMode::Standard
        );
    }
}
