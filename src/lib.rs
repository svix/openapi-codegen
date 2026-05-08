pub mod api;
mod cli_v1;
pub(crate) mod cli_v2;
mod codesamples;
mod generator;
mod postprocessing;
mod template;
mod utils;

type JsonValue = serde_json::Value;
type JsonObject = serde_json::Map<String, JsonValue>;

pub use crate::{
    cli_v1::{IncludeMode, run_cli_v1_main},
    codesamples::{CodeSample, CodesampleTemplates, generate_codesamples},
    generator::generate,
    postprocessing::CodegenLanguage,
};
pub use aide;
pub use schemars;
