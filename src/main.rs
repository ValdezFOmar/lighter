use clap::{Args, Parser, Subcommand};
use std::fs;
use std::io;
use std::path::Path;
use std::process::ExitCode;

use crate::percent::Percent;

mod percent {
    #[derive(Debug, Clone, PartialEq)]
    pub struct Percent(f32);

    impl Percent {
        pub fn new(p: f32) -> Option<Self> {
            if p.is_finite() && (0.0..=100.0).contains(&p) { Some(Self(p)) } else { None }
        }

        pub fn get(&self) -> f32 {
            self.0
        }
    }

    pub fn clap_parser(s: &str) -> Result<Percent, String> {
        let percent: f32 = s.parse().map_err(|_| format!("`{s}` is not a percentage"))?;
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
    brightness: Brightness,
    max_brightness: Brightness,
}

impl Device {
    fn set_brightness(&mut self, value: Brightness) -> io::Result<()> {
        let path = Path::new(PREFIX).join(&self.name).join("brightness");
        let brightness = value.min(self.max_brightness);
        fs::write(path, brightness.to_string())?;
        self.brightness = brightness;
        Ok(())
    }

    fn get_all() -> Vec<Self> {
        let Ok(read_dir) = Path::new(PREFIX).read_dir() else {
            return Vec::new();
        };

        let mut devices = read_dir
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .map(|path| {
                let name = path
                    .file_name()
                    .expect("the path to have a name")
                    .to_str()
                    .expect("the name to be valid UTF-8")
                    .to_string();
                let brightness = fs::read_to_string(path.join("brightness"))
                    .expect("brightness file to exists")
                    .trim()
                    .parse::<Brightness>()
                    .expect("brightness to be an integer");
                let max_brightness = fs::read_to_string(path.join("max_brightness"))
                    .expect("max_brightness file to exists")
                    .trim()
                    .parse::<Brightness>()
                    .expect("max_brightness to be an integer");

                assert!(brightness <= max_brightness);

                Device { name, brightness, max_brightness }
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
    let exp = (percent / 100.0) * (max_brightness as f32).log10();
    (10_f32).powf(exp).round() as Brightness // Float to integer is a saturated cast
}

/// Inverse of `brightness_from_percent`.
pub fn brightness_to_percent(brightness: Brightness, max_brightness: Brightness) -> Percent {
    if brightness == 0 {
        return Percent::new(0.0).unwrap();
    }
    if max_brightness <= 1 {
        return Percent::new(if brightness <= max_brightness { 0.0 } else { 100.0 }).unwrap();
    }
    let percent = (brightness as f32).log(max_brightness as f32) * 100.0;
    Percent::new(percent).expect("percent calculcation to always give a valid value")
}

fn update_brightness<F>(args: &UpdateArgs, calc_brightness: F) -> ExitCode
where
    F: FnOnce(&Device, Brightness) -> Brightness,
{
    let mut devices = Device::get_all();
    let Some(device) = devices.first_mut() else {
        eprintln!("no device found");
        return ExitCode::FAILURE;
    };

    let brightness = brightness_from_percent(&args.percent, device.max_brightness);
    let total_brightness = calc_brightness(device, brightness);

    let result = if args.simulate {
        Ok(total_brightness)
    } else {
        device.set_brightness(total_brightness).and_then(|_| Ok(device.brightness))
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

    /// Do not modify the brightness, only pretend that it does.
    #[arg(long)]
    simulate: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.command {
        Command::Add(args) => update_brightness(&args, |dev, b| dev.brightness.saturating_add(b)),
        Command::Sub(args) => update_brightness(&args, |dev, b| dev.brightness.saturating_sub(b)),
        Command::Set(args) => update_brightness(&args, |_, brightness| brightness),
        Command::Get => {
            let devices = Device::get_all();
            if let Some(device) = devices.first() {
                let percent = brightness_to_percent(device.brightness, device.max_brightness).get();
                println!("{percent:.2}");
            } else {
                eprintln!("no devices found");
            };
            ExitCode::SUCCESS
        }
        Command::Info => {
            for device in Device::get_all() {
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

    // TODO:
    // These tests should be the inverse of each other,
    // but since the percentage is rounded to a integer it causes loss of precision,
    // which becomes more noticeable for the smaller brightness values.
    // `Percent` should really support fractional values, which would also require support
    // for decimal percentages in the CLI (custom validator).

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
