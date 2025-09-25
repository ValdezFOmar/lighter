use clap::{Args, Parser, Subcommand};
use std::fs;
use std::io;
use std::path::Path;
use std::process::ExitCode;

use crate::percent::Percent;

mod percent {
    use super::Brightness;

    #[derive(Debug, PartialEq)]
    pub struct Percent(u8);

    impl Percent {
        pub fn new(value: u8) -> Option<Self> {
            match value {
                0..=100 => Some(Self(value)),
                _ => None,
            }
        }

        pub fn get(&self) -> u8 {
            self.0
        }

        pub fn from_brightness(brightness: Brightness, max_brightness: Brightness) -> Self {
            let percent = ((brightness as f32).log10() * 100.0) / (max_brightness as f32).log10();
            Self::new(percent as u8)
                .expect("calculation of percent to always fall in the range 0..=100")
        }

        /// Convert to a brightness value relative to a maximum brightness.
        /// The conversion adjusts the value in accordance to [human perception][perception].
        ///
        /// [perception]: https://konradstrack.ninja/blog/changing-screen-brightness-in-accordance-with-human-perception/
        pub fn to_brightness(&self, max_brightness: Brightness) -> Brightness {
            if self.0 == 0 {
                return 0;
            }
            let exp = (self.0 as f32 / 100.0) * (max_brightness as f32).log10();
            (10_f32).powf(exp).floor() as Brightness // Float to integer is a saturated cast
        }
    }

    #[test]
    fn test_percent() {
        use std::u8;
        assert_eq!(Percent::new(0), Some(Percent(0)));
        assert_eq!(Percent::new(15), Some(Percent(15)));
        assert_eq!(Percent::new(100), Some(Percent(100)));
        assert_eq!(Percent::new(101), None);
        assert_eq!(Percent::new(u8::MAX), None);
    }

    #[test]
    fn test_percent_to_brightness() {
        assert_eq!(Percent::new(0).unwrap().to_brightness(100), 0);
        assert_eq!(Percent::new(10).unwrap().to_brightness(100), 1);
        assert_eq!(Percent::new(20).unwrap().to_brightness(100), 2);
        assert_eq!(Percent::new(30).unwrap().to_brightness(100), 3);
        assert_eq!(Percent::new(40).unwrap().to_brightness(100), 6);
        assert_eq!(Percent::new(50).unwrap().to_brightness(100), 10);
        assert_eq!(Percent::new(60).unwrap().to_brightness(100), 15);
        assert_eq!(Percent::new(70).unwrap().to_brightness(100), 25);
        assert_eq!(Percent::new(80).unwrap().to_brightness(100), 39);
        assert_eq!(Percent::new(90).unwrap().to_brightness(100), 63);
        assert_eq!(Percent::new(95).unwrap().to_brightness(100), 79);
        assert_eq!(Percent::new(99).unwrap().to_brightness(100), 95);
        assert_eq!(Percent::new(100).unwrap().to_brightness(100), 100);
        assert_eq!(Percent::new(100).unwrap().to_brightness(12345), 12345);
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
        let path = Path::new(PREFIX)
            .join(&self.name)
            .join("brightness");
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

fn update_brightness<F>(args: &SubArgs, calc_brightness: F) -> ExitCode
where
    F: FnOnce(&Device, Brightness) -> Brightness,
{
    let mut devices = Device::get_all();
    let Some(device) = devices.first_mut() else {
        eprintln!("no device found");
        return ExitCode::FAILURE;
    };

    let brightness = Percent::new(args.percent)
        .expect("a value in the range 0..=100")
        .to_brightness(device.max_brightness);
    let total_brightness = calc_brightness(&device, brightness);

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
                let percent = Percent::from_brightness(device.brightness, device.max_brightness);
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
}
