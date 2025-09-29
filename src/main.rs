use clap::{Args, Parser, Subcommand};
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

type Brightness = u16;

#[derive(Debug, Clone)]
struct Device {
    name: String,
    prefix: PathBuf,
    brightness: Brightness,
    max_brightness: Brightness,
}

impl Device {
    fn set_brightness(&mut self, value: Brightness) -> io::Result<()> {
        let path = self.prefix.join(&self.name).join("brightness");
        let brightness = value.min(self.max_brightness);
        fs::write(path, brightness.to_string())?;
        self.brightness = brightness;
        Ok(())
    }

    fn get(prefix: impl AsRef<Path>) -> io::Result<Self> {
        fn parse_brightness(path: &Path) -> io::Result<Brightness> {
            fs::read_to_string(path)?
                .trim()
                .parse::<Brightness>()
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
                prefix: PathBuf::from(prefix),
            })
        }

        inner(prefix.as_ref())
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
            .filter_map(|path| Self::get(path).inspect_err(|err| eprintln!("{err}")).ok())
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

fn update_brightness<F>(args: &UpdateArgs, calc_percent: F) -> ExitCode
where
    F: FnOnce(Percent) -> Percent,
{
    let mut devices = Device::get_all(PREFIX);
    let Some(device) = devices.first_mut() else {
        eprintln!("no device found");
        return ExitCode::FAILURE;
    };

    let percent = calc_percent(brightness_to_percent(device.brightness, device.max_brightness));
    let brightness = brightness_from_percent(&percent, device.max_brightness);

    let result = if args.simulate {
        Ok(brightness)
    } else {
        device
            .set_brightness(brightness)
            .map(|()| device.brightness)
    };

    match result {
        Ok(brightness) => {
            let percent = brightness_to_percent(brightness, device.max_brightness).get();
            println!("{percent:.2}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Debug, Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
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

fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.command {
        Command::Add(args) => update_brightness(&args, |percent| percent + args.percent),
        Command::Sub(args) => update_brightness(&args, |percent| percent - args.percent),
        Command::Set(args) => update_brightness(&args, |_| args.percent),
        Command::Get => {
            let devices = Device::get_all(PREFIX);
            if let Some(device) = devices.first() {
                let percent = brightness_to_percent(device.brightness, device.max_brightness).get();
                println!("{percent:.2}");
            } else {
                eprintln!("no devices found");
            }
            ExitCode::SUCCESS
        }
        Command::Info => {
            for device in Device::get_all(PREFIX) {
                println!("{device:#?}");
            }
            ExitCode::SUCCESS
        }
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
