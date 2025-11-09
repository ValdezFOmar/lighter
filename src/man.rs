use std::io;

use clap::CommandFactory;
use clap_mangen::Man;
use clap_mangen::roff::{Roff, italic, roman};

use crate::BIN_NAME;
use crate::Cli;

fn render_examples_section(roff: &mut Roff) {
    roff.control("SH", ["EXAMPLES"]);

    roff.control("TP", None);
    roff.text([roman(BIN_NAME), roman(" set 50")]);
    roff.text([roman("Set current brightness to 50 percent.")]);

    roff.control("TP", None);
    roff.text([roman(BIN_NAME), roman(" add 10")]);
    roff.text([roman("Add 10 percent to current brightness.")]);

    roff.control("TP", None);
    roff.text([roman(BIN_NAME), roman(" sub 20")]);
    roff.text([roman("Subtract 20 percent to current brightness.")]);

    roff.control("TP", None);
    roff.text([
        roman(BIN_NAME),
        roman(" get --device "),
        italic("platform::fnlock"),
        roman(" --class "),
        italic("leds"),
    ]);
    roff.text([
        roman("Print current brightness for device with name "),
        italic("platform::fnlock"),
        roman(" and device class "),
        italic("leds to stdout."),
    ]);

    roff.control("TP", None);
    roff.text([
        roman(BIN_NAME),
        roman(" info --class "),
        italic("backlight"),
        roman(" --format csv"),
    ]);
    roff.text([
        roman("Print information about devices matching the class "),
        italic("backlight"),
        roman(" using a CSV format to stdout."),
    ]);
}

fn render_subcommands_section(roff: &mut Roff, cmd: &clap::Command) {
    roff.control("SH", ["SUBCOMMANDS"]);
    for subcmd in cmd.get_subcommands() {
        roff.control("SS", [subcmd.get_name()]);
        if let Some(about) = subcmd.get_long_about().or_else(|| subcmd.get_about()) {
            for line in about.to_string().lines() {
                if line.trim().is_empty() {
                    roff.control("PP", []);
                } else {
                    roff.text([roman(line)]);
                }
            }
        }
    }
}

pub fn render(output: &mut dyn io::Write) -> io::Result<()> {
    let man = Man::new(Cli::command());
    man.render_title(output)?;
    man.render_name_section(output)?;
    man.render_synopsis_section(output)?;
    man.render_description_section(output)?;
    man.render_options_section(output)?;

    let mut roff = Roff::new();
    render_subcommands_section(&mut roff, &Cli::command());
    render_examples_section(&mut roff);
    roff.to_writer(output)?;

    man.render_version_section(output)?;
    Ok(())
}
