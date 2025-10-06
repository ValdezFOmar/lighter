# Brighter

## Interface

For the MVP, only support cases where there's just one device.
Proposed API:

```bash
brighter info # show backlight device information
brighter get {value} # get current brightness percentage
brighter set {value} # set brightness percentage
brighter add {value} # add percentage to current brightness
brighter sub {value} # subtract percentage from current brightness
```

Where:

- `{value}` is always a percent (without '%')
- The percentage is calculated relative to the maximum and scaled to
  adjust it to [human perception][perception].

Formula for calculating the perceived percentage of a given value:

```
# value to percent
percent = log10(value) * 100 / log10(max_value)
        = log(value, base=max_value) * 100
# percent to value
value = 10 ^ (percent * log10(max_value) / 100)
```

[perception]: https://konradstrack.ninja/blog/changing-screen-brightness-in-accordance-with-human-perception/
[sysfs-backlight]: https://www.kernel.org/doc/html/latest/admin-guide/abi-stable-files.html#abi-file-stable-sysfs-class-backlight
