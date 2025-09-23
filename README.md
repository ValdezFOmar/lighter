# Lighter

Set monitor/screen brightness using [`/sys/class/backlight`][sysfs-backlight].

## Interface

For the MVP, only support cases where there's just one device.
Proposed API:

```bash
lighter info # show backlight device information
lighter get {value} # get current brightness percentage
lighter set {value} # set brightness percentage
lighter add {value} # add percentage to current brightness
lighter sub {value} # subtract percentage from current brightness
```

Where:

- `{value}` is always a percent (without '%')
- The percentage is calculated relative to the maximum and scaled to
  adjust it to [human perception][perception].

Take a look at <https://gitlab.com/wavexx/acpilight>

Formula for calculating the perceived percentage of a given value:

```
# value to percent
percent = (log10(value) * 100) / log10(max_value)
# percent to value
value = floor(10 ^ ((percent * log10(max_value)) / 100))
```

[perception]: https://konradstrack.ninja/blog/changing-screen-brightness-in-accordance-with-human-perception/
[sysfs-backlight]: https://www.kernel.org/doc/html/latest/admin-guide/abi-stable-files.html#abi-file-stable-sysfs-class-backlight
