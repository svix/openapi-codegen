// Deprecated: use `ListByMsg` instead, passing the endpoint ID through options
func (messageAttempt *MessageAttempt) ListAttemptsForEndpoint(
	ctx context.Context,
	appId string,
	msgId string,
	endpointId string,
	options *MessageAttemptListOptions,
) (*ListResponseMessageAttemptEndpointOut, error) {
	options.EndpointId = &endpointId
	return messageAttempt.ListByMsg(ctx, appId, msgId, options)
}
