/*!
# DAT3 - Fallout Archive Tool

A cross-platform tool for managing Fallout 1 and 2 DAT archive files.
Supports both DAT1 (Fallout 1) and DAT2 (Fallout 2) formats.
*/

use anyhow::{bail, Result};
use clap::{Parser, Subcommand};
use std::path::PathBuf;

// Use a faster memory allocator on Linux
#[cfg(target_os = "linux")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod common; // Shared utilities and the main DatArchive interface
mod dat1; // Fallout 1 DAT format implementation
mod dat2; // Fallout 2 DAT format implementation
mod lzss; // LZSS decompression for DAT1 files

#[cfg(test)]
mod common_tests;

use common::{utils, CompressionLevel, DatArchive, ExtractionMode};

/// Command-line interface definition.
/// The `clap` crate uses these derive macros to automatically parse arguments.
#[derive(Parser)]
#[command(name = "dat3")]
#[command(author = "DAT Tool Rewrite")]
#[command(about = "Fallout .dat management cli")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

/// All supported commands for working with DAT archives
#[derive(Subcommand)]
enum Commands {
    /// List files in a DAT archive
    #[command(name = "l")]
    List {
        dat_file: PathBuf,
        /// Specific files to list (if empty, lists all)
        files: Vec<String>,
    },

    /// Extract files preserving directory structure
    #[command(name = "x")]
    Extract {
        dat_file: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
        files: Vec<String>,
    },

    /// Extract files flat (no subdirectories)
    #[command(name = "e")]
    ExtractFlat {
        dat_file: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
        files: Vec<String>,
    },

    /// Add files to a DAT archive
    #[command(name = "a")]
    Add {
        dat_file: PathBuf,
        /// Files or directories to add
        files: Vec<PathBuf>,
        /// Compression level 0-9
        #[arg(short, long)]
        compression: Option<u8>,
        /// Force DAT1 format for new archives
        #[arg(long)]
        dat1: bool,
        /// Target directory inside the archive
        #[arg(short, long)]
        target_dir: Option<String>,
    },

    /// Delete files from a DAT archive
    #[command(name = "d")]
    Delete {
        dat_file: PathBuf,
        files: Vec<String>,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::List { dat_file, files } => {
            let archive = DatArchive::open(&dat_file)?;
            let patterns = utils::expand_response_files_for_archive(&files)?;
            archive.list(&patterns)?;
        }

        Commands::Extract {
            dat_file,
            output,
            files,
        } => {
            let archive = DatArchive::open(&dat_file)?;
            let output_dir = output.unwrap_or_else(|| PathBuf::from(".")); // default: current directory
            let patterns = utils::expand_response_files_for_archive(&files)?;
            archive.extract(&output_dir, &patterns, ExtractionMode::PreserveStructure)?;
        }

        Commands::ExtractFlat {
            dat_file,
            output,
            files,
        } => {
            let archive = DatArchive::open(&dat_file)?;
            let output_dir = output.unwrap_or_else(|| PathBuf::from(".")); // default: current directory
            let patterns = utils::expand_response_files_for_archive(&files)?;
            archive.extract(&output_dir, &patterns, ExtractionMode::Flat)?;
        }

        Commands::Add {
            dat_file,
            files,
            compression,
            dat1,
            target_dir,
        } => {
            // Track if the user explicitly set compression (for the DAT1 warning below)
            let compression_explicitly_set = compression.is_some();
            let compression = compression.unwrap_or(1); // default: level 1
            let compression_level = CompressionLevel::new(compression)?;

            // Expand @response files and glob patterns
            let file_strings: Vec<String> = files
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect();
            let expanded = utils::expand_response_files_with_stripping(&file_strings)?;

            // Count files upfront - fails immediately if any path doesn't exist
            let mut total_files_to_add = 0;
            for file_path in &expanded {
                let collected_files = utils::collect_files(file_path)?;
                total_files_to_add += collected_files.len();
            }

            if total_files_to_add == 0 {
                bail!("No files to add to archive");
            }

            let mut archive = if dat_file.exists() {
                // Open existing archive - format is fixed, can't change it
                let archive = DatArchive::open(&dat_file)?;
                if dat1 && !archive.is_dat1() {
                    bail!("Error: {} is a DAT2 archive, but --dat1 flag was specified. Cannot change archive format.", dat_file.display());
                }
                archive
            } else if dat1 {
                DatArchive::new_dat1() // Fallout 1 format
            } else {
                DatArchive::new_dat2() // Fallout 2 format (default)
            };

            if archive.is_dat1() && compression_explicitly_set && compression > 0 {
                eprintln!("Warning: DAT1 format does not support compression, files will be stored uncompressed");
            }

            for file_path in expanded {
                archive.add_file(&file_path, compression_level, target_dir.as_deref())?;
            }

            archive.save(&dat_file)?;
        }

        Commands::Delete { dat_file, files } => {
            let mut archive = DatArchive::open(&dat_file)?;
            let patterns = utils::expand_response_files_for_archive(&files)?;

            for pattern in patterns {
                archive.delete_file(&pattern)?;
            }

            archive.save(&dat_file)?;
        }
    }

    Ok(())
}
