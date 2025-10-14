use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum DeviceClass {
    Leds,
    Backlight,
}

impl DeviceClass {
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::Leds => "/sys/class/leds",
            Self::Backlight => "/sys/class/backlight",
        }
    }
}

pub type Brightness = u16;

#[derive(Serialize, Deserialize)]
pub struct DeviceData {
    pub path: PathBuf,
    pub brightness: Brightness,
}

#[derive(Debug, Clone)]
pub struct Device {
    /// Device name, derived from its path.
    pub name: OsString,
    /// Full path to the device, including its name.
    pub path: PathBuf,
    pub brightness: Brightness,
    pub max_brightness: Brightness,
}

impl Device {
    pub fn set_brightness(&mut self, value: Brightness) -> io::Result<()> {
        let path = self.path.join("brightness");
        let brightness = value.min(self.max_brightness);
        fs::write(path, brightness.to_string())?;
        self.brightness = brightness;
        Ok(())
    }

    pub fn from_path(prefix: impl AsRef<Path>) -> io::Result<Self> {
        fn inner(prefix: &Path) -> io::Result<Device> {
            log::debug!("creating device from path: {}", prefix.display());

            let name = prefix
                .file_name()
                .ok_or_else(|| io::Error::other(format!("{} has no file name", prefix.display())))?
                .to_os_string();

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
                path: prefix.to_path_buf(),
            })
        }
        inner(prefix.as_ref())
    }
}

fn parse_brightness(path: &Path) -> io::Result<Brightness> {
    fs::read_to_string(path)?
        .trim()
        .parse()
        .map_err(io::Error::other)
}

#[derive(Default)]
pub struct DeviceFilters {
    pub class: Option<DeviceClass>,
    pub device_name: Option<OsString>,
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

fn iter_paths(prefix: &str) -> io::Result<impl Iterator<Item = PathBuf>> {
    Ok(fs::read_dir(prefix)?
        .filter_map(|entry| entry.inspect_err(|err| log::warn!("{err}")).ok())
        .map(|entry| entry.path())
        .filter(|path| path.is_dir()))
}

fn iter_devices(filters: &DeviceFilters) -> io::Result<impl Iterator<Item = Device>> {
    let filter_name = |path: &PathBuf| -> bool {
        filters
            .device_name
            .as_ref()
            .is_none_or(|name| path.ends_with(name))
    };

    let mut paths: Vec<PathBuf> = if let Some(class) = filters.class {
        iter_paths(class.prefix())?.filter(filter_name).collect()
    } else {
        iter_paths(DeviceClass::Backlight.prefix())?
            .chain(iter_paths(DeviceClass::Leds.prefix())?)
            .filter(filter_name)
            .collect()
    };

    paths.sort();

    Ok(paths.into_iter().filter_map(|path| {
        Device::from_path(&path)
            .inspect_err(|err| log::warn!("{err}: {}", path.display()))
            .ok()
    }))
}

/// Returns all devices matching the given filters.
pub fn get_devices(filters: &DeviceFilters) -> io::Result<Vec<Device>> {
    iter_devices(filters).map(|iter| iter.collect())
}

/// Returns the first encountered device matching the given filters.
/// Which device is "first" is determined by alphabetical order.
pub fn get_device(filters: &DeviceFilters) -> io::Result<Device> {
    iter_devices(filters)?
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no devices found"))
}
