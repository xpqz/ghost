use argh::FromArgs;
use ghost::mkdocs_utils::audit;
use std::path::PathBuf;

#[derive(FromArgs, Debug)]
/// Audit MkDocs navigation vs on-disk markdown.
struct Cli {
    #[argh(option, long = "mkdocs-yaml")]
    /// path to the mkdocs.yml file to read
    mkdocs_yaml: PathBuf,

    #[argh(option, long = "help-urls")]
    /// path to the header file containing HELP_URL definitions
    help_urls: PathBuf,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Cli {
        mkdocs_yaml,
        help_urls,
    } = argh::from_env();

    let audit = audit(&mkdocs_yaml, &help_urls)?;

    println!("Missing nav entries: {:#?}", audit.nav_missing);
    println!("Ghost files: {:#?}", audit.ghost);
    println!("Missing help files: {:#?}", audit.help_missing);

    Ok(())
}
