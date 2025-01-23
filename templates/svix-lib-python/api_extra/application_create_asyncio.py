async def get_or_create(
    self,
    application_in: ApplicationIn,
    options: ApplicationCreateOptions = ApplicationCreateOptions(),
) -> ApplicationOut:
    response = await self._request_asyncio(
        method="post",
        path="/api/v1/app",
        path_params={},
        query_params={"get_if_exists": "true"},
        header_params=options._header_params(),
        json_body=application_in.to_dict(),
    )
    return ApplicationOut.from_dict(response.json())
