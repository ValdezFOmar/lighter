use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

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
        fn parse_brightness(path: &Path) -> io::Result<Brightness> {
            fs::read_to_string(path)?
                .trim()
                .parse()
                .map_err(io::Error::other)
        }

        fn inner(prefix: &Path) -> io::Result<Device> {
            let name = prefix
                .file_name()
                .ok_or_else(|| io::Error::other(format!("{prefix:#?} has no file name")))?
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

    fn read_dir(prefix: &str) -> io::Result<impl Iterator<Item = PathBuf>> {
        Ok(fs::read_dir(prefix)?
            .filter_map(|entry| entry.inspect_err(|err| eprintln!("{err}")).ok())
            .map(|entry| entry.path())
            .filter(|path| path.is_dir()))
    }

    /// Returns the first encountered device under the given `prefix`.
    /// Which device is "first" is determined by alphabetical order.
    pub fn get(prefix: &str) -> io::Result<Self> {
        let mut paths = Self::read_dir(prefix)?.collect::<Vec<_>>();
        paths.sort();

        let path = paths
            .first()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no devices found"))?;

        Self::from_path(path)
    }

    pub fn get_all(prefix: &str) -> Vec<Self> {
        let read_dir = match Self::read_dir(prefix) {
            Ok(x) => x,
            Err(err) => {
                eprintln!("{err}");
                return Vec::new();
            }
        };

        let mut devices = read_dir
            .filter_map(|path| {
                Self::from_path(path)
                    .inspect_err(|err| eprintln!("{err}"))
                    .ok()
            })
            .collect::<Vec<_>>();

        devices.sort_by(|dev1, dev2| dev1.name.cmp(&dev2.name));
        devices
    }
}
