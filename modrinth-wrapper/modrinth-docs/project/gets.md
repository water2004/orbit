# Get multiple projects
## Endpoint
`GET /projects`

## Query Parameters
### ids
| Type | Description | Example |
| --- | --- | --- |
| `string` | The IDs and/or slugs of the projects. Passed as a JSON-encoded array. | `["AABBCCDD", "EEFFGGHH"]` |

## Response
### 200 OK
Response is a JSON array of objects, the same as the project objects returned by the `GET /project/{id|slug}` endpoint.

### 400 Bad Request
Request is invalid.

### 404 Not Found
The requested projects were not found.
