/*!
# DAT3 - Fallout Archive Tool

A cross-platform tool for managing Fallout 1 and 2 DAT archive files.
Supports both DAT1 (Fallout 1) and DAT2 (Fallout 2) formats.
*/

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use std::path::{Path, PathBuf};

// Use a faster memory allocator on Linux
#[cfg(target_os = "linux")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod arcanum; // Arcanum (Troika) DAT format implementation
mod common; // Shared utilities and the main DatArchive interface
mod config; // Optional .bgforge.yml defaults
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
        /// Resolve add operands relative to this directory
        #[arg(short = 'C', long = "change-dir", value_name = "DIR")]
        change_dir: Option<PathBuf>,
        /// Files or directories to add
        files: Vec<PathBuf>,
        /// Compression level 0-9
        #[arg(short, long, value_parser = clap::value_parser!(u8).range(0..=9))]
        compression: Option<u8>,
        /// Format for new archives; without it, .bgforge.yml's
        /// dat3.default_format applies, then dat2 (existing archives keep theirs)
        #[arg(long, value_enum)]
        format: Option<ArchiveFormat>,
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

/// Archive format selector for the `a` command
#[derive(Debug, Clone, Copy, PartialEq, ValueEnum)]
enum ArchiveFormat {
    /// Fallout 1 (big-endian, LZSS; created uncompressed)
    Dat1,
    /// Fallout 2 (little-endian, zlib) - the default for new archives
    Dat2,
    /// Arcanum (little-endian, zlib)
    Arcanum,
}

impl ArchiveFormat {
    /// The value as typed on the command line, for error messages
    fn arg_name(self) -> &'static str {
        match self {
            Self::Dat1 => "dat1",
            Self::Dat2 => "dat2",
            Self::Arcanum => "arcanum",
        }
    }
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
            change_dir,
            compression,
            format,
            target_dir,
        } => {
            // Track if the user explicitly set compression (for the DAT1 warning below)
            let compression_explicitly_set = compression.is_some();
            let compression = compression.unwrap_or(1); // default: level 1
            let compression_level = CompressionLevel::new(compression)?;

            let change_dir = match change_dir {
                Some(path) => {
                    let resolved = std::fs::canonicalize(&path).with_context(|| {
                        format!("Failed to resolve -C directory: {}", path.display())
                    })?;
                    if !resolved.is_dir() {
                        bail!("-C must point to a directory: {}", path.display());
                    }
                    Some(resolved)
                }
                None => None,
            };

            // Expand @response files and glob patterns
            let file_strings: Vec<String> = files
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect();
            let expanded =
                utils::expand_response_files_with_stripping(&file_strings, change_dir.as_deref())?;
            let expanded: Vec<PathBuf> = expanded
                .iter()
                .map(|path| utils::resolve_add_input_path(path, change_dir.as_deref()))
                .collect::<Result<_>>()?;

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
                if let Some(requested) = format {
                    let actual = match archive {
                        DatArchive::Dat1(_) => ArchiveFormat::Dat1,
                        DatArchive::Dat2(_) => ArchiveFormat::Dat2,
                        DatArchive::Arcanum(_) => ArchiveFormat::Arcanum,
                    };
                    if requested != actual {
                        bail!("{}: archive format is {}, but --format {} was specified. Cannot change the format of an existing archive.", dat_file.display(), archive.format_name(), requested.arg_name());
                    }
                }
                archive
            } else {
                // Explicit flag wins; then the per-directory config; then dat2
                let format = format
                    .or_else(|| config::default_format(Path::new(".")))
                    .unwrap_or(ArchiveFormat::Dat2);
                match format {
                    ArchiveFormat::Dat1 => DatArchive::new_dat1(),
                    ArchiveFormat::Dat2 => DatArchive::new_dat2(),
                    ArchiveFormat::Arcanum => DatArchive::new_arcanum(),
                }
            };

            if archive.is_dat1() && compression_explicitly_set && compression > 0 {
                eprintln!("Warning: DAT1 format does not support compression, files will be stored uncompressed");
            }

            for file_path in expanded {
                archive.add_file(
                    &file_path,
                    compression_level,
                    target_dir.as_deref(),
                    change_dir.as_deref(),
                )?;
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
