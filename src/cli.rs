use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, Parser)]
#[command(
    name = "devdeck",
    version,
    about = "A terminal repository explorer with live previews."
)]
pub struct Cli {
    #[arg(value_name = "PATH", default_value = ".")]
    pub path: PathBuf,

    #[arg(long, help = "Show hidden files and directories.")]
    pub hidden: bool,

    #[arg(long, help = "Disable filesystem watching.")]
    pub no_watch: bool,
}

impl Cli {
    pub fn parse_args() -> Self {
        Self::parse()
    }
}
