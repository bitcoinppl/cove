use std::sync::Arc;

use act_zero::call;
use cove_util::result_ext::ResultExt as _;

use crate::{
    database::Database,
    keychain::Keychain,
    label_manager::LabelManagerError,
    loading_popup::with_loading_popup,
    reporting::HistoricalFiatPriceReport,
    wallet::{Wallet, metadata::WalletId},
};

use cove_types::confirm::QrDensity;

use super::{
    Error, LabelExportResult, RustWalletManager, TransactionExportResult, XpubExportResult,
};

#[uniffi::export(async_runtime = "tokio")]
impl RustWalletManager {
    #[uniffi::method]
    pub async fn create_transactions_with_fiat_export(&self) -> Result<String, Error> {
        let fiat_currency = Database::global().global_config.fiat_currency().unwrap_or_default();

        let txns_with_prices = call!(self.actor.txns_with_prices()).await.unwrap().unwrap();

        let report = HistoricalFiatPriceReport::new(fiat_currency, txns_with_prices);
        let csv = report.create_csv().map_err_str(Error::CsvCreationError)?;

        Ok(csv.into_string())
    }

    /// Export labels for share with conditional loading popup
    #[uniffi::method]
    pub async fn export_labels_for_share(&self) -> Result<LabelExportResult, LabelManagerError> {
        let lm = self.label_manager.clone();
        let name = self.metadata.read().name.clone();

        with_loading_popup(async move {
            let content = lm.export().await?;
            let filename = format!("{}.jsonl", lm.export_default_file_name(name));
            Ok(LabelExportResult { content, filename })
        })
        .await
    }

    /// Export labels as QR codes with conditional loading popup
    #[uniffi::method]
    pub async fn export_labels_for_qr(
        &self,
        density: Arc<QrDensity>,
    ) -> Result<Vec<String>, LabelManagerError> {
        let lm = self.label_manager.clone();

        with_loading_popup(async move { lm.export_to_bbqr_with_density(&density).await }).await
    }

    /// Export public descriptors (xpub) for share
    #[uniffi::method]
    pub async fn export_xpub_for_share(&self) -> Result<XpubExportResult, Error> {
        let id = self.id.clone();
        let name = self.metadata.read().name.clone();

        with_loading_popup(async move {
            let content = get_public_descriptor_content(&id)?;

            let sanitized_name = name
                .replace(' ', "_")
                .replace(|c: char| !c.is_alphanumeric() && c != '_', "")
                .to_ascii_lowercase();

            let sanitized_name =
                if sanitized_name.is_empty() { "wallet".to_string() } else { sanitized_name };

            let filename = format!("{sanitized_name}_descriptors.txt");

            Ok(XpubExportResult { content, filename })
        })
        .await
    }

    /// Export public descriptors (xpub) as QR codes
    #[uniffi::method]
    pub async fn export_xpub_for_qr(&self, density: Arc<QrDensity>) -> Result<Vec<String>, Error> {
        use bbqr::{
            encode::Encoding,
            file_type::FileType,
            qr::Version,
            split::{Split, SplitOptions},
        };

        let id = self.id.clone();

        with_loading_popup(async move {
            let content = get_public_descriptor_content(&id)?;
            let max_version = density.bbqr_max_version();

            cove_tokio::task::spawn_blocking(move || {
                let data = content.as_bytes();
                let version = Version::try_from(max_version).unwrap_or(Version::V15);

                let split = Split::try_from_data(
                    data,
                    FileType::UnicodeText,
                    SplitOptions {
                        encoding: Encoding::Zlib,
                        min_split_number: 1,
                        max_split_number: 100,
                        min_version: Version::V01,
                        max_version: version,
                    },
                )
                .map_err_prefix("BBQr encoding failed", Error::UnknownError)?;

                Ok(split.parts)
            })
            .await
            .map_err_str(Error::UnknownError)?
        })
        .await
    }

    /// Export transactions as CSV with conditional loading popup
    #[uniffi::method]
    pub async fn export_transactions_csv(&self) -> Result<TransactionExportResult, Error> {
        let name = self.metadata.read().name.clone();
        let actor = self.actor.clone();

        with_loading_popup(async move {
            let txns_with_prices = call!(actor.txns_with_prices())
                .await
                .map_err_str(Error::TransactionsRetrievalError)?
                .map_err_str(Error::GetHistoricalPricesError)?;

            cove_tokio::task::spawn_blocking(move || {
                let fiat_currency =
                    Database::global().global_config.fiat_currency().unwrap_or_default();
                let report = HistoricalFiatPriceReport::new(fiat_currency, txns_with_prices);
                let csv = report.create_csv().map_err_str(Error::CsvCreationError)?;

                let sanitized_name = name
                    .replace(' ', "_")
                    .replace(|c: char| !c.is_alphanumeric() && c != '_', "")
                    .to_ascii_lowercase();

                let sanitized_name =
                    if sanitized_name.is_empty() { "wallet".to_string() } else { sanitized_name };

                let filename = format!("{sanitized_name}_transactions.csv");
                Ok(TransactionExportResult { content: csv.into_string(), filename })
            })
            .await
            .map_err_str(Error::CsvCreationError)?
        })
        .await
    }
}

/// Get the public descriptor content for export
///
/// Tries single-line BIP-389 multipath `<0;1>` format first,
/// falls back to two normalized descriptors on separate lines
fn get_public_descriptor_content(id: &WalletId) -> Result<String, Error> {
    use cove_bdk::descriptor_ext::DescriptorExt;

    match Wallet::try_load_persisted(id.clone()) {
        Ok(wallet) => {
            let external = wallet.bdk.public_descriptor(bdk_wallet::KeychainKind::External);
            let internal = wallet.bdk.public_descriptor(bdk_wallet::KeychainKind::Internal);

            Ok(DescriptorExt::to_export_string(external, internal))
        }
        Err(load_error) => {
            if let Ok(Some((external, internal))) = Keychain::global().get_public_descriptor(id) {
                return Ok(DescriptorExt::to_export_string(&external, &internal));
            }

            Err(Error::UnknownError(format!("failed to load wallet: {load_error}")))
        }
    }
}
