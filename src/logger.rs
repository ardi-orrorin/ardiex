use anyhow::Result;
use env_logger::Builder;
use log::LevelFilter;
use std::fs;
use std::path::PathBuf;

pub fn init_file_logging(log_dir: &PathBuf) -> Result<()> {
    fs::create_dir_all(log_dir)?;
    
    let log_file = log_dir.join("ardiex.log");
    
    // Simple file logging setup
    Builder::new()
        .target(env_logger::Target::Pipe(Box::new(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&log_file)?
        )))
        .filter_level(LevelFilter::Info)
        .init();
    
    println!("Logging to file: {:?}", log_file);
    Ok(())
}
