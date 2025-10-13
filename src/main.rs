use clap::{Args, Parser, Subcommand, ValueEnum};
use std::borrow::Cow;
use std::env;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::device::{Device, DeviceData, Brightness};
use crate::percent::Percent;

mod device;
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
        Percent::new(percent).ok_or_else(|| format!("`{s}` is not a percentage between 0 and 100"))
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

const BIN_NAME: &str = env!("CARGO_BIN_NAME");
const DATA_FILE_NAME: &str = "device-data.json";

const LEDS_PREFIX: &str = "/sys/class/leds";
const BACKLIGHT_PREFIX: &str = "/sys/class/backlight";

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

#[derive(Clone, Copy)]
enum UpdateAction {
    Add,
    Sub,
    Set,
}

fn update_brightness(args: &UpdateArgs, action: UpdateAction) -> io::Result<()> {
    let mut device = Device::get(args.class.prefix())?;

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

fn get_xdg_state_path() -> Option<PathBuf> {
    env::var_os("XDG_STATE_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| Some(env::home_dir()?.join(".local/state")))
        .map(|p| p.join(BIN_NAME))
}

fn get_save_path(default: Option<&PathBuf>) -> io::Result<Cow<'_, Path>> {
    default
        .map(Cow::from)
        .or_else(|| Some(Cow::from(get_xdg_state_path()?.join(DATA_FILE_NAME))))
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "could not determine a valid path")
        })
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum DeviceClass {
    Leds,
    Backlight,
}

impl Default for DeviceClass {
    fn default() -> Self {
        Self::Backlight
    }
}

impl DeviceClass {
    fn prefix(self) -> &'static str {
        match self {
            Self::Leds => LEDS_PREFIX,
            Self::Backlight => BACKLIGHT_PREFIX,
        }
    }
}

#[derive(Debug, Args)]
struct UpdateArgs {
    /// Value in the range [0, 100], supports decimals (e.g. 10.5).
    #[arg(value_parser = percent::clap_parser)]
    percent: Percent,

    /// Do not modify the brightness, only pretend to do it.
    #[arg(short, long)]
    simulate: bool,

    /// Filter by device class
    #[arg(short, long, value_enum, default_value_t)]
    class: DeviceClass,

    /// Filter by device name
    #[arg(short, long)]
    device: Option<OsString>,
}

#[derive(Debug, Args)]
struct SaveArgs {
    /// Destiny of persistent files.
    #[arg(short, long)]
    file: Option<PathBuf>,

    /// Filter by device class
    #[arg(short, long, value_enum, default_value_t)]
    class: DeviceClass,

    /// Print default values used without save or restoring.
    #[arg(long, exclusive = true)]
    print_defaults: bool,
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
    Get {
        /// Filter by device class
        #[arg(short, long, value_enum, default_value_t)]
        class: DeviceClass,
    },
    /// Get information about backlight devices.
    Info {
        /// Filter by device class
        #[arg(short, long, value_enum)]
        class: Option<DeviceClass>,
    },
    /// Save current brightness
    Save(SaveArgs),
    /// Restore brightness (inverse of `save` command)
    Restore {
        /// File path to restore the brightness from
        #[arg(short, long)]
        file: Option<PathBuf>,
    },
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
            Command::Get { class } => {
                let device = Device::get(class.prefix())?;
                let percent = brightness_to_percent(device.brightness, device.max_brightness).get();
                println!("{percent:.2}");
            }
            Command::Info { class } => {
                let devices = if let Some(c) = class {
                    Device::get_all(c.prefix())
                } else {
                    let mut ds = Device::get_all(BACKLIGHT_PREFIX);
                    ds.extend(Device::get_all(LEDS_PREFIX));
                    ds
                };
                for device in devices {
                    println!("{device:#?}");
                }
            }
            Command::Save(args) => {
                let path = get_save_path(args.file.as_ref())?;
                let device = Device::get(args.class.prefix())?;

                if args.print_defaults {
                    eprintln!("file = {}", path.display());
                    eprintln!("device = {}", device.name.display());
                    return Ok(());
                }

                let data = DeviceData {
                    path: device.path,
                    brightness: device.brightness,
                };
                fs::create_dir_all(&path)?;
                fs::write(path, serde_json::to_string(&data)?)?;
            }
            Command::Restore { file } => {
                let path = get_save_path(file.as_ref())?;
                let content = fs::read(path)?;
                let data: DeviceData = serde_json::from_slice(&content)?;
                let mut device = Device::from_path(data.path)?;
                device.set_brightness(data.brightness)?;
                println!(
                    r#"restored device "{}" with brightness: {}"#,
                    device.name.display(),
                    device.brightness
                );
            }
        }

        Ok(())
    }
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
