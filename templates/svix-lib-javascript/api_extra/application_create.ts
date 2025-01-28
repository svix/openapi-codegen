/** Get the application with the UID from `applicationIn`, or create it if it doesn't exist yet. */
public async getOrCreate(
    applicationIn: ApplicationIn,
    options?: PostOptions
): Promise<ApplicationOut> {
    const request = new SvixRequest(HttpMethod.POST, "/api/v1/app");

    request.body = applicationIn;
    request.setQueryParam("get_if_exists", true);
    request.setHeaderParam("idempotency-key", options?.idempotencyKey);

    const responseBody: any = await request.send(this.requestCtx);
    return responseBody as ApplicationOut;
}
