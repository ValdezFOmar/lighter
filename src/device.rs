use std::fmt::Display;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use clap::ValueEnum;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, ValueEnum, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceClass {
    Leds,
    Backlight,
}

impl DeviceClass {
    const fn prefix(self) -> &'static str {
        match self {
            Self::Leds => "/sys/class/leds",
            Self::Backlight => "/sys/class/backlight",
        }
    }
}

impl Display for DeviceClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DeviceClass::Leds => write!(f, "leds"),
            DeviceClass::Backlight => write!(f, "backlight"),
        }
    }
}

pub type Brightness = u16;

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

#[derive(Debug, Clone)]
pub struct Device {
    /// Device name, derived from its path.
    pub name: String,
    /// Full path to the device, including its name.
    pub path: PathBuf,
    pub class: DeviceClass,
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

    pub fn from_path(prefix: impl Into<PathBuf>) -> io::Result<Self> {
        fn inner(path: PathBuf) -> io::Result<Device> {
            log::debug!("creating device from path: {}", path.display());

            let name = path
                .file_name()
                .ok_or_else(|| io::Error::other(format!("{} has no file name", path.display())))?
                .to_string_lossy()
                .into_owned();

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
                Some("leds") => DeviceClass::Leds,
                _ => DeviceClass::Backlight,
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

fn parse_brightness(path: &Path) -> io::Result<Brightness> {
    fs::read_to_string(path)?
        .trim()
        .parse()
        .map_err(io::Error::other)
}

#[derive(Default)]
pub struct DeviceFilters {
    pub class: Option<DeviceClass>,
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
        Device::from_path(path.clone())
            .inspect_err(|err| log::warn!("{err}: {}", path.display()))
            .ok()
    }))
}

fn not_found_err(filters: &DeviceFilters) -> io::Error {
    let msg = if let Some(name) = &filters.device_name {
        format!(r#"device with name "{name}" not found"#)
    } else {
        "no devices found".to_string()
    };

    io::Error::new(io::ErrorKind::NotFound, msg)
}

/// Returns all devices matching the given filters.
pub fn get_devices(filters: &DeviceFilters) -> io::Result<impl Iterator<Item = Device>> {
    let mut iter = iter_devices(filters)?.peekable();
    if iter.peek().is_some() {
        Ok(iter)
    } else {
        Err(not_found_err(filters))
    }
}

/// Returns the first encountered device matching the given filters.
/// Which device is "first" is determined by alphabetical order.
pub fn get_device(filters: &DeviceFilters) -> io::Result<Device> {
    iter_devices(filters)?
        .next()
        .ok_or_else(|| not_found_err(filters))
}
