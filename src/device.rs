use std::error::Error;
use std::fmt;
use std::fmt::Display;
use std::fs;
use std::io;
use std::num::ParseIntError;
use std::path::{Path, PathBuf};

use clap::ValueEnum;
use serde::Serialize;

use error::PathError;

mod error {
    use std::error::Error;
    use std::fmt;
    use std::path::PathBuf;

    #[derive(Debug)]
    pub struct PathError<E: Error> {
        error: E,
        path: PathBuf,
    }

    impl<E: Error> PathError<E> {
        pub fn new<P: Into<PathBuf>>(error: E, path: P) -> Self {
            Self {
                error,
                path: path.into(),
            }
        }
    }

    impl<E: Error> fmt::Display for PathError<E> {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(f, "{}: \"{}\"", self.error, self.path.display())
        }
    }

    impl<E: Error> Error for PathError<E> {}
}

#[derive(Debug, Clone, Copy, ValueEnum, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Class {
    Leds,
    Backlight,
}

impl Class {
    const fn prefix(self) -> &'static str {
        match self {
            Self::Leds => "/sys/class/leds",
            Self::Backlight => "/sys/class/backlight",
        }
    }
}

impl Display for Class {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Class::Leds => write!(f, "leds"),
            Class::Backlight => write!(f, "backlight"),
        }
    }
}

pub type Brightness = u16;

#[derive(Debug)]
pub struct DeviceError(PathError<io::Error>);

impl DeviceError {
    fn from_no_name(path: impl Into<PathBuf>) -> Self {
        let err = io::Error::new(io::ErrorKind::InvalidFilename, "path has no name component");
        Self(PathError::new(err, path.into()))
    }

    fn from_parse_err(err: ParseIntError, path: impl Into<PathBuf>) -> Self {
        let err = io::Error::new(io::ErrorKind::InvalidData, err);
        Self(PathError::new(err, path.into()))
    }
}

impl fmt::Display for DeviceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl Error for DeviceError {}

impl From<PathError<io::Error>> for DeviceError {
    fn from(value: PathError<io::Error>) -> Self {
        Self(value)
    }
}

type DeviceResult<T> = Result<T, DeviceError>;

#[derive(Debug, Clone)]
pub struct Device {
    /// Device name, derived from its path.
    pub name: String,
    /// Full path to the device, including its name.
    pub path: PathBuf,
    pub class: Class,
    pub brightness: Brightness,
    pub max_brightness: Brightness,
}

impl Device {
    pub fn set_brightness(&mut self, value: Brightness) -> DeviceResult<()> {
        let path = self.path.join("brightness");
        let brightness = value.min(self.max_brightness);
        fs::write(&path, brightness.to_string()).map_err(|err| PathError::new(err, path))?;
        self.brightness = brightness;
        Ok(())
    }

    pub fn from_path(prefix: impl Into<PathBuf>) -> DeviceResult<Device> {
        fn inner(path: PathBuf) -> DeviceResult<Device> {
            log::debug!("creating device from path: {}", path.display());

            let name = path
                .file_name()
                .ok_or_else(|| DeviceError::from_no_name(&path))?
                .to_string_lossy()
                .into_owned();

            // Available paths to device properties
            // https://www.kernel.org/doc/html/latest/admin-guide/abi-stable-files.html#abi-file-stable-sysfs-class-backlight

            let brightness = parse_brightness(&path.join("brightness"))?;
            let max_brightness = parse_brightness(&path.join("max_brightness"))?;

            assert!(
                brightness <= max_brightness,
                "brightness = {brightness} > max_brightness = {max_brightness}"
            );

            let class = match path
                .parent()
                .and_then(|path| path.file_name())
                .and_then(|name| name.to_str())
            {
                Some("leds") => Class::Leds,
                _ => Class::Backlight,
            };

            Ok(Device {
                name,
                path,
                class,
                brightness,
                max_brightness,
            })
        }
        inner(prefix.into())
    }
}

fn parse_brightness(path: &Path) -> DeviceResult<Brightness> {
    fs::read_to_string(path)
        .map_err(|err| PathError::new(err, path))?
        .trim()
        .parse()
        .map_err(|err| DeviceError::from_parse_err(err, path))
}

#[derive(Debug, Clone, Default)]
pub struct DeviceFilters {
    pub class: Option<Class>,
    pub device_name: Option<String>,
}

impl From<crate::FilterArgs> for DeviceFilters {
    #[inline]
    fn from(filter: crate::FilterArgs) -> Self {
        Self {
            class: filter.class,
            device_name: filter.device,
        }
    }
}

fn iter_paths(prefix: &str) -> Result<impl Iterator<Item = PathBuf>, PathError<io::Error>> {
    Ok(fs::read_dir(prefix)
        .map_err(|err| PathError::new(err, prefix))?
        .filter_map(|entry| entry.inspect_err(|err| log::warn!("{err}")).ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_dir()))
}

fn iter_devices(
    filters: &DeviceFilters,
) -> Result<impl Iterator<Item = Device>, PathError<io::Error>> {
    let filter_name = |path: &PathBuf| -> bool {
        filters
            .device_name
            .as_ref()
            .is_none_or(|name| path.ends_with(name))
    };

    let mut paths: Vec<PathBuf> = if let Some(class) = filters.class {
        iter_paths(class.prefix())?.filter(filter_name).collect()
    } else {
        iter_paths(Class::Backlight.prefix())?
            .chain(iter_paths(Class::Leds.prefix())?)
            .filter(filter_name)
            .collect()
    };

    paths.sort();

    Ok(paths.into_iter().filter_map(|path| {
        Device::from_path(path)
            .inspect_err(|err| log::warn!("{err}"))
            .ok()
    }))
}

#[derive(Debug)]
pub enum FetchDeviceError {
    IO(PathError<io::Error>),
    NotFound(DeviceFilters),
}

impl fmt::Display for FetchDeviceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::IO(error) => error.fmt(f),
            Self::NotFound(filters) => {
                if let Some(name) = &filters.device_name {
                    write!(f, r#"device with name "{name}" not found"#)
                } else {
                    f.write_str("no devices found")
                }
            }
        }
    }
}

impl Error for FetchDeviceError {}

impl From<PathError<io::Error>> for FetchDeviceError {
    fn from(value: PathError<io::Error>) -> Self {
        FetchDeviceError::IO(value)
    }
}

type FetchResult<T> = Result<T, FetchDeviceError>;

/// Returns all devices matching the given filters.
pub fn get_devices(filters: &DeviceFilters) -> FetchResult<impl Iterator<Item = Device> + '_> {
    let mut iter = iter_devices(filters)?.peekable();
    if iter.peek().is_some() {
        Ok(iter)
    } else {
        Err(FetchDeviceError::NotFound(filters.clone()))
    }
}

/// Returns the first encountered device matching the given filters.
/// Which device is "first" is determined by alphabetical order.
pub fn get_device(filters: &DeviceFilters) -> FetchResult<Device> {
    iter_devices(filters)?
        .next()
        .ok_or_else(|| FetchDeviceError::NotFound(filters.clone()))
}
