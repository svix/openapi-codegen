//! A WIP new cli, to the openapi-codegen
pub(crate) mod frontmatter;
pub(crate) mod value_vec;

#[derive(Debug, thiserror::Error)]
pub(super) enum Error {
    #[error("failed to parse toml frontmatter {0}")]
    TomlParseError(#[from] toml::de::Error),

    #[error("Failed to extract frontmatter from template: {0}")]
    UnableToExtractFrontmatter(&'static str),
}

pub(super) type Result<T> = std::result::Result<T, Error>;
