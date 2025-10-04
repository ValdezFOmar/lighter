use clap::{Args, Parser, Subcommand};
use serde::Deserialize;
use serde::Serialize;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::percent::Percent;

mod percent {
    use core::ops::{Add, Sub};

    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct Percent(f32);

    impl Percent {
        pub fn new(p: f32) -> Option<Self> {
            if p.is_finite() && (0.0..=100.0).contains(&p) {
                Some(Self(p))
            } else {
                None
            }
        }

        pub fn get(self) -> f32 {
            self.0
        }
    }

    impl Add for Percent {
        type Output = Self;

        fn add(self, rhs: Self) -> Self::Output {
            Self::new((self.0 + rhs.0).min(100.0)).expect("percent to not be greater than 100")
        }
    }

    impl Sub for Percent {
        type Output = Self;

        fn sub(self, rhs: Self) -> Self::Output {
            Self::new((self.0 - rhs.0).max(0.0)).expect("percent to not be less than 0")
        }
    }

    pub fn clap_parser(s: &str) -> Result<Percent, String> {
        let percent: f32 = s
            .parse()
            .map_err(|_| format!("`{s}` is not a percentage"))?;
        match Percent::new(percent) {
            Some(p) => Ok(p),
            None => Err(format!("`{s}` is not a percentage between 0 and 100")),
        }
    }

    #[test]
    fn test_percent() {
        assert_eq!(Percent::new(0.0), Some(Percent(0.0)));
        assert_eq!(Percent::new(15.0), Some(Percent(15.0)));
        assert_eq!(Percent::new(100.0), Some(Percent(100.0)));
        assert_eq!(Percent::new(-1.0), None);
        assert_eq!(Percent::new(101.0), None);
        assert_eq!(Percent::new(f32::MIN), None);
        assert_eq!(Percent::new(f32::MAX), None);
        assert_eq!(Percent::new(f32::NAN), None);
        assert_eq!(Percent::new(f32::INFINITY), None);
        assert_eq!(Percent::new(f32::NEG_INFINITY), None);
    }
}

const PREFIX: &str = "/sys/class/backlight";
const DATA_FILE_NAME: &str = "device-data.json";

type Brightness = u16;

#[derive(Debug, Clone)]
struct Device {
    /// Device name, derived from its path.
    name: String,
    /// Full path to the device, including its name.
    path: PathBuf,
    brightness: Brightness,
    max_brightness: Brightness,
}

impl Device {
    fn set_brightness(&mut self, value: Brightness) -> io::Result<()> {
        let path = self.path.join("brightness");
        let brightness = value.min(self.max_brightness);
        fs::write(path, brightness.to_string())?;
        self.brightness = brightness;
        Ok(())
    }

    fn from_path(prefix: impl AsRef<Path>) -> io::Result<Self> {
        fn parse_brightness(path: &Path) -> io::Result<Brightness> {
            fs::read_to_string(path)?
                .trim()
                .parse()
                .map_err(io::Error::other)
        }

        fn inner(prefix: &Path) -> io::Result<Device> {
            let name = prefix
                .file_name()
                .ok_or_else(|| io::Error::other(format!("{prefix:#?} has no file name")))?
                .to_string_lossy()
                .to_string();

            let brightness = parse_brightness(&prefix.join("brightness"))?;
            let max_brightness = parse_brightness(&prefix.join("max_brightness"))?;

            assert!(
                brightness <= max_brightness,
                "brightness = {brightness} > max_brightness = {max_brightness}"
            );

            Ok(Device {
                name,
                brightness,
                max_brightness,
                path: PathBuf::from(prefix),
            })
        }

        inner(prefix.as_ref())
    }

    // Returns the first encountered device under the given `prefix`.
    // Which device is "first" is determined by alphabetical order.
    fn get(prefix: &str) -> io::Result<Self> {
        let read_dir = fs::read_dir(prefix)?;

        let mut paths = read_dir
            .filter_map(|entry| entry.inspect_err(|err| eprintln!("{err}")).ok())
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>();
        paths.sort();

        let path = paths
            .first()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no devices found"))?;

        Self::from_path(path)
    }

    fn get_all(prefix: &str) -> Vec<Self> {
        let read_dir = match fs::read_dir(prefix) {
            Ok(x) => x,
            Err(err) => {
                eprintln!("{err}");
                return Vec::new();
            }
        };

        let mut devices = read_dir
            .filter_map(|entry| entry.inspect_err(|err| eprintln!("{err}")).ok())
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .filter_map(|path| {
                Self::from_path(path)
                    .inspect_err(|err| eprintln!("{err}"))
                    .ok()
            })
            .collect::<Vec<_>>();

        devices.sort_by(|dev1, dev2| dev1.name.cmp(&dev2.name));
        devices
    }
}

/// Convert to a brightness value relative to a maximum brightness.
/// The conversion adjusts the value in accordance to [human perception][perception].
///
/// [perception]: https://konradstrack.ninja/blog/changing-screen-brightness-in-accordance-with-human-perception/
pub fn brightness_from_percent(percent: &Percent, max_brightness: Brightness) -> Brightness {
    let percent = percent.get();
    if percent == 0.0 || max_brightness == 0 {
        return 0;
    }
    let exp = (percent / 100.0) * f32::from(max_brightness).log10();
    (10_f32).powf(exp).round() as Brightness // Float to integer is a saturated cast
}

/// Inverse of `brightness_from_percent`.
pub fn brightness_to_percent(brightness: Brightness, max_brightness: Brightness) -> Percent {
    if brightness == 0 {
        return Percent::new(0.0).unwrap();
    }
    if max_brightness <= 1 {
        let percent = if brightness < max_brightness {
            0.0
        } else {
            100.0
        };
        return Percent::new(percent).unwrap();
    }
    let percent = f32::from(brightness).log(f32::from(max_brightness)) * 100.0;
    Percent::new(percent).expect("percent calculation to always give a valid value")
}

enum UpdateAction {
    Add,
    Sub,
    Set,
}

fn update_brightness(args: &UpdateArgs, action: UpdateAction) -> io::Result<()> {
    let mut device = Device::get(PREFIX)?;

    let percent = match action {
        UpdateAction::Add => {
            brightness_to_percent(device.brightness, device.max_brightness) + args.percent
        }
        UpdateAction::Sub => {
            brightness_to_percent(device.brightness, device.max_brightness) - args.percent
        }
        UpdateAction::Set => args.percent,
    };
    let brightness = brightness_from_percent(&percent, device.max_brightness);

    let brightness = if args.simulate {
        Ok(brightness)
    } else {
        device
            .set_brightness(brightness)
            .map(|()| device.brightness)
    }?;

    let percent = brightness_to_percent(brightness, device.max_brightness).get();
    println!("{percent:.2}");

    Ok(())
}

fn get_xdg_path() -> Option<PathBuf> {
    let base_path = match env::var_os("XDG_STATE_HOME") {
        Some(p) if !p.is_empty() => PathBuf::from(p),
        _ => env::home_dir()?,
    };

    if base_path.is_absolute() {
        Some(base_path.join("lighter"))
    } else {
        None
    }
}

#[derive(Serialize, Deserialize)]
struct DeviceData {
    path: PathBuf,
    brightness: Brightness,
}

#[derive(Debug, Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

impl Cli {
    fn run(self) -> Result<(), Box<dyn core::error::Error>> {
        match self.command {
            Command::Add(args) => update_brightness(&args, UpdateAction::Add)?,
            Command::Sub(args) => update_brightness(&args, UpdateAction::Sub)?,
            Command::Set(args) => update_brightness(&args, UpdateAction::Set)?,
            Command::Get => {
                let device = Device::get(PREFIX)?;
                let percent = brightness_to_percent(device.brightness, device.max_brightness).get();
                println!("{percent:.2}");
            }
            Command::Info => {
                for device in Device::get_all(PREFIX) {
                    println!("{device:#?}");
                }
            }
            Command::Save(args) => {
                let path = args.path.or_else(get_xdg_path).ok_or_else(|| {
                    io::Error::new(io::ErrorKind::InvalidInput, "could not determine a valid path")
                })?;
                let device = Device::get(PREFIX)?;
                let data = DeviceData {
                    path: device.path.clone(),
                    brightness: device.brightness,
                };
                fs::create_dir_all(&path)?;
                fs::write(path.join(DATA_FILE_NAME), serde_json::to_string(&data)?)?;
            }
            Command::Restore(args) => {
                let path = args
                    .path
                    .or_else(get_xdg_path)
                    .ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "could not determine a valid path",
                        )
                    })?
                    .join(DATA_FILE_NAME);

                let content = fs::read(&path)?;
                let data: DeviceData = serde_json::from_slice(&content)?;
                let mut device = Device::from_path(data.path)?;
                device.set_brightness(data.brightness)?;
                println!(
                    r#"restored device "{}" with brightness: {}"#,
                    device.name, device.brightness
                );
            }
        }

        Ok(())
    }
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Increment brightness by the given percentage.
    Add(UpdateArgs),
    /// Decrease brightness by the given percentage.
    Sub(UpdateArgs),
    /// Set brightness to the given percentage.
    Set(UpdateArgs),
    /// Get current brightness as a percentage.
    Get,
    /// Get information about backlight devices.
    Info,
    /// Save current brightness
    Save(SaveArgs),
    /// Restore brightness (inverse of `save` command)
    Restore(SaveArgs),
}

#[derive(Debug, Args)]
struct UpdateArgs {
    /// Value in the range [0, 100], supports decimals (e.g. 10.5).
    #[arg(value_parser = percent::clap_parser)]
    percent: Percent,

    /// Do not modify the brightness, only pretend to do it.
    #[arg(short, long)]
    simulate: bool,
}

#[derive(Debug, Args)]
struct SaveArgs {
    #[arg(short, long)]
    path: Option<PathBuf>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if let Err(err) = cli.run() {
        eprintln!("{err}");
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn test_cli() {
        Cli::command().debug_assert();
    }

    #[test]
    fn test_brightness_from_percent() {
        assert_eq!(brightness_from_percent(&Percent::new(0.0).unwrap(), 100), 0);
        assert_eq!(brightness_from_percent(&Percent::new(10.0).unwrap(), 100), 2);
        assert_eq!(brightness_from_percent(&Percent::new(20.0).unwrap(), 100), 3);
        assert_eq!(brightness_from_percent(&Percent::new(30.0).unwrap(), 100), 4);
        assert_eq!(brightness_from_percent(&Percent::new(40.0).unwrap(), 100), 6);
        assert_eq!(brightness_from_percent(&Percent::new(50.0).unwrap(), 100), 10);
        assert_eq!(brightness_from_percent(&Percent::new(60.0).unwrap(), 100), 16);
        assert_eq!(brightness_from_percent(&Percent::new(70.0).unwrap(), 100), 25);
        assert_eq!(brightness_from_percent(&Percent::new(80.0).unwrap(), 100), 40);
        assert_eq!(brightness_from_percent(&Percent::new(90.0).unwrap(), 100), 63);
        assert_eq!(brightness_from_percent(&Percent::new(95.0).unwrap(), 100), 79);
        assert_eq!(brightness_from_percent(&Percent::new(99.0).unwrap(), 100), 95);
        assert_eq!(brightness_from_percent(&Percent::new(100.0).unwrap(), 100), 100);
        assert_eq!(brightness_from_percent(&Percent::new(100.0).unwrap(), 12345), 12345);
    }

    #[test]
    fn test_brightness_to_percent() {
        use assert_float_eq::assert_float_absolute_eq;

        let ep = 0.01; // epsilon
        assert_float_absolute_eq!(brightness_to_percent(0, 100).get(), 0.0, ep);
        assert_float_absolute_eq!(brightness_to_percent(2, 100).get(), 15.05, ep);
        assert_float_absolute_eq!(brightness_to_percent(3, 100).get(), 23.86, ep);
        assert_float_absolute_eq!(brightness_to_percent(4, 100).get(), 30.10, ep);
        assert_float_absolute_eq!(brightness_to_percent(6, 100).get(), 38.91, ep);
        assert_float_absolute_eq!(brightness_to_percent(10, 100).get(), 50.0, ep);
        assert_float_absolute_eq!(brightness_to_percent(16, 100).get(), 60.21, ep);
        assert_float_absolute_eq!(brightness_to_percent(25, 100).get(), 69.89, ep);
        assert_float_absolute_eq!(brightness_to_percent(40, 100).get(), 80.10, ep);
        assert_float_absolute_eq!(brightness_to_percent(63, 100).get(), 89.96, ep);
        assert_float_absolute_eq!(brightness_to_percent(79, 100).get(), 94.88, ep);
        assert_float_absolute_eq!(brightness_to_percent(95, 100).get(), 98.88, ep);
        assert_float_absolute_eq!(brightness_to_percent(100, 100).get(), 100.0, ep);
        assert_float_absolute_eq!(brightness_to_percent(12345, 12345).get(), 100.0, ep);
    }
}
