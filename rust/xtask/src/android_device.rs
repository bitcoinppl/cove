use crate::common::{command_exists, print_info};
use color_eyre::{
    eyre::{eyre, Context, ContextCompat},
    Result,
};
use std::{
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct AndroidDevice {
    serial: String,
    details: String,
}

impl AndroidDevice {
    pub(crate) fn serial(&self) -> &str {
        &self.serial
    }

    pub(crate) fn description(&self) -> String {
        if self.details.is_empty() {
            self.serial.clone()
        } else {
            format!("{} ({})", self.serial, self.details)
        }
    }

    /// Select a connected Android device.
    ///
    /// `device` accepts:
    /// - `main` (default) — physical phone via `ANDROID_DEVICE_MAIN`, else first non-emulator
    /// - `sim` — emulator via `ANDROID_DEVICE_SIM`, else first `emulator-*`
    /// - an exact adb serial
    pub(crate) fn select(device: Option<&str>) -> Result<Self> {
        let devices = list_connected()?;
        let selector = AndroidDeviceSelector::from_arg(device)?;
        selector.resolve(&devices)
    }

    pub(crate) fn select_connected() -> Result<Self> {
        Self::select(None)
    }

    pub(crate) fn ensure_ready(&self) -> Result<()> {
        let output =
            self.adb_command().arg("get-state").output().wrap_err("Failed to run adb get-state")?;

        if !output.status.success() {
            color_eyre::eyre::bail!(
                "Android device {} is not ready: {}",
                self.serial,
                command_error(&output)
            );
        }

        let state = adb_stdout(&output).trim().to_string();
        if state != "device" {
            color_eyre::eyre::bail!("Android device {} is not ready: {state}", self.serial);
        }

        Ok(())
    }

    pub(crate) fn remote_dir_exists(&self, remote_dir: &str) -> Result<bool> {
        let command = format!("[ -d {} ]", remote_shell_quote(remote_dir));
        let status = self
            .adb_command()
            .args(["shell", &command])
            .status()
            .wrap_err_with(|| format!("Failed to check Android directory {remote_dir}"))?;

        Ok(status.success())
    }

    pub(crate) fn list_screenshot_files(&self, remote_dir: &str) -> Result<Vec<String>> {
        let command = format!(
            "find {} -maxdepth 1 -type f \\( -iname '*.png' -o -iname '*.jpg' -o -iname '*.jpeg' -o -iname '*.webp' \\) -print",
            remote_shell_quote(remote_dir)
        );
        let output =
            self.shell_output(&command, &format!("Failed to list screenshots in {remote_dir}"))?;

        Ok(output.lines().filter(|line| !line.is_empty()).map(ToOwned::to_owned).collect())
    }

    pub(crate) fn pull_file(&self, remote_path: &str, target_path: &Path) -> Result<()> {
        let status = self
            .adb_command()
            .args(["pull", remote_path])
            .arg(target_path)
            .status()
            .wrap_err_with(|| format!("Failed to pull Android screenshot {remote_path}"))?;

        if !status.success() {
            color_eyre::eyre::bail!("Failed to pull Android screenshot {remote_path}: {status}");
        }

        Ok(())
    }

    pub(crate) fn delete_file(&self, remote_path: &str) -> Result<()> {
        let command = format!("rm -f {}", remote_shell_quote(remote_path));
        let status = self
            .adb_command()
            .args(["shell", &command])
            .status()
            .wrap_err_with(|| format!("Failed to delete Android screenshot {remote_path}"))?;

        if !status.success() {
            color_eyre::eyre::bail!("Failed to delete Android screenshot {remote_path}: {status}");
        }

        Ok(())
    }

    fn adb_command(&self) -> Command {
        let mut command = Command::new("adb");
        command.args(["-s", &self.serial]);
        command
    }

    fn selection_row(&self) -> String {
        if self.details.is_empty() {
            return self.serial.clone();
        }

        format!("{}\t{}", self.serial, self.details)
    }

    fn shell_output(&self, command: &str, context: &str) -> Result<String> {
        let output = self
            .adb_command()
            .args(["shell", command])
            .output()
            .wrap_err_with(|| context.to_string())?;

        if !output.status.success() {
            color_eyre::eyre::bail!("{context}: {}", command_error(&output));
        }

        Ok(adb_stdout(&output))
    }

    fn is_emulator(&self) -> bool {
        serial_is_emulator(&self.serial)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum AndroidDeviceSelector {
    Main,
    Sim,
    Serial(String),
}

impl AndroidDeviceSelector {
    fn from_arg(device: Option<&str>) -> Result<Self> {
        let Some(device) = device.map(str::trim).filter(|value| !value.is_empty()) else {
            print_info("No Android device specified; defaulting to alias 'main'");
            return Ok(Self::Main);
        };

        match device.to_ascii_lowercase().as_str() {
            "main" => Ok(Self::Main),
            "sim" => Ok(Self::Sim),
            _ => Ok(Self::Serial(device.to_string())),
        }
    }

    fn resolve(&self, devices: &[AndroidDevice]) -> Result<AndroidDevice> {
        if devices.is_empty() {
            color_eyre::eyre::bail!("No connected Android device found");
        }

        match self {
            Self::Main => resolve_main(devices),
            Self::Sim => resolve_sim(devices),
            Self::Serial(serial) => resolve_serial(devices, serial),
        }
    }
}

fn resolve_main(devices: &[AndroidDevice]) -> Result<AndroidDevice> {
    if let Some(serial) = env_device_serial("ANDROID_DEVICE_MAIN") {
        print_info(&format!("Resolved Android alias 'main' from ANDROID_DEVICE_MAIN={serial}"));
        return resolve_serial(devices, &serial);
    }

    let physical =
        devices.iter().filter(|device| !device.is_emulator()).cloned().collect::<Vec<_>>();

    match physical.as_slice() {
        [] => {
            print_info(
                "Android alias 'main' has no physical device; using first available Android device",
            );
            select_from_devices(devices)
        }
        [device] => {
            print_info(&format!("Using Android main device {}", device.description()));
            Ok(device.clone())
        }
        _ => {
            print_info("Multiple physical Android devices connected; choose main device");
            select_from_devices(&physical)
        }
    }
}

fn resolve_sim(devices: &[AndroidDevice]) -> Result<AndroidDevice> {
    if let Some(serial) = env_device_serial("ANDROID_DEVICE_SIM") {
        print_info(&format!("Resolved Android alias 'sim' from ANDROID_DEVICE_SIM={serial}"));
        return resolve_serial(devices, &serial);
    }

    let emulators =
        devices.iter().filter(|device| device.is_emulator()).cloned().collect::<Vec<_>>();

    match emulators.as_slice() {
        [] => Err(eyre!(
            "Android alias 'sim' found no emulator. Start an AVD, or set ANDROID_DEVICE_SIM to an adb serial (list devices with: adb devices -l)"
        )),
        [device] => {
            print_info(&format!("Using Android sim device {}", device.description()));
            Ok(device.clone())
        }
        _ => {
            print_info("Multiple Android emulators connected; choose sim device");
            select_from_devices(&emulators)
        }
    }
}

fn resolve_serial(devices: &[AndroidDevice], serial: &str) -> Result<AndroidDevice> {
    devices.iter().find(|device| device.serial == serial).cloned().ok_or_else(|| {
        eyre!(
            "No connected Android device with serial '{serial}'. {}",
            available_android_device_context(devices)
        )
    })
}

fn select_from_devices(devices: &[AndroidDevice]) -> Result<AndroidDevice> {
    match devices {
        [] => color_eyre::eyre::bail!("No connected Android device found"),
        [device] => Ok(device.clone()),
        _ => select_with_fzf(devices),
    }
}

fn list_connected() -> Result<Vec<AndroidDevice>> {
    let output = Command::new("adb")
        .args(["devices", "-l"])
        .output()
        .wrap_err("Failed to list connected Android devices")?;

    if !output.status.success() {
        color_eyre::eyre::bail!(
            "Failed to list connected Android devices: {}",
            command_error(&output)
        );
    }

    Ok(parse_connected(&adb_stdout(&output)))
}

fn env_device_serial(key: &str) -> Option<String> {
    std::env::var(key).ok().and_then(|value| {
        let value = value.trim();
        (!value.is_empty()).then(|| value.to_string())
    })
}

fn serial_is_emulator(serial: &str) -> bool {
    serial.starts_with("emulator-")
}

fn available_android_device_context(devices: &[AndroidDevice]) -> String {
    if devices.is_empty() {
        return "No available Android devices found".to_string();
    }

    let devices = devices.iter().map(AndroidDevice::description).collect::<Vec<_>>().join(", ");

    format!("Available Android devices: {devices}")
}

fn parse_connected(output: &str) -> Vec<AndroidDevice> {
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let serial = fields.next()?;
            let state = fields.next()?;
            if state != "device" {
                return None;
            }

            Some(AndroidDevice {
                serial: serial.to_string(),
                details: fields.collect::<Vec<_>>().join(" "),
            })
        })
        .collect()
}

fn select_with_fzf(devices: &[AndroidDevice]) -> Result<AndroidDevice> {
    if !command_exists("fzf") {
        color_eyre::eyre::bail!("fzf is required to choose between connected Android devices");
    }

    let rows = devices.iter().map(AndroidDevice::selection_row).collect::<Vec<_>>();
    let mut fzf = Command::new("fzf")
        .args([r"--delimiter=\t", "--with-nth=1,2", "--prompt=Select Android device: "])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .wrap_err("Failed to start fzf")?;

    {
        let mut stdin = fzf.stdin.take().wrap_err("Failed to open fzf stdin")?;
        stdin
            .write_all(format!("{}\n", rows.join("\n")).as_bytes())
            .wrap_err("Failed to send Android devices to fzf")?;
    }

    let output = fzf.wait_with_output().wrap_err("Failed to read fzf selection")?;
    if !output.status.success() {
        color_eyre::eyre::bail!("No Android device selected");
    }

    let selection =
        String::from_utf8(output.stdout).wrap_err("Selected Android device was not valid UTF-8")?;
    let selection = selection.trim_end();

    devices
        .iter()
        .zip(rows)
        .find_map(|(device, row)| (row == selection).then(|| device.clone()))
        .context("Selected Android device was not recognized")
}

pub(crate) fn adb_stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).replace('\r', "")
}

pub(crate) fn command_error(output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).replace('\r', "");
    let stderr = stderr.trim();
    if !stderr.is_empty() {
        return stderr.to_string();
    }

    let stdout = adb_stdout(output);
    let stdout = stdout.trim();
    if !stdout.is_empty() {
        return stdout.to_string();
    }

    output.status.to_string()
}

fn remote_shell_quote(value: &str) -> String {
    let mut quoted = String::from("'");

    for character in value.chars() {
        if character == '\'' {
            quoted.push_str("'\\''");
        } else {
            quoted.push(character);
        }
    }

    quoted.push('\'');
    quoted
}

#[cfg(test)]
mod tests {
    use super::{
        parse_connected, remote_shell_quote, serial_is_emulator, AndroidDevice,
        AndroidDeviceSelector,
    };

    #[test]
    fn parses_only_ready_connected_android_devices() {
        let output = "List of devices attached\n\
                      emulator-5554 device product:sdk_phone model:Pixel_9 transport_id:1\n\
                      R5CX unauthorized usb:1-1 transport_id:2\n\
                      192.0.2.1:5555 offline transport_id:3\n\
                      ABC123 device usb:1-2 model:Pixel_8 transport_id:4\n";

        let devices = parse_connected(output);

        assert_eq!(
            devices,
            vec![
                AndroidDevice {
                    serial: "emulator-5554".to_string(),
                    details: "product:sdk_phone model:Pixel_9 transport_id:1".to_string(),
                },
                AndroidDevice {
                    serial: "ABC123".to_string(),
                    details: "usb:1-2 model:Pixel_8 transport_id:4".to_string(),
                },
            ]
        );
    }

    #[test]
    fn formats_android_device_selection_rows() {
        let device = AndroidDevice {
            serial: "emulator-5554".to_string(),
            details: "model:Pixel_9 transport_id:1".to_string(),
        };

        assert_eq!(device.selection_row(), "emulator-5554\tmodel:Pixel_9 transport_id:1");
    }

    #[test]
    fn quotes_android_shell_paths() {
        assert_eq!(
            remote_shell_quote("/sdcard/Pictures/Screenshots"),
            "'/sdcard/Pictures/Screenshots'"
        );
        assert_eq!(
            remote_shell_quote("/sdcard/Pictures/Screenshots/it's.png"),
            "'/sdcard/Pictures/Screenshots/it'\\''s.png'"
        );
    }

    #[test]
    fn serial_is_emulator_detects_avd_serials() {
        assert!(serial_is_emulator("emulator-5554"));
        assert!(!serial_is_emulator("RFCN4002AKK"));
        assert!(!serial_is_emulator("192.0.2.1:5555"));
    }

    #[test]
    fn android_device_selector_parses_aliases_and_serials() {
        assert_eq!(
            AndroidDeviceSelector::from_arg(None).expect("default"),
            AndroidDeviceSelector::Main
        );
        assert_eq!(
            AndroidDeviceSelector::from_arg(Some("main")).expect("main"),
            AndroidDeviceSelector::Main
        );
        assert_eq!(
            AndroidDeviceSelector::from_arg(Some("SIM")).expect("sim"),
            AndroidDeviceSelector::Sim
        );
        assert_eq!(
            AndroidDeviceSelector::from_arg(Some("emulator-5554")).expect("serial"),
            AndroidDeviceSelector::Serial("emulator-5554".to_string())
        );
    }

    #[test]
    fn resolve_prefers_physical_for_main_and_emulator_for_sim() {
        let devices = vec![
            AndroidDevice {
                serial: "emulator-5554".to_string(),
                details: "model:Pixel_9".to_string(),
            },
            AndroidDevice {
                serial: "RFCN4002AKK".to_string(),
                details: "model:SM_G981W".to_string(),
            },
        ];

        let main = AndroidDeviceSelector::Main.resolve(&devices).expect("main");
        assert_eq!(main.serial, "RFCN4002AKK");

        let sim = AndroidDeviceSelector::Sim.resolve(&devices).expect("sim");
        assert_eq!(sim.serial, "emulator-5554");
    }

    #[test]
    fn resolve_sim_errors_without_emulator() {
        let devices = vec![AndroidDevice {
            serial: "RFCN4002AKK".to_string(),
            details: "model:SM_G981W".to_string(),
        }];

        let err = AndroidDeviceSelector::Sim.resolve(&devices).expect_err("sim needs emulator");
        assert!(err.to_string().contains("ANDROID_DEVICE_SIM"));
    }
}
