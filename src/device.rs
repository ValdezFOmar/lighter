use std::fmt;
use std::fmt::Display;
use std::fs;
use std::io;
use std::num::ParseIntError;
use std::path::{Path, PathBuf};

use clap::ValueEnum;
use serde::Serialize;
use zbus::zvariant::Type;

use error::PathError;

pub use controller::Controller;

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

mod controller {
    use std::{fmt, io};

    use zbus::blocking::connection::Connection;
    use zbus::proxy;

    use super::{Brightness, Class, Device, error::PathError};

    #[derive(Debug)]
    pub enum Error {
        IO(PathError<io::Error>),
        DBus(zbus::Error),
    }

    impl From<zbus::Error> for Error {
        fn from(value: zbus::Error) -> Self {
            Self::DBus(value)
        }
    }

    impl From<PathError<io::Error>> for Error {
        fn from(value: PathError<io::Error>) -> Self {
            Self::IO(value)
        }
    }

    impl fmt::Display for Error {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Error::IO(error) => error.fmt(f),
                Error::DBus(error) => error.fmt(f),
            }
        }
    }

    impl core::error::Error for Error {}

    #[proxy(
        default_service = "org.freedesktop.login1",
        default_path = "/org/freedesktop/login1/session/auto",
        interface = "org.freedesktop.login1.Session"
    )]
    trait Session {
        // `SetBrightness()` method, needs to be connected to the system bus.
        // See: org.freedesktop.login1(5)
        fn set_brightness(&self, class: Class, name: &str, brightness: u32) -> zbus::Result<()>;
    }

    pub struct Controller(Option<Connection>);

    impl Controller {
        pub fn new() -> Self {
            let connection = Connection::system().inspect_err(|err| {
                log::warn!("failed to connect to system bus: {err}");
            });
            Self(connection.ok())
        }

        pub fn set_brightness(&self, device: &mut Device, value: Brightness) -> Result<(), Error> {
            let brightness = value.min(device.max_brightness);
            if let Some(connection) = &self.0 {
                log::debug!("setting brightness using D-Bus");
                let proxy = SessionProxyBlocking::new(connection)?;
                proxy.set_brightness(device.class, &device.name, u32::from(value))?;
            } else {
                let path = device.path.join("brightness");
                log::debug!("setting brightness by writing to {}", path.display());
                std::fs::write(&path, value.to_string())
                    .map_err(|err| PathError::new(err, path))?;
            }
            device.brightness = brightness;
            Ok(())
        }
    }
}

#[derive(Debug, Clone, Copy, Type, ValueEnum, Serialize)]
#[serde(rename_all = "lowercase")]
#[zvariant(signature = "s")]
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
pub struct Error(PathError<io::Error>);

impl Error {
    fn from_no_name(path: impl Into<PathBuf>) -> Self {
        let err = io::Error::new(io::ErrorKind::InvalidFilename, "path has no name component");
        Self(PathError::new(err, path.into()))
    }

    fn from_parse_err(err: ParseIntError, path: impl Into<PathBuf>) -> Self {
        let err = io::Error::new(io::ErrorKind::InvalidData, err);
        Self(PathError::new(err, path.into()))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl core::error::Error for Error {}

impl From<PathError<io::Error>> for Error {
    fn from(value: PathError<io::Error>) -> Self {
        Self(value)
    }
}

type DeviceResult<T> = Result<T, Error>;

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
    pub fn from_path(prefix: impl Into<PathBuf>) -> DeviceResult<Device> {
        fn inner(path: PathBuf) -> DeviceResult<Device> {
            log::debug!("creating device from path: {}", path.display());

            let name = path
                .file_name()
                .ok_or_else(|| Error::from_no_name(&path))?
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
        .map_err(|err| Error::from_parse_err(err, path))
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
) -> Result<impl Iterator<Item = Device> + '_, PathError<io::Error>> {
    let mut paths: Vec<PathBuf> = if let Some(class) = filters.class {
        iter_paths(class.prefix())?.collect()
    } else {
        iter_paths(Class::Backlight.prefix())?
            .chain(iter_paths(Class::Leds.prefix())?)
            .collect()
    };

    paths.sort();

    let paths = paths.into_iter().filter_map(|path| {
        if filters
            .device_name
            .as_ref()
            .is_none_or(|name| path.ends_with(name))
        {
            Device::from_path(path)
                .inspect_err(|err| log::warn!("{err}"))
                .ok()
        } else {
            None
        }
    });

    Ok(paths)
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
