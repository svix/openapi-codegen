/**
 * Creates a [MessageIn] with a pre-serialized payload.
 *
 * The payload is not normalized on the server. Normally, payloads are required to be JSON, and Svix
 * will minify the payload before sending the webhooks (for example, by removing extraneous
 * whitespace or unnecessarily escaped characters in strings). With this function, the payload will
 * be sent "as is", without any minification or other processing.
 *
 * @param payload Serialized message payload
 * @param contentType The value to use for the Content-Type header of the webhook sent by Svix,
 *   overwriting the default of `application/json` if specified
 *
 * See the class documentation for details about the other parameters.
 */
fun messageInRaw(
    eventType: String,
    payload: String,
    contentType: String? = null,
    application: ApplicationIn? = null,
    channels: Set<String>? = null,
    eventId: String? = null,
    payloadRetentionHours: Long? = null,
    payloadRetentionPeriod: Long? = 90L,
    tags: Set<String>? = null,
    transformationsParams: Map<String, Any> = mapOf(),
): MessageIn {
    val transformationsParams = transformationsParams.toMutableMap()
    transformationsParams.put("rawPayload", payload)
    if (contentType != null) {
        val headers = mapOf("content-type" to contentType)
        transformationsParams.put("headers", headers)
    }

    return MessageIn(
        eventType = eventType,
        payload = mapOf<String, String>(),
        application = application,
        channels = channels,
        eventId = eventId,
        payloadRetentionHours = payloadRetentionHours,
        payloadRetentionPeriod = payloadRetentionPeriod,
        tags = tags,
        transformationsParams = transformationsParams,
    )
}
