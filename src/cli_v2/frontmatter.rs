#![allow(dead_code)]
use super::{Error, Result};

#[derive(Debug, serde::Deserialize, PartialEq, Eq)]
struct TemplateFrontmatter {
    template_scope: TemplateScope,
}

#[derive(Debug, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
enum TemplateScope {
    /// The template will have access to the whole openapi spec, and will evaluated once.
    Spec,
    /// The template will have access to a single resource, and will evaluated once per resources.
    Resource,
}

fn parse_frontmatter(template: &str) -> Result<TemplateFrontmatter> {
    let frontmatter = extract_frontmatter_from_comment(template)?;
    Ok(toml::from_str(frontmatter)?)
}

fn extract_frontmatter_from_comment(template: &str) -> Result<&str> {
    let trimmed = template.trim_start();

    if !trimmed.starts_with("{#") {
        return Err(Error::UnableToExtractFrontmatter(
            "Frontmatter must be at start of file",
        ));
    }

    let start_len = if trimmed.starts_with("{#-") { 3 } else { 2 };
    let remaining = &trimmed[start_len..];

    let end_pos = match (remaining.find("-#}"), remaining.find("#}")) {
        (None, None) => Err(Error::UnableToExtractFrontmatter(
            "No frontmatter end found",
        )),
        (Some(end_pos), None) | (None, Some(end_pos)) => Ok(end_pos),
        // if we find both `-#}` and `#}` in the file, make sure to use the fist one. we don't want to use the position of a random comment
        (Some(end_pos_whitespace_controlled), Some(end_pos_not_whitespace_controlled)) => {
            Ok(end_pos_whitespace_controlled.min(end_pos_not_whitespace_controlled))
        }
    }?;

    Ok(&remaining[..end_pos])
}

#[cfg(test)]
mod tests {
    use crate::cli_v2::frontmatter::{
        TemplateFrontmatter, TemplateScope, extract_frontmatter_from_comment, parse_frontmatter,
    };

    #[rstest::rstest]
    #[case::no_whitespace_control("{#", "#}")]
    #[case::start_whitespace_control("{#-", "#}")]
    #[case::end_whitespace_control("{#", "-#}")]
    #[case::full_whitespace_control("{#-", "-#}")]
    fn test_multiline_comment(#[case] comment_start: &str, #[case] comment_end: &str) {
        let expected_frontmatter = r#"hello "" ' ' # { } # = -  world"#;

        let tml = format!(
            r#"
        {comment_start}
        hello "" ' ' # {{ }} # = -  world
        {comment_end}
            "#
        );
        let frontmatter = extract_frontmatter_from_comment(&tml).unwrap();
        assert_eq!(frontmatter.trim(), expected_frontmatter);
    }

    #[rstest::rstest]
    #[case::no_whitespace_control("{#", "#}")]
    #[case::start_whitespace_control("{#-", "#}")]
    #[case::end_whitespace_control("{#", "-#}")]
    #[case::full_whitespace_control("{#-", "-#}")]
    fn test_parse_frontmatter(#[case] comment_start: &str, #[case] comment_end: &str) {
        let expected_frontmatter = TemplateFrontmatter {
            template_scope: TemplateScope::Spec,
        };

        let tml = format!(
            r#"
        {comment_start}
        template_scope = "spec"
        {comment_end}
            "#
        );

        let frontmatter = parse_frontmatter(&tml).unwrap();
        assert_eq!(frontmatter, expected_frontmatter);
    }

    #[rstest::rstest]
    #[case::no_whitespace_control("{#", "#}")]
    #[case::start_whitespace_control("{#-", "#}")]
    #[case::end_whitespace_control("{#", "-#}")]
    #[case::full_whitespace_control("{#-", "-#}")]
    fn test_comments_after_frontmatter_ignored(
        #[case] comment_start: &str,
        #[case] comment_end: &str,
    ) {
        let expected_frontmatter = r#" template_scope = "spec" "#;

        let tml = format!(
            r#"
        {comment_start} template_scope = "spec" {comment_end}

        {{#    #}}
        {{#-   #}}
        {{#   -#}}
        {{#-  -#}}
        "#
        );
        let frontmatter = extract_frontmatter_from_comment(&tml).unwrap();
        assert_eq!(frontmatter, expected_frontmatter);
    }

    #[rstest::rstest]
    #[case::no_whitespace_control("{#", "#}")]
    #[case::start_whitespace_control("{#-", "#}")]
    #[case::end_whitespace_control("{#", "-#}")]
    #[case::full_whitespace_control("{#-", "-#}")]
    fn test_frontmatter_comment_permutations(
        #[case] comment_start: &str,
        #[case] comment_end: &str,
    ) {
        let expected_frontmatter = r#"hello "" ' ' # { } # = -  world"#;

        let tml = format!(r#"{comment_start}hello "" ' ' # {{ }} # = -  world{comment_end}"#);

        let frontmatter = extract_frontmatter_from_comment(&tml).unwrap();
        assert_eq!(frontmatter, expected_frontmatter);
    }
}
