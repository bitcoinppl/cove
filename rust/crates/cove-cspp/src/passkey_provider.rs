use crate::backup_data::{PasskeyProviderHint, PasskeyRegistrationPlatform};

const ZERO_AAGUID: &str = "00000000-0000-0000-0000-000000000000";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KnownPasskeyProvider {
    Keeper,
    ProtonPass,
    Dashlane,
    SamsungPass,
    Passwall,
    KasperskyPasswordManager,
    ChromeOnMac,
    ZohoVault,
    LastPass,
    NordPass,
    OnePassword,
    IPasswords,
    MicrosoftPasswordManager,
    Bitwarden,
    StickyPasswordManager,
    IcloudKeychainManaged,
    GooglePasswordManager,
    KeePassDx,
    ApplePasswords,
    KeePassXc,
    Enpass,
}

impl KnownPasskeyProvider {
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Keeper => "Keeper",
            Self::ProtonPass => "Proton Pass",
            Self::Dashlane => "Dashlane",
            Self::SamsungPass => "Samsung Pass",
            Self::Passwall => "Passwall",
            Self::KasperskyPasswordManager => "Kaspersky Password Manager",
            Self::ChromeOnMac => "Chrome on Mac",
            Self::ZohoVault => "Zoho Vault",
            Self::LastPass => "LastPass",
            Self::NordPass => "NordPass",
            Self::OnePassword => "1Password",
            Self::IPasswords => "iPasswords",
            Self::MicrosoftPasswordManager => "Microsoft Password Manager",
            Self::Bitwarden => "Bitwarden",
            Self::StickyPasswordManager => "Sticky Password Manager",
            Self::IcloudKeychainManaged => "iCloud Keychain (Managed)",
            Self::GooglePasswordManager => "Google Password Manager",
            Self::KeePassDx => "KeePassDX",
            Self::ApplePasswords => "Apple Passwords",
            Self::KeePassXc => "KeePassXC",
            Self::Enpass => "Enpass",
        }
    }
}

struct KnownPasskeyProviderEntry {
    aaguid: &'static str,
    provider: KnownPasskeyProvider,
}

macro_rules! known_passkey_providers {
    ($($aaguid:literal => $provider:ident),+ $(,)?) => {
        &[
            $(
                KnownPasskeyProviderEntry {
                    aaguid: $aaguid,
                    provider: KnownPasskeyProvider::$provider,
                },
            )+
        ]
    };
}

// sourced from https://github.com/passkeydeveloper/passkey-authenticator-aaguids
// for provider display names
const KNOWN_PASSKEY_PROVIDERS: &[KnownPasskeyProviderEntry] = known_passkey_providers! {
    "0ea242b4-43c4-4a1b-8b17-dd6d0b6baec6" => Keeper,
    "50726f74-6f6e-5061-7373-50726f746f6e" => ProtonPass,
    "531126d6-e717-415c-9320-3d9aa6981239" => Dashlane,
    "53414d53-554e-4700-0000-000000000000" => SamsungPass,
    "70617373-7761-6c6c-6669-646f32303236" => Passwall,
    "a10c6dd9-465e-4226-8198-c7c44b91c555" => KasperskyPasswordManager,
    "adce0002-35bc-c60a-648b-0b25f1f05503" => ChromeOnMac,
    "b35a26b2-8f6e-4697-ab1d-d44db4da28c6" => ZohoVault,
    "b78a0a55-6ef8-d246-a042-ba0f6d55050c" => LastPass,
    "b84e4048-15dc-4dd0-8640-f4f60813c8af" => NordPass,
    "bada5566-a7aa-401f-bd96-45619a55120d" => OnePassword,
    "bfc748bb-3429-4faa-b9f9-7cfa9f3b76d0" => IPasswords,
    "d3452668-01fd-4c12-926c-83a4204853aa" => MicrosoftPasswordManager,
    "d548826e-79b4-db40-a3d8-11116f7e8349" => Bitwarden,
    "d9be9d39-e6a6-4c28-a581-32b044d986e4" => StickyPasswordManager,
    "dd4ec289-e01d-41c9-bb89-70fa845d4bf2" => IcloudKeychainManaged,
    "ea9b8d66-4d01-1d21-3ce4-b6b48cb575d4" => GooglePasswordManager,
    "eaecdef2-1c31-5634-8639-f1cbd9c00a08" => KeePassDx,
    "fbfc3007-154e-4ecc-8c0b-6e020557d7bd" => ApplePasswords,
    "fdb141b2-5d84-443e-8a35-4698c205a502" => KeePassXc,
    "f3809540-7f14-49c1-a8b3-8f813b225541" => Enpass,
};

impl PasskeyProviderHint {
    pub fn known_provider(&self) -> Option<KnownPasskeyProvider> {
        if let Some(entry) =
            KNOWN_PASSKEY_PROVIDERS.iter().find(|entry| entry.aaguid == self.aaguid)
        {
            return Some(entry.provider);
        }

        // zero AAGUID is an iOS fallback used when Apple does not provide attestation data
        if self.aaguid == ZERO_AAGUID
            && self.registered_platform == PasskeyRegistrationPlatform::Ios
        {
            return Some(KnownPasskeyProvider::ApplePasswords);
        }

        None
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_google_provider() {
        let hint = PasskeyProviderHint {
            aaguid: "ea9b8d66-4d01-1d21-3ce4-b6b48cb575d4".into(),
            registered_platform: PasskeyRegistrationPlatform::Android,
            registered_at: 1_777_661_234,
        };

        assert_eq!(hint.known_provider(), Some(KnownPasskeyProvider::GooglePasswordManager));
    }

    #[test]
    fn resolves_zero_aaguid_for_ios_only() {
        let ios_hint = PasskeyProviderHint {
            aaguid: ZERO_AAGUID.into(),
            registered_platform: PasskeyRegistrationPlatform::Ios,
            registered_at: 1_777_661_234,
        };
        let android_hint = PasskeyProviderHint {
            aaguid: ZERO_AAGUID.into(),
            registered_platform: PasskeyRegistrationPlatform::Android,
            registered_at: 1_777_661_236,
        };

        assert_eq!(ios_hint.known_provider(), Some(KnownPasskeyProvider::ApplePasswords));
        assert_eq!(android_hint.known_provider(), None);
    }

    #[test]
    fn leaves_unknown_provider_unresolved() {
        let hint = PasskeyProviderHint {
            aaguid: "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee".into(),
            registered_platform: PasskeyRegistrationPlatform::Android,
            registered_at: 1_777_661_236,
        };

        assert_eq!(hint.known_provider(), None);
    }
}
