// Self-Hosting TriCTI Compiler Main Binary
use clap::Parser;
use tricti::cli::{Cli, CliHandler};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let handler = CliHandler::new();
    
    handler.handle_command(cli.command)?;
    
    Ok(())
}