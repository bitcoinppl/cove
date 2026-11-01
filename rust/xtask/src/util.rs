use base64::{engine::general_purpose::STANDARD, Engine};
#[allow(deprecated)]
use bdk_wallet::{SignOptions, Wallet};
use bip39::Mnemonic;
use bitcoin::{Network, Psbt};
use color_eyre::{eyre::eyre, Result};
use colored::Colorize;
use gif::{Encoder as GifEncoder, Frame, Repeat};
use image::{GrayImage, Luma};
use qrcode::{render::unicode, QrCode};

/// Output format for signed PSBT
#[derive(Debug, Clone, Copy, Default)]
pub enum OutputFormat {
    /// Base64-encoded PSBT (default)
    #[default]
    Base64,
    /// Hex-encoded PSBT
    Hex,
    /// Raw binary PSBT file
    Binary,
    /// Animated GIF with BBQr-encoded QR codes
    BbqrGif,
    /// Animated GIF with UR-encoded QR codes (crypto-psbt)
    UrGif,
}

/// Sign a PSBT without finalizing it (for testing hardware wallet flows)
pub fn sign_psbt(
    mnemonic: &str,
    psbt_input: &str,
    network_str: &str,
    format: OutputFormat,
    output_path: Option<&str>,
) -> Result<()> {
    let network = parse_network(network_str)?;

    println!("{}", "=== Parsing mnemonic ===".blue().bold());
    let mnemonic = Mnemonic::parse(mnemonic).map_err(|e| eyre!("Invalid mnemonic: {}", e))?;
    println!("  Mnemonic parsed successfully");

    println!("{}", "\n=== Creating wallet ===".blue().bold());
    let wallet = bip84_signing_wallet(&mnemonic, network)?;
    println!("  Network: {:?}", network);

    println!("{}", "\n=== Decoding PSBT ===".blue().bold());

    // Handle file path or base64 string
    let psbt_bytes = if psbt_input.ends_with(".psbt") || std::path::Path::new(psbt_input).exists() {
        let file_bytes = std::fs::read(psbt_input)?;

        // Check if it's raw PSBT (starts with magic bytes) or base64 encoded
        if file_bytes.starts_with(b"psbt\xff") {
            println!("  Reading raw PSBT file");
            file_bytes
        } else {
            // Assume base64 encoded
            println!("  Reading base64-encoded PSBT file");
            let base64_str = String::from_utf8(file_bytes)
                .map_err(|e| eyre!("File is not valid UTF-8 or raw PSBT: {}", e))?;
            STANDARD
                .decode(base64_str.trim())
                .map_err(|e| eyre!("Failed to decode base64: {}", e))?
        }
    } else {
        // Treat as base64 string
        STANDARD.decode(psbt_input).map_err(|e| eyre!("Failed to decode base64: {}", e))?
    };

    let mut psbt =
        Psbt::deserialize(&psbt_bytes).map_err(|e| eyre!("Failed to parse PSBT: {}", e))?;

    println!("  Inputs: {}", psbt.inputs.len());
    for (i, input) in psbt.inputs.iter().enumerate() {
        println!(
            "  Input {}: partial_sigs={}, finalized={}",
            i,
            input.partial_sigs.len(),
            input.final_script_witness.is_some()
        );

        // Show derivation path if available
        if let Some((_, (fingerprint, path))) = input.bip32_derivation.iter().next() {
            println!("    Derivation: [{}]{}", fingerprint, path);
        }
    }

    println!("{}", "\n=== Signing (without finalizing) ===".blue().bold());

    let signed = sign_without_finalizing(&wallet, &mut psbt)?;

    if signed {
        println!("  {}", "Signed successfully".green());
    } else {
        println!("  {}", "Warning: No signatures added (key may not match)".yellow());
    }

    println!("{}", "\n=== PSBT after signing ===".blue().bold());
    for (i, input) in psbt.inputs.iter().enumerate() {
        println!(
            "  Input {}: partial_sigs={}, finalized={}",
            i,
            input.partial_sigs.len(),
            input.final_script_witness.is_some()
        );
    }

    // Serialize signed PSBT
    let signed_psbt_bytes = psbt.serialize();

    // Output based on format
    match format {
        OutputFormat::Base64 => {
            let signed_psbt_base64 = STANDARD.encode(&signed_psbt_bytes);
            println!("{}", "\n=== Signed PSBT (base64) ===".blue().bold());
            println!("{}", signed_psbt_base64);

            // Try to show QR code in terminal
            print_terminal_qr(&signed_psbt_base64);

            if let Some(path) = output_path {
                std::fs::write(path, &signed_psbt_base64)?;
                println!("\nSigned PSBT (base64) saved to: {}", path);
            }
        }

        OutputFormat::Hex => {
            let signed_psbt_hex = hex::encode(&signed_psbt_bytes);
            println!("{}", "\n=== Signed PSBT (hex) ===".blue().bold());
            println!("{}", signed_psbt_hex);

            if let Some(path) = output_path {
                std::fs::write(path, &signed_psbt_hex)?;
                println!("\nSigned PSBT (hex) saved to: {}", path);
            }
        }

        OutputFormat::Binary => {
            let path =
                output_path.ok_or_else(|| eyre!("Output path required for binary format"))?;
            std::fs::write(path, &signed_psbt_bytes)?;
            println!("{}", "\n=== Signed PSBT (binary) ===".blue().bold());
            println!("Saved {} bytes to: {}", signed_psbt_bytes.len(), path);
        }

        OutputFormat::BbqrGif => {
            let path =
                output_path.ok_or_else(|| eyre!("Output path required for bbqr-gif format"))?;
            println!("{}", "\n=== Generating BBQr animated GIF ===".blue().bold());
            generate_bbqr_gif(&signed_psbt_bytes, path)?;
            println!("BBQr animated GIF saved to: {}", path);
        }

        OutputFormat::UrGif => {
            let path =
                output_path.ok_or_else(|| eyre!("Output path required for ur-gif format"))?;
            println!("{}", "\n=== Generating UR animated GIF ===".blue().bold());
            generate_ur_gif(&signed_psbt_bytes, path)?;
            println!("UR animated GIF saved to: {}", path);
        }
    }

    Ok(())
}

/// Parse a network name as the `util sign-psbt` command accepts it
pub fn parse_network(network: &str) -> Result<Network> {
    match network {
        "bitcoin" | "mainnet" => Ok(Network::Bitcoin),
        "testnet" => Ok(Network::Testnet),
        "signet" => Ok(Network::Signet),
        "regtest" => Ok(Network::Regtest),
        _ => Err(eyre!("Unknown network: {network}")),
    }
}

/// A wallet holding the BIP84 account 0 private keys of a mnemonic, the account
/// hardware wallet test fixtures export
pub fn bip84_signing_wallet(mnemonic: &Mnemonic, network: Network) -> Result<Wallet> {
    let seed = mnemonic.to_seed("");
    let xprv = bitcoin::bip32::Xpriv::new_master(network, &seed)?;
    let coin_type = if network == Network::Bitcoin { 0 } else { 1 };

    let external_desc = format!("wpkh({xprv}/84'/{coin_type}'/0'/0/*)");
    let internal_desc = format!("wpkh({xprv}/84'/{coin_type}'/0'/1/*)");

    Ok(Wallet::create(external_desc, internal_desc).network(network).create_wallet_no_persist()?)
}

/// Sign every input the wallet owns and leave finalizing to the app, as a
/// hardware wallet does. Returns whether any signature was added
pub fn sign_without_finalizing(wallet: &Wallet, psbt: &mut Psbt) -> Result<bool> {
    #[allow(deprecated)]
    let sign_options = SignOptions { try_finalize: false, ..Default::default() };
    let signatures =
        |psbt: &Psbt| psbt.inputs.iter().map(|input| input.partial_sigs.len()).sum::<usize>();
    let before = signatures(psbt);

    // `sign` reports whether it finalized, which it never does here, so count signatures instead
    wallet.sign(psbt, sign_options)?;

    Ok(signatures(psbt) > before)
}

/// Split a PSBT into BBQr QR payloads, at least `min_parts` of them
pub fn bbqr_parts(psbt_bytes: &[u8], min_parts: usize) -> Result<Vec<String>> {
    use bbqr::{
        encode::Encoding,
        file_type::FileType,
        qr::Version,
        split::{Split, SplitOptions},
    };

    let split = Split::try_from_data(
        psbt_bytes,
        FileType::Psbt,
        SplitOptions {
            encoding: Encoding::Zlib,
            min_split_number: min_parts.max(1),
            max_split_number: 100,
            min_version: Version::V01,
            max_version: Version::V15,
        },
    )
    .map_err(|e| eyre!("BBQr encoding failed: {e}"))?;

    Ok(split.parts)
}

/// Encode a PSBT as UR `crypto-psbt` parts, at least `min_parts` of them
///
/// The first parts of a fountain encoder are the plain fragments, so the
/// returned parts decode on their own without any mixed parts
pub fn ur_parts(psbt_bytes: &[u8], min_parts: usize) -> Result<Vec<String>> {
    use cove_ur::CryptoPsbt;
    use foundation_ur::Encoder;

    let crypto_psbt = CryptoPsbt::from_psbt_bytes(psbt_bytes.to_vec())
        .map_err(|e| eyre!("Invalid PSBT bytes: {e}"))?;

    let cbor = crypto_psbt.encode().map_err(|e| eyre!("CBOR encoding failed: {e}"))?;

    // QR v15 alphanumeric capacity is ~758 chars, leaving room for UR overhead
    let max_fragment_len = foundation_ur::max_fragment_len("crypto-psbt", 100, 700)
        .min(cbor.len().div_ceil(min_parts.max(1)).max(10));

    let mut ur_encoder = Encoder::new();
    ur_encoder.start("crypto-psbt", &cbor, max_fragment_len);

    let total_parts = ur_encoder.sequence_count().max(1);
    Ok((0..total_parts).map(|_| ur_encoder.next_part().to_string()).collect())
}

/// Print a QR code to the terminal if the data fits
fn print_terminal_qr(data: &str) {
    println!("{}", "\n=== QR Code ===".blue().bold());

    match QrCode::new(data.as_bytes()) {
        Ok(qr) => {
            let qr_string = qr
                .render::<unicode::Dense1x2>()
                .dark_color(unicode::Dense1x2::Light)
                .light_color(unicode::Dense1x2::Dark)
                .build();
            println!("{}", qr_string);
        }
        Err(e) => {
            println!(
                "  {} (data may be too large for single QR: {})",
                "Could not generate QR code".yellow(),
                e
            );
        }
    }
}

/// Generate an animated GIF with BBQr-encoded QR codes
fn generate_bbqr_gif(psbt_bytes: &[u8], output_path: &str) -> Result<()> {
    let parts = bbqr_parts(psbt_bytes, 1)?;
    println!("  Split into {} QR codes", parts.len());

    // Generate QR code images for each part
    let mut qr_images: Vec<GrayImage> = parts
        .iter()
        .map(|part| {
            let qr = QrCode::new(part.as_bytes())
                .map_err(|e| eyre!("Failed to generate QR code: {e}"))?;
            Ok(qr.render::<Luma<u8>>().quiet_zone(true).min_dimensions(400, 400).build())
        })
        .collect::<Result<Vec<_>>>()?;

    // Ensure all images have the same dimensions (use the first image's dimensions)
    if !qr_images.is_empty() {
        let (target_width, target_height) = qr_images[0].dimensions();
        for img in qr_images.iter_mut().skip(1) {
            if img.dimensions() != (target_width, target_height) {
                *img = image::imageops::resize(
                    img,
                    target_width,
                    target_height,
                    image::imageops::FilterType::Nearest,
                );
            }
        }
    }

    create_animated_gif(&qr_images, output_path, 300)?;

    Ok(())
}

/// Generate an animated GIF with UR-encoded QR codes (crypto-psbt)
fn generate_ur_gif(psbt_bytes: &[u8], output_path: &str) -> Result<()> {
    let parts = ur_parts(psbt_bytes, 1)?;
    println!("  Split into {} UR parts", parts.len());

    // Generate QR code images for each part
    let qr_images: Vec<_> = parts
        .iter()
        .map(|part| {
            let qr = QrCode::new(part.as_bytes())
                .map_err(|e| eyre!("Failed to generate QR code: {e}"))?;
            Ok(qr.render::<Luma<u8>>().quiet_zone(true).min_dimensions(400, 400).build())
        })
        .collect::<Result<Vec<_>>>()?;

    create_animated_gif(&qr_images, output_path, 300)?;

    Ok(())
}

/// Create an animated GIF from a sequence of grayscale images
fn create_animated_gif(images: &[GrayImage], output_path: &str, delay_ms: u16) -> Result<()> {
    if images.is_empty() {
        return Err(eyre!("No images to create GIF from"));
    }

    let width = images[0].width() as u16;
    let height = images[0].height() as u16;

    // Create a grayscale palette (256 shades from black to white)
    let mut palette = Vec::with_capacity(256 * 3);
    for i in 0..=255u8 {
        palette.push(i); // R
        palette.push(i); // G
        palette.push(i); // B
    }

    let file = std::fs::File::create(output_path)?;
    let mut encoder = GifEncoder::new(file, width, height, &palette)?;
    encoder.set_repeat(Repeat::Infinite)?;

    // GIF delay is in centiseconds (1/100th of a second)
    let delay_centisec = delay_ms / 10;

    for image in images {
        // Grayscale pixels are already indexed (0-255 maps to palette)
        let pixels: Vec<u8> = image.as_raw().clone();

        // Create frame with the raw pixels (uses global palette)
        let mut frame = Frame::from_indexed_pixels(width, height, pixels, None);
        frame.delay = delay_centisec;

        encoder.write_frame(&frame)?;
    }

    Ok(())
}
