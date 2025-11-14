use std::env;
use std::error::Error;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};

use crate::device::{Brightness, Class, Device};
use crate::percent::Percent;

mod device;

mod colors {
    pub use anstyle::Reset;
    use anstyle::{AnsiColor, Color, Style};

    pub const NONE: Style = Style::new();
    pub const BOLD: Style = Style::new().bold();

    pub const RED: Style = fg(AnsiColor::Red);
    pub const CYAN: Style = fg(AnsiColor::Cyan);
    pub const BLUE: Style = fg(AnsiColor::Blue);
    pub const GREEN: Style = fg(AnsiColor::Green);
    pub const YELLOW: Style = fg(AnsiColor::Yellow);
    pub const MAGENTA: Style = fg(AnsiColor::Magenta);

    const fn fg(color: AnsiColor) -> Style {
        Style::new().fg_color(Some(Color::Ansi(color)))
    }
}

mod logger {
    use std::io::Write;

    use log::{Level, Metadata, Record};

    use crate::BIN_NAME;
    use crate::colors::{self, CYAN, GREEN, MAGENTA, RED, Reset, YELLOW};

    pub struct Logger;

    impl log::Log for Logger {
        fn enabled(&self, metadata: &Metadata) -> bool {
            metadata.level() <= Level::Debug
        }

        fn log(&self, record: &Record) {
            if self.enabled(record.metadata()) {
                let (severity, color) = match record.level() {
                    Level::Error => ("error", RED),
                    Level::Warn => ("warning", YELLOW),
                    Level::Info => ("info", GREEN),
                    Level::Debug => ("debug", MAGENTA),
                    Level::Trace => ("trace", MAGENTA),
                };
                let msg_color = if record.level() == Level::Error {
                    colors::BOLD
                } else {
                    colors::NONE
                };
                _ = writeln!(
                    anstream::stderr(),
                    "{CYAN}{BIN_NAME}{CYAN:#}: {color}{severity}{color:#}: {msg_color}{}{Reset}",
                    record.args()
                );
            }
        }

        fn flush(&self) {}
    }
}

mod percent {
    use core::ops::{Add, Sub};
    use std::fmt;

    #[derive(Debug, Clone, Copy, PartialEq)]
    pub struct Percent(f32);

    impl Percent {
        pub const MIN: Self = Self::new(0.0).unwrap();
        pub const MAX: Self = Self::new(100.0).unwrap();

        pub const fn new(p: f32) -> Option<Self> {
            if p.is_finite() && p >= 0.0 && p <= 100.0 {
                Some(Self(p))
            } else {
                None
            }
        }

        pub const fn get(self) -> f32 {
            self.0
        }
    }

    impl fmt::Display for Percent {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            fmt::Display::fmt(&self.0, f)
        }
    }

    impl Add for Percent {
        type Output = Self;

        fn add(self, rhs: Self) -> Self::Output {
            Self::new(self.0 + rhs.0).unwrap_or(Self::MAX)
        }
    }

    impl Sub for Percent {
        type Output = Self;

        fn sub(self, rhs: Self) -> Self::Output {
            Self::new(self.0 - rhs.0).unwrap_or(Self::MIN)
        }
    }

    pub fn clap_parser(s: &str) -> Result<Percent, String> {
        let percent = s
            .parse::<f32>()
            .map_err(|_| "not a percentage".to_string())?;
        Percent::new(percent).ok_or_else(|| "not a percentage between 0 and 100".to_string())
    }

    #[test]
    fn test_percent() {
        assert_eq!(Percent::new(0.0), Some(Percent::MIN));
        assert_eq!(Percent::new(15.0), Some(Percent(15.0)));
        assert_eq!(Percent::new(100.0), Some(Percent::MAX));
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

// Formulas for calculating the perceived percentage of a given value:
//
// # value to percent
// percent = log10(value) * 100 / log10(max_value)
//         = log(value, base=max_value) * 100
// # percent to value
// value = 10 ^ (percent * log10(max_value) / 100)

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
        return Percent::MIN;
    }
    if max_brightness <= 1 {
        return if brightness < max_brightness {
            Percent::MIN
        } else {
            Percent::MAX
        };
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

fn update_brightness(args: UpdateArgs, action: UpdateAction) -> Result<(), Box<dyn Error>> {
    use UpdateAction as UA;

    let mut device = device::get_device(&args.filters.into())?;

    let percent = match action {
        UA::Add => brightness_to_percent(device.brightness, device.max_brightness) + args.percent,
        UA::Sub => brightness_to_percent(device.brightness, device.max_brightness) - args.percent,
        UA::Set => args.percent,
    };
    let brightness = brightness_from_percent(&percent, device.max_brightness);

    if !args.simulate {
        device.set_brightness(brightness)?;
    }

    let percent = brightness_to_percent(brightness, device.max_brightness);
    writeln!(io::stdout(), "{percent:.2}")?;

    Ok(())
}

fn get_xdg_state_path() -> Option<PathBuf> {
    let path = env::var_os("XDG_STATE_HOME");
    log::info!("XDG_STATE_HOME = {path:?}");
    path.filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| Some(env::home_dir()?.join(".local/state")))
        .map(|p| p.join(BIN_NAME))
}

type FilePath = (PathBuf, String);

fn get_save_path(default: Option<FilePath>) -> io::Result<FilePath> {
    default
        .or_else(|| Some((get_xdg_state_path()?, "device-data.json".into())))
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "could not determine a valid path")
        })
}

fn validate_file_path(opt: &str) -> Result<FilePath, String> {
    if opt.ends_with('/') {
        return Err("must be a path to a file, not a directory".to_string());
    }

    let path = PathBuf::from(opt);
    let base = path
        .parent()
        .map_or_else(|| PathBuf::from(""), PathBuf::from);
    let name = path
        .file_name()
        .ok_or_else(|| "path has no name component".to_string())?
        .to_string_lossy()
        .into_owned();

    Ok((base, name))
}

#[derive(Serialize, Deserialize)]
pub struct SaveData {
    pub path: PathBuf,
    pub brightness: Brightness,
}

impl From<Device> for SaveData {
    fn from(device: Device) -> Self {
        Self {
            path: device.path,
            brightness: device.brightness,
        }
    }
}

#[derive(Serialize)]
struct DeviceOutput {
    name: String,
    path: PathBuf,
    class: Class,
    brightness: Brightness,
    max_brightness: Brightness,
}

impl From<Device> for DeviceOutput {
    #[inline]
    fn from(device: Device) -> Self {
        Self {
            name: device.name,
            path: device.path,
            class: device.class,
            brightness: device.brightness,
            max_brightness: device.max_brightness,
        }
    }
}

#[derive(Args)]
struct FilterArgs {
    /// Filter by device class
    #[arg(short, long, value_enum)]
    class: Option<Class>,

    /// Filter by device name
    #[arg(short, long)]
    device: Option<String>,
}

#[derive(Args)]
struct UpdateArgs {
    /// Value in the range [0, 100], supports decimals (e.g. 10.5).
    #[arg(value_parser = percent::clap_parser)]
    percent: Percent,

    /// Do not modify any device, only pretend to do it.
    #[arg(short, long)]
    simulate: bool,

    #[command(flatten)]
    filters: FilterArgs,
}

#[derive(Copy, Clone, Default, ValueEnum)]
enum OutputFormat {
    #[default]
    Plain,
    Json,
    JsonLines,
    Csv,
}

impl OutputFormat {
    fn write<O, I>(self, mut output: O, devices: I) -> io::Result<()>
    where
        O: Write,
        I: Iterator<Item = Device>,
    {
        use crate::colors::{BLUE, CYAN, GREEN, MAGENTA, Reset as R, YELLOW};
        match self {
            OutputFormat::Plain => {
                for device in devices {
                    writeln!(output, "{MAGENTA}{}{R}", device.name)?;
                    writeln!(output, "    {CYAN}path:{R} {}", device.path.display())?;
                    writeln!(output, "    {CYAN}class:{R} {}", device.class)?;
                    writeln!(output, "    {CYAN}brightness: {R} {}", device.brightness)?;
                    writeln!(output, "    {CYAN}max brightness:{R} {}", device.max_brightness)?;
                }
            }
            OutputFormat::Json => {
                #[derive(Serialize)]
                struct Output {
                    devices: Vec<DeviceOutput>,
                }
                let devices = devices.map(DeviceOutput::from).collect();
                serde_json::to_writer(output, &Output { devices })?;
            }
            OutputFormat::JsonLines => {
                for device in devices {
                    let device = DeviceOutput::from(device);
                    serde_json::to_writer(&mut output, &device)?;
                }
            }
            OutputFormat::Csv => {
                for device in devices {
                    writeln!(
                        output,
                        "{BLUE}{}{R},{GREEN}{}{R},{YELLOW}{}{R},{CYAN}{}{R},{MAGENTA}{}{R}",
                        device.name,
                        device.path.display(),
                        device.class,
                        device.brightness,
                        device.max_brightness
                    )?;
                }
            }
        }
        Ok(())
    }
}

#[derive(Args)]
struct InfoArgs {
    /// Format to output device data
    #[arg(short, long, value_enum, default_value_t)]
    format: OutputFormat,

    #[command(flatten)]
    filters: FilterArgs,
}

#[derive(Args)]
struct SaveArgs {
    /// Path to the file where device state will be saved
    #[arg(short, long, value_parser = validate_file_path)]
    file: Option<FilePath>,

    #[command(flatten)]
    filters: FilterArgs,

    /// Print the values that would be used without saving.
    #[arg(long)]
    print_defaults: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Increment brightness by the given percentage.
    Add(UpdateArgs),
    /// Decrease brightness by the given percentage.
    Sub(UpdateArgs),
    /// Set brightness to the given percentage.
    Set(UpdateArgs),
    /// Get current brightness as a percentage.
    Get(FilterArgs),
    /// Get information about devices.
    Info(InfoArgs),
    /// Save current device(s) brightness
    Save(SaveArgs),
    /// Restore brightness (inverse of `save` command)
    Restore {
        /// Path to the file to read device state from
        #[arg(short, long, value_parser = validate_file_path)]
        file: Option<FilePath>,
    },
}

/// Control and fetch brightness information for backlight and led devices.
#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// Set verbosity level
    #[arg(short, long, action = clap::ArgAction::Count, global = true)]
    verbose: u8,

    #[command(flatten)]
    color: colorchoice_clap::Color,
}

impl Cli {
    fn log_level(&self) -> log::LevelFilter {
        match self.verbose {
            0 => log::LevelFilter::Error,
            1 => log::LevelFilter::Warn,
            2 => log::LevelFilter::Info,
            _ => log::LevelFilter::Debug,
        }
    }

    fn run(self) -> Result<ExitCode, Box<dyn Error>> {
        match self.command {
            Command::Add(args) => update_brightness(args, UpdateAction::Add)?,
            Command::Sub(args) => update_brightness(args, UpdateAction::Sub)?,
            Command::Set(args) => update_brightness(args, UpdateAction::Set)?,
            Command::Get(filters) => {
                let device = device::get_device(&filters.into())?;
                let percent = brightness_to_percent(device.brightness, device.max_brightness);
                writeln!(io::stdout(), "{percent:.2}")?;
            }
            Command::Info(args) => {
                let filters = args.filters.into();
                let devices = device::get_devices(&filters)?;
                let ouput = anstream::stdout().lock();
                args.format.write(ouput, devices)?;
            }
            Command::Save(mut args) => {
                // Save all backlight devices by default if no filters were provided,
                // on the belief that this would be the common usage.
                if args.filters.class.is_none() && args.filters.device.is_none() {
                    args.filters.class = Some(Class::Backlight);
                }

                let (base_path, name) = get_save_path(args.file)?;
                let file_path = base_path.join(name);
                let filters = args.filters.into();
                let devices = device::get_devices(&filters)?;

                if args.print_defaults {
                    let devices = devices.map(|dev| dev.name).collect::<Vec<_>>().join(", ");
                    let mut stdout = io::stdout();
                    writeln!(stdout, "file = {}", file_path.display())?;
                    writeln!(stdout, "device(s) = {devices}")?;
                    return Ok(ExitCode::SUCCESS);
                }

                let data: Vec<_> = devices.map(SaveData::from).collect();
                fs::create_dir_all(&base_path)?;
                fs::write(file_path, serde_json::to_string_pretty(&data)?)?;
            }
            Command::Restore { file } => {
                let path = {
                    let (base, name) = get_save_path(file)?;
                    base.join(name)
                };

                let content = fs::read(path)?;
                let save_data: Vec<SaveData> = serde_json::from_slice(&content)?;
                let mut fail_to_restore = false;

                // Explicitly handle all errors to allow restoring as much devices as possible.
                for data in save_data {
                    match Device::from_path(data.path) {
                        Ok(mut device) => {
                            if let Err(err) = device.set_brightness(data.brightness) {
                                fail_to_restore = true;
                                log::error!(
                                    r#"failed to set brightness for device "{}": {err}"#,
                                    device.name
                                );
                            } else {
                                log::info!(
                                    r#"restored device "{}" with brightness: {}"#,
                                    device.name,
                                    device.brightness
                                );
                            }
                        }
                        Err(err) => {
                            fail_to_restore = true;
                            log::error!("{err}");
                        }
                    }
                }

                if fail_to_restore {
                    return Ok(ExitCode::FAILURE);
                }
            }
        }

        Ok(ExitCode::SUCCESS)
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    cli.color.write_global();

    log::set_logger(&logger::Logger).expect("setting logger");
    log::set_max_level(cli.log_level());

    match cli.run() {
        Ok(code) => code,
        Err(err) => {
            if let Some(ioerr) = err.downcast_ref::<io::Error>()
                && ioerr.kind() == io::ErrorKind::BrokenPipe
            {
                return ExitCode::SUCCESS;
            }
            log::error!("{err}");
            ExitCode::FAILURE
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
