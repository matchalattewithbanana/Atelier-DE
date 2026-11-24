use anyhow::{bail, Context, Result};
use atelier_services::system_power_snapshot;
use std::env;

fn main() -> Result<()> {
    let args: Vec<_> = env::args().skip(1).collect();
    match args.as_slice() {
        [arg] if arg == "power" => {
            let snapshot = system_power_snapshot().context("read Linux power supply inventory")?;
            println!("{}", serde_json::to_string(&snapshot)?);
        }
        [arg] if arg == "--help" || arg == "-h" => println!("atelier-services power\nPrint a versioned JSON power snapshot. No privileges or compositor required."),
        _ => bail!("usage: atelier-services power"),
    }
    Ok(())
}
