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
    let network = match network_str {
        "bitcoin" | "mainnet" => Network::Bitcoin,
        "testnet" => Network::Testnet,
        "signet" => Network::Signet,
        "regtest" => Network::Regtest,
        _ => return Err(eyre!("Unknown network: {}", network_str)),
    };

    println!("{}", "=== Parsing mnemonic ===".blue().bold());
    let mnemonic = Mnemonic::parse(mnemonic).map_err(|e| eyre!("Invalid mnemonic: {}", e))?;
    println!("  Mnemonic parsed successfully");

    println!("{}", "\n=== Creating wallet ===".blue().bold());
    let seed = mnemonic.to_seed("");
    let xprv = bitcoin::bip32::Xpriv::new_master(network, &seed)?;

    // Use coin type based on network
    let coin_type = if network == Network::Bitcoin { 0 } else { 1 };

    let external_desc = format!("wpkh({}/84'/{}'/0'/0/*)", xprv, coin_type);
    let internal_desc = format!("wpkh({}/84'/{}'/0'/1/*)", xprv, coin_type);

    let wallet = Wallet::create(external_desc.clone(), internal_desc.clone())
        .network(network)
        .create_wallet_no_persist()?;

    println!("  Network: {:?}", network);
    println!("  External descriptor: {}...", &external_desc[..50.min(external_desc.len())]);

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

    #[allow(deprecated)]
    let sign_options = SignOptions { try_finalize: false, ..Default::default() };

    let signed = wallet.sign(&mut psbt, sign_options)?;

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
    use bbqr::{
        encode::Encoding,
        file_type::FileType,
        qr::Version,
        split::{Split, SplitOptions},
    };

    // Split the PSBT data into BBQr parts
    let split = Split::try_from_data(
        psbt_bytes,
        FileType::Psbt,
        SplitOptions {
            encoding: Encoding::Zlib,
            min_split_number: 1,
            max_split_number: 100,
            min_version: Version::V01,
            max_version: Version::V15,
        },
    )
    .map_err(|e| eyre!("BBQr encoding failed: {e}"))?;

    println!("  Split into {} QR codes", split.parts.len());

    // Generate QR code images for each part
    let qr_images: Vec<_> = split
        .parts
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

/// Generate an animated GIF with UR-encoded QR codes (crypto-psbt)
fn generate_ur_gif(psbt_bytes: &[u8], output_path: &str) -> Result<()> {
    use foundation_ur::Encoder;
    use minicbor::data::Tag;

    // Encode PSBT as crypto-psbt CBOR (tag 310 + bytes)
    let mut cbor = Vec::new();
    let mut encoder = minicbor::Encoder::new(&mut cbor);
    encoder.tag(Tag::new(310)).map_err(|e| eyre!("CBOR tag error: {e}"))?;
    encoder.bytes(psbt_bytes).map_err(|e| eyre!("CBOR encode error: {e}"))?;

    // Calculate max fragment length for QR code version 15
    // QR v15 alphanumeric capacity is ~758 chars, leaving room for UR overhead
    let max_fragment_len = foundation_ur::max_fragment_len("crypto-psbt", 100, 700);

    // Create UR encoder for multi-part encoding
    let mut ur_encoder = Encoder::new();
    ur_encoder.start("crypto-psbt", &cbor, max_fragment_len);

    // Determine how many parts we need (at minimum, the sequence count)
    let total_parts = ur_encoder.sequence_count().max(1);
    println!("  Split into {} UR parts", total_parts);

    // Generate all the UR parts
    let mut ur_parts = Vec::new();
    for _ in 0..total_parts {
        let part = ur_encoder.next_part();
        ur_parts.push(part.to_string());
    }

    // Generate QR code images for each part
    let qr_images: Vec<_> = ur_parts
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

    let file = std::fs::File::create(output_path)?;
    let mut encoder = GifEncoder::new(file, width, height, &[])?;
    encoder.set_repeat(Repeat::Infinite)?;

    // GIF delay is in centiseconds (1/100th of a second)
    let delay_centisec = delay_ms / 10;

    for image in images {
        // Convert grayscale to indexed color (gif requires indexed)
        let pixels: Vec<u8> = image.as_raw().clone();

        // Create frame with the raw pixels
        let mut frame = Frame::from_indexed_pixels(width, height, pixels, None);
        frame.delay = delay_centisec;

        encoder.write_frame(&frame)?;
    }

    Ok(())
}
