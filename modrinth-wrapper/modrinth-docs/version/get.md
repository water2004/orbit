# Get a version given a version number or ID
## Endpoint
`GET /project/{id|slug}/version/{id|number}`

## Path Parameters
### id|slug
| Type | Description | Example |
| --- | --- | --- |
| `string` | The ID or slug of the project | `gravestones` |

### id|number
| Type | Description | Example |
| --- | --- | --- |
| `string` | The ID or version number of the version | `1.0.0` |

## Response
### 200 OK
Expected response to a valid request. Response is a JSON object the same properties as the version objects returned by the `GET /project/{id|slug}/version` endpoint.