/// Create the application with the given ID, or create a new one if it
/// doesn't exist yet.
pub async fn get_or_create(
    &self,
    application_in: ApplicationIn,
    options: Option<PostOptions>,
) -> Result<ApplicationOut> {
    let PostOptions { idempotency_key } = options.unwrap_or_default();

    crate::request::Request::new(http1::Method::POST, "/api/v1/app")
        .with_body_param(application_in)
        .with_query_param("get_if_exists", "true".to_owned())
        .with_optional_header_param("idempotency-key", idempotency_key)
        .execute(self.cfg)
        .await
}
