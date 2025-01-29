/** Get the application with the UID from `applicationIn`, or create it if it doesn't exist yet. */
public getOrCreate(
  applicationIn: ApplicationIn,
  options?: PostOptions
): Promise<ApplicationOut> {
  const request = new SvixRequest(HttpMethod.POST, "/api/v1/app");

  request.setQueryParam("get_if_exists", true);
  request.setHeaderParam("idempotency-key", options?.idempotencyKey);
  request.setBody(applicationIn, "ApplicationIn");

  return request.send(this.requestCtx, "ApplicationOut");
}
