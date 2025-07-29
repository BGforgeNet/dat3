/*!
# DAT3 - Fallout Archive Tool

A cross-platform tool for managing Fallout 1 and 2 DAT archive files.
Supports both DAT1 (Fallout 1) and DAT2 (Fallout 2) formats.
*/

// Import the libraries we need
use anyhow::Result; // For easy error handling
use clap::{Parser, Subcommand}; // For command-line argument parsing
use std::path::PathBuf; // For cross-platform file paths

// Use a faster memory allocator on Linux (optional optimization)
#[cfg(target_os = "linux")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

// Our own modules that implement the DAT format handling
mod common; // Shared utilities and the main DatArchive interface
mod dat1; // Fallout 1 DAT format implementation
mod dat2; // Fallout 2 DAT format implementation
mod lzss; // LZSS compression for DAT1 files

// Import what we need from our common module
use common::{utils, CompressionLevel, DatArchive};

/// This is the main structure that defines our command-line interface
/// The clap library uses this to automatically parse command-line arguments
#[derive(Parser)]
#[command(name = "dat3")]
#[command(author = "DAT Tool Rewrite")]
#[command(version = "0.1.0")]
#[command(about = "Fallout .dat management cli")]
struct Cli {
    #[command(subcommand)]
    command: Commands, // Which specific command the user wants to run
}

/// All the commands our tool supports
/// Each command corresponds to a different operation on DAT files
#[derive(Subcommand)]
enum Commands {
    /// List files in a DAT archive (command: l)
    #[command(name = "l")]
    List {
        /// The DAT file to examine
        dat_file: PathBuf,
        /// Specific files to list (if empty, lists all files)
        files: Vec<String>,
    },

    /// Extract files from a DAT archive with directory structure (command: x)
    #[command(name = "x")]
    Extract {
        /// The DAT file to extract from
        dat_file: PathBuf,
        /// Where to put the extracted files (-o flag)
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Which files to extract (if empty, extracts all)
        files: Vec<String>,
    },

    /// Extract files without creating directories - all files go to one folder (command: e)
    #[command(name = "e")]
    ExtractFlat {
        /// The DAT file to extract from
        dat_file: PathBuf,
        /// Where to put all the extracted files (-o flag)
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Which files to extract (if empty, extracts all)
        files: Vec<String>,
    },

    /// Add files to a DAT archive (command: a)
    #[command(name = "a")]
    Add {
        /// The DAT file to add to (will be created if it doesn't exist)
        dat_file: PathBuf,
        /// Files or directories to add to the archive
        files: Vec<PathBuf>,
        /// Add directories and all their contents (-r flag)
        #[arg(short, long)]
        recursive: bool,
        /// How much to compress files, 0=none to 9=maximum (-c flag)
        #[arg(short, long, default_value = "6")]
        compression: u8,
        /// Force creating a DAT1 format archive (--dat1 flag)
        #[arg(long)]
        dat1: bool,
        /// Put files in this directory inside the archive (-t flag)
        #[arg(short, long)]
        target_dir: Option<String>,
    },

    /// Delete files from a DAT archive (command: d)
    #[command(name = "d")]
    Delete {
        /// The DAT file to modify
        dat_file: PathBuf,
        /// Files to remove from the archive
        files: Vec<String>,
    },
}

/// The main function - this is where the program starts
fn main() -> Result<()> {
    // Parse what the user typed on the command line
    let cli = Cli::parse();

    // Figure out which command they want to run and do it
    match cli.command {
        Commands::List { dat_file, files } => {
            // LIST COMMAND: Show what files are in the archive
            let archive = DatArchive::open(&dat_file)?;
            let expanded_files = utils::expand_response_files(&files)?;
            archive.list(&expanded_files)?;
        }

        Commands::Extract {
            dat_file,
            output,
            files,
        } => {
            // EXTRACT COMMAND: Get files out of the archive, keeping folder structure
            let archive = DatArchive::open(&dat_file)?;
            let output_dir = output.unwrap_or_else(|| PathBuf::from(".")); // Use current directory if not specified
            let expanded_files = utils::expand_response_files(&files)?;
            archive.extract(&output_dir, &expanded_files, false)?; // false = keep directories
        }

        Commands::ExtractFlat {
            dat_file,
            output,
            files,
        } => {
            // EXTRACT FLAT COMMAND: Get files out but put them all in one folder
            let archive = DatArchive::open(&dat_file)?;
            let output_dir = output.unwrap_or_else(|| PathBuf::from(".")); // Use current directory if not specified
            let expanded_files = utils::expand_response_files(&files)?;
            archive.extract(&output_dir, &expanded_files, true)?; // true = ignore directories
        }
        Commands::Add {
            dat_file,
            files,
            recursive,
            compression,
            dat1,
            target_dir,
        } => {
            // ADD COMMAND: Put new files into the archive
            let mut archive = if dat_file.exists() {
                // Open existing archive
                DatArchive::open(&dat_file)?
            } else {
                // Create new archive - choose format
                if dat1 {
                    DatArchive::new_dat1() // Fallout 1 format
                } else {
                    DatArchive::new_dat2() // Fallout 2 format (default)
                }
            };

            // Validate compression level
            let compression_level = CompressionLevel::new(compression)?;

            // Handle response files (files starting with @)
            let file_strings: Vec<String> = files
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect();
            let expanded_files = utils::expand_response_files(&file_strings)?;

            // Add each file or directory to the archive
            for file_path_str in expanded_files {
                let file_path = PathBuf::from(file_path_str);
                archive.add_file(
                    &file_path,
                    recursive,
                    compression_level,
                    target_dir.as_deref(),
                )?;
            }

            // Save the changes back to the file
            archive.save(&dat_file)?;
        }

        Commands::Delete { dat_file, files } => {
            // DELETE COMMAND: Remove files from the archive
            let mut archive = DatArchive::open(&dat_file)?;
            let expanded_files = utils::expand_response_files(&files)?;

            for file in expanded_files {
                archive.delete_file(&file)?;
            }

            // Save the changes back to the file
            archive.save(&dat_file)?;
        }
    }

    // If we got here, everything worked fine
    Ok(())
}
