# Brighter

Command line utility to control and fetch brightness information for
backlight and led devices.

## Examples

### Get brightness

Get the current brightness as a percentage:

```console
$ brighter get
65.15
```

### Set brightness

Set brightness to a new value as a percentage:

```console
$ brighter set 50
50.00
$ brighter add 10
60.00
$ brighter sub 20
40.00
```

The value supplied is always taken as a percentage (without `%`) and is
scaled to adjust it to [human perception][perception]. Fractional
percentages are supported, e.g. `12.5`.

### Get device info

Get general information for available devices:

```console
$ brighter info
intel_backlight
    path: /sys/class/backlight/intel_backlight
    class: backlight
    brightness:  514
    max brightness: 21333
platform::fnlock
    path: /sys/class/leds/platform::fnlock
    class: leds
    brightness:  1
    max brightness: 1
```

You can also specify a different format:

```console
$ brighter info --format=csv
intel_backlight,/sys/class/backlight/intel_backlight,backlight,514,21333
platform::fnlock,/sys/class/leds/platform::fnlock,leds,1,1

$ brighter info --format=json-lines
{"name":"intel_backlight","path":"/sys/class/backlight/intel_backlight","class":"backlight","brightness":514,"max_brightness":21333}
{"name":"platform::fnlock","path":"/sys/class/leds/platform::fnlock","class":"leds","brightness":1,"max_brightness":1}
```

### Save/Restore brightness

You can save the current brightness value for devices using the `save`
command: `$ brighter save`. By default, the brightness for all devices
of class `backlight` is saved, but you can change this using
[filters](#filters).

The saved brightness value is stored under under
`$XDG_STATE_HOME/brighter` or `~/.local/state/brighter` by default.
You can restore the brightness with the `restore` command: `$ brighter
restore`.

### Filters

Most commands accept filter arguments to target devices more
specifically, for example:

```console
$ brighter get --device platform::fnlock --class leds
100.00

$ brighter info --class leds --format csv
input2::capslock,/sys/class/leds/input2::capslock,leds,0,1
platform::fnlock,/sys/class/leds/platform::fnlock,leds,1,1

$ brighter set --device input2::capslock 100
100.00
```

[perception]: https://konradstrack.ninja/blog/changing-screen-brightness-in-accordance-with-human-perception/
