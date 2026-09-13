/*!
# DAT3 - Fallout Archive Tool

A cross-platform tool for managing Fallout and Troika DAT archive files.
*/

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use std::path::{Path, PathBuf};

// Use a faster memory allocator on Linux
#[cfg(target_os = "linux")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

mod config; // Optional .bgforge.yml defaults

use dat3_core::common::{
    self, CaseMode, CompressionLevel, ExtractionMode, ListFormat, MissingFiles, Selection, utils,
};
use dat3_core::{ArchiveFormat, DatArchive};

/// Command-line interface definition.
/// The `clap` crate uses these derive macros to automatically parse arguments.
#[derive(Parser)]
#[command(name = "dat3")]
#[command(author = "DAT Tool Rewrite")]
#[command(about = "Fallout and Troika .dat management CLI")]
#[command(version)]
struct Cli {
    /// Match, list, extract and add entry names exactly as stored. Without it,
    /// names match regardless of case and are listed, extracted and added in lowercase.
    #[arg(long, global = true)]
    case_sensitive: bool,

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
        /// Print the listing as JSON instead of aligned columns
        #[arg(long)]
        json: bool,
        /// Warn about requested files that are not in the archive instead of failing
        #[arg(long)]
        ignore_missing: bool,
    },

    /// Extract files preserving directory structure
    #[command(name = "x")]
    Extract {
        dat_file: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
        files: Vec<String>,
        /// Warn about requested files that are not in the archive instead of failing
        #[arg(long)]
        ignore_missing: bool,
    },

    /// Extract files flat (no subdirectories)
    #[command(name = "e")]
    ExtractFlat {
        dat_file: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
        files: Vec<String>,
        /// Warn about requested files that are not in the archive instead of failing
        #[arg(long)]
        ignore_missing: bool,
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

/// Map the `--ignore-missing` flag to the policy the archive operations take.
fn missing_files_policy(ignore_missing: bool) -> MissingFiles {
    if ignore_missing {
        MissingFiles::Warn
    } else {
        MissingFiles::Fail
    }
}

/// Open an archive, warning when case-insensitive mode meets stored names that
/// differ only in case: those keep their stored case rather than merging.
fn open_archive(path: &Path, case: CaseMode) -> Result<DatArchive> {
    let archive = DatArchive::open(path)?;
    if case == CaseMode::Insensitive {
        let names = archive.entry_names();
        common::report_case_only_duplicates(names.iter().map(String::as_str));
    }
    Ok(archive)
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let case = if cli.case_sensitive {
        CaseMode::Sensitive
    } else {
        CaseMode::Insensitive
    };

    match cli.command {
        Commands::List {
            dat_file,
            files,
            json,
            ignore_missing,
        } => {
            let archive = open_archive(&dat_file, case)?;
            let patterns = utils::expand_response_files_for_archive(&files)?;
            let format = if json {
                ListFormat::Json
            } else {
                ListFormat::Text
            };
            let selection = Selection {
                patterns: &patterns,
                on_missing: missing_files_policy(ignore_missing),
                case,
            };
            archive.list(&selection, format)?;
        }

        Commands::Extract {
            dat_file,
            output,
            files,
            ignore_missing,
        } => {
            let archive = open_archive(&dat_file, case)?;
            let output_dir = output.unwrap_or_else(|| PathBuf::from(".")); // default: current directory
            let patterns = utils::expand_response_files_for_archive(&files)?;
            let selection = Selection {
                patterns: &patterns,
                on_missing: missing_files_policy(ignore_missing),
                case,
            };
            archive.extract(&output_dir, ExtractionMode::PreserveStructure, &selection)?;
        }

        Commands::ExtractFlat {
            dat_file,
            output,
            files,
            ignore_missing,
        } => {
            let archive = open_archive(&dat_file, case)?;
            let output_dir = output.unwrap_or_else(|| PathBuf::from(".")); // default: current directory
            let patterns = utils::expand_response_files_for_archive(&files)?;
            let selection = Selection {
                patterns: &patterns,
                on_missing: missing_files_policy(ignore_missing),
                case,
            };
            archive.extract(&output_dir, ExtractionMode::Flat, &selection)?;
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
            let mut skipped_symlinks = Vec::new();
            for file_path in &expanded {
                total_files_to_add += utils::count_files(file_path, &mut skipped_symlinks)?;
            }

            if total_files_to_add == 0 {
                // The add pass that would report them never runs, so say it here
                utils::report_skipped_symlinks(&skipped_symlinks);
                bail!("No files to add to archive");
            }

            let mut archive = if dat_file.exists() {
                // Open existing archive - format is fixed, can't change it
                let archive = open_archive(&dat_file, case)?;
                if let Some(requested) = format {
                    let actual = archive.format();
                    if requested != actual {
                        bail!(
                            "{}: archive format is {}, but --format {} was specified. Cannot change the format of an existing archive.",
                            dat_file.display(),
                            actual.display_name(),
                            requested.arg_name()
                        );
                    }
                }
                archive
            } else {
                // Explicit flag wins; then the per-directory config; then dat2
                let format = format
                    .or_else(|| config::default_format(Path::new(".")))
                    .unwrap_or(ArchiveFormat::Dat2);
                DatArchive::new(format)
            };

            if archive.format() == ArchiveFormat::Dat1
                && compression_explicitly_set
                && compression > 0
            {
                eprintln!(
                    "Warning: DAT1 format does not support compression, files will be stored uncompressed"
                );
            }

            for file_path in expanded {
                archive.add_file(
                    &file_path,
                    compression_level,
                    target_dir.as_deref(),
                    change_dir.as_deref(),
                    case,
                )?;
            }

            archive.save(&dat_file)?;
        }

        Commands::Delete { dat_file, files } => {
            let mut archive = open_archive(&dat_file, case)?;
            let patterns = utils::expand_response_files_for_archive(&files)?;
            archive.delete(&patterns, case)?;
            archive.save(&dat_file)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod cli_args {
    use clap::Parser;

    #[test]
    fn rejects_out_of_range_compression_at_parse_time() {
        let result = crate::Cli::try_parse_from(["dat3", "a", "test.dat", "-c", "10", "file"]);
        assert!(
            result.is_err(),
            "compression level 10 should be rejected during argument parsing"
        );
    }

    #[test]
    fn accepts_maximum_compression_level() {
        let result = crate::Cli::try_parse_from(["dat3", "a", "test.dat", "-c", "9", "file"]);
        assert!(result.is_ok());
    }

    #[test]
    fn accepts_each_archive_format() {
        for format in ["dat1", "dat2", "arcanum", "toee"] {
            let result =
                crate::Cli::try_parse_from(["dat3", "a", "test.dat", "--format", format, "file"]);
            assert!(result.is_ok(), "--format {format} should parse");
        }
    }

    #[test]
    fn rejects_unknown_format_and_removed_format_flags() {
        for args in [
            ["dat3", "a", "test.dat", "--format", "zip", "file"].as_slice(),
            ["dat3", "a", "test.dat", "--dat1", "file"].as_slice(),
            ["dat3", "a", "test.dat", "--arcanum", "file"].as_slice(),
        ] {
            assert!(
                crate::Cli::try_parse_from(args.iter().copied()).is_err(),
                "{args:?} should be rejected"
            );
        }
    }
}
