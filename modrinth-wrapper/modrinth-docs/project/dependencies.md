# Get all of a project's dependencies
## Endpoint
`GET /project/{id|slug}/dependencies`

## Path Parameters
### id|slug
| Type | Description | Example |
| --- | --- | --- |
| `string` | The ID or slug of the project | `gravestones` |

## Response
### 200 OK
Response is a JSON array of objects, each containing the following properties:
| Name | Type | Description |
| --- | --- | --- |
| projects | `array<object>` | An array of project objects that are dependencies of the given project |
| versions | `array<object>` | An array of version objects that are dependencies of the given project |

The project objects in the response have the same properties as the project objects returned by the `GET /project/{id|slug}` endpoint, and the version objects list have the same properties as the version objects returned by the `GET /project/{id|slug}/version` endpoint.