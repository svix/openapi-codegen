use camino::Utf8PathBuf;
use clap::{Parser, Subcommand};
use serde::Serialize;

pub mod api;
pub mod codesamples;
mod docker_postprocessing;
pub mod generator;
pub mod postprocessing;
pub mod template;

#[derive(Parser)]
pub struct CliArgs {
    /// Which operations to include.
    #[arg(global = true, long, value_enum, default_value_t = IncludeMode::OnlyPublic)]
    pub include_mode: IncludeMode,

    /// Ignore a specified operation id
    #[arg(global = true, short, long = "exclude-op-id")]
    pub excluded_operations: Vec<String>,

    /// Only include specified operations
    ///
    /// This option only works with `--include-mode=only-specified`.
    ///
    /// Use this option, to run the codegen with a limited set of operations.
    /// Op webhook models will be excluded from the generation
    #[arg(global = true, long = "include-op-id")]
    pub specified_operations: Vec<String>,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Clone, Subcommand)]
pub enum Command {
    /// Generate code from an OpenAPI spec.
    Generate {
        /// Path to a template file to use (`.jinja` extension can be omitted).
        #[arg(short, long)]
        template: Utf8PathBuf,

        /// Path to the input file(s).
        #[arg(short, long)]
        input_file: Vec<String>,

        /// Path to the output directory.
        #[arg(short, long)]
        output_dir: Option<Utf8PathBuf>,

        /// Disable automatic postprocessing of the output (formatting and automatic style fixes).
        #[arg(long)]
        no_postprocess: bool,
    },
    /// Generate api.ron and types.ron files, for debugging.
    Debug {
        /// Path to the input file(s).
        #[arg(short, long)]
        input_file: Vec<String>,
    },
}

#[derive(Copy, Clone, clap::ValueEnum)]
#[clap(rename_all = "kebab-case")]
pub enum IncludeMode {
    /// Only public options
    OnlyPublic,
    /// Both public operations and operations marked with `x-hidden`
    PublicAndHidden,
    /// Only operations marked with `x-hidden`
    OnlyHidden,
    /// Only operations that were specified in `--include-op-id`
    OnlySpecified,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum CodegenLanguage {
    Python,
    Rust,
    Go,
    Kotlin,
    CSharp,
    Java,
    TypeScript,
    Ruby,
    Php,
    Shell,
    Unknown,
}

impl CodegenLanguage {
    pub fn ext(self) -> &'static str {
        match self {
            CodegenLanguage::Python => "py",
            CodegenLanguage::Rust => "rs",
            CodegenLanguage::Go => "go",
            CodegenLanguage::Kotlin => "kt",
            CodegenLanguage::CSharp => "cs",
            CodegenLanguage::Java => "java",
            CodegenLanguage::TypeScript => "ts",
            CodegenLanguage::Ruby => "rb",
            CodegenLanguage::Php => "php",
            CodegenLanguage::Shell => "sh",
            CodegenLanguage::Unknown => "txt",
        }
    }
}
