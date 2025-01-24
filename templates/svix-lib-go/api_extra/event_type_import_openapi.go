// Deprecated: Use eventType.ImportOpenapi instead
func (eventType *EventType) ImportOpenApi(
	ctx context.Context,
	eventTypeImportOpenApiIn EventTypeImportOpenApiIn,
) (*EventTypeImportOpenApiOut, error) {
	return eventType.ImportOpenapi(ctx, &eventTypeImportOpenApiIn)
}

// Deprecated: Use eventType.ImportOpenapiWithOptions instead
func (eventType *EventType) ImportOpenApiWithOptions(
	ctx context.Context,
	eventTypeImportOpenApiIn EventTypeImportOpenApiIn,
	options *PostOptions,
) (*EventTypeImportOpenApiOut, error) {
	return eventType.ImportOpenapiWithOptions(ctx, &eventTypeImportOpenApiIn, options)
}
