use clap::{Args, Parser, Subcommand};
use std::fs;
use std::io;
use std::path::Path;
use std::process::ExitCode;

use crate::percent::Percent;

mod percent {
    #[derive(Debug, PartialEq)]
    pub struct Percent(u8);

    impl Percent {
        pub fn new(value: u8) -> Self {
            Self(value.max(100))
        }

        pub fn get(&self) -> u8 {
            self.0
        }
    }

    #[test]
    fn test_percent() {
        use std::u8;
        assert_eq!(Percent::new(0), Percent(0));
        assert_eq!(Percent::new(15), Percent(15));
        assert_eq!(Percent::new(100), Percent(100));
        assert_eq!(Percent::new(101), Percent(100));
        assert_eq!(Percent::new(u8::MAX), Percent(100));
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

                Device {
                    name,
                    brightness,
                    max_brightness,
                }
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
    if percent == 0 || max_brightness == 0 {
        return 0;
    }
    let exp = (percent as f32 / 100.0) * (max_brightness as f32).log10();
    (10_f32).powf(exp).round() as Brightness // Float to integer is a saturated cast
}

/// Inverse of `brightness_from_percent`.
pub fn brightness_to_percent(brightness: Brightness, max_brightness: Brightness) -> Percent {
    if brightness == 0 {
        return Percent::new(0);
    }
    if max_brightness <= 1 {
        return Percent::new(if brightness <= max_brightness { 0 } else { 100 });
    }
    let percent = (brightness as f32).log(max_brightness as f32) * 100.0;
    Percent::new(percent.round() as u8)
}

fn update_brightness<F>(args: &SubArgs, calc_brightness: F) -> ExitCode
where
    F: FnOnce(&Device, Brightness) -> Brightness,
{
    let mut devices = Device::get_all();
    let Some(device) = devices.first_mut() else {
        eprintln!("no device found");
        return ExitCode::FAILURE;
    };

    let percent = Percent::new(args.percent);
    let brightness = brightness_from_percent(&percent, device.max_brightness);
    let total_brightness = calc_brightness(device, brightness);

    match device.set_brightness(total_brightness) {
        Ok(()) => ExitCode::SUCCESS,
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
    Add(SubArgs),
    Sub(SubArgs),
    Set(SubArgs),
    Get,
    Info,
}

#[derive(Debug, Args)]
struct SubArgs {
    #[arg(value_parser = clap::value_parser!(u8).range(0..=100))]
    percent: u8,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    match cli.command {
        Command::Add(args) => update_brightness(&args, |dev, brightness| {
            dev.brightness.saturating_add(brightness)
        }),
        Command::Sub(args) => update_brightness(&args, |dev, brihtness| {
            dev.brightness.saturating_sub(brihtness)
        }),
        Command::Set(args) => update_brightness(&args, |_, brightness| brightness),
        Command::Get => {
            let devices = Device::get_all();
            if let Some(device) = devices.first() {
                let percent = brightness_to_percent(device.brightness, device.max_brightness);
                println!("{}", percent.get());
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
        assert_eq!(brightness_from_percent(&Percent::new(0), 100), 0);
        assert_eq!(brightness_from_percent(&Percent::new(10), 100), 2);
        assert_eq!(brightness_from_percent(&Percent::new(20), 100), 3);
        assert_eq!(brightness_from_percent(&Percent::new(30), 100), 4);
        assert_eq!(brightness_from_percent(&Percent::new(40), 100), 6);
        assert_eq!(brightness_from_percent(&Percent::new(50), 100), 10);
        assert_eq!(brightness_from_percent(&Percent::new(60), 100), 16);
        assert_eq!(brightness_from_percent(&Percent::new(70), 100), 25);
        assert_eq!(brightness_from_percent(&Percent::new(80), 100), 40);
        assert_eq!(brightness_from_percent(&Percent::new(90), 100), 63);
        assert_eq!(brightness_from_percent(&Percent::new(95), 100), 79);
        assert_eq!(brightness_from_percent(&Percent::new(99), 100), 95);
        assert_eq!(brightness_from_percent(&Percent::new(100), 100), 100);
        assert_eq!(brightness_from_percent(&Percent::new(100), 12345), 12345);
    }

    #[test]
    fn test_brightness_to_percent() {
        assert_eq!(brightness_to_percent(0, 100), Percent::new(0));
        assert_eq!(brightness_to_percent(2, 100), Percent::new(15));
        assert_eq!(brightness_to_percent(3, 100), Percent::new(24));
        assert_eq!(brightness_to_percent(4, 100), Percent::new(30));
        assert_eq!(brightness_to_percent(6, 100), Percent::new(39));
        assert_eq!(brightness_to_percent(10, 100), Percent::new(50));
        assert_eq!(brightness_to_percent(16, 100), Percent::new(60));
        assert_eq!(brightness_to_percent(25, 100), Percent::new(70));
        assert_eq!(brightness_to_percent(40, 100), Percent::new(80));
        assert_eq!(brightness_to_percent(63, 100), Percent::new(90));
        assert_eq!(brightness_to_percent(79, 100), Percent::new(95));
        assert_eq!(brightness_to_percent(95, 100), Percent::new(99));
        assert_eq!(brightness_to_percent(100, 100), Percent::new(100));
        assert_eq!(brightness_to_percent(12345, 12345), Percent::new(100));
    }
}
