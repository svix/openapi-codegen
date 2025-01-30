/** Get or create an application. */
suspend fun getOrCreate(
    applicationIn: ApplicationIn,
    options: PostOptions = PostOptions(),
): ApplicationOut {
    try {
        return api.v1ApplicationCreate(applicationIn, true, options.idempotencyKey)
    } catch (e: Exception) {
        throw ApiException.wrap(e)
    }
}
