# Latest version of a project from a hash, loader(s), and game version(s)
## Endpoint
`POST /version_file/{hash}/update`

## Path Parameters
### hash
| Type | Description | Example |
| --- | --- | --- |
| `string` | The hash of the file | `d2c7e5b6c8f9a1b2c3d4e5f6a7b8c9d0e1f` |

## Query Parameters
### algorithm
| Type | Description | Example |
| --- | --- | --- |
| `string` | The hashing algorithm used to generate the hash. Allowed values: sha1 sha512 | `sha1` |

Default: sha1

## Request Body
Request body is a JSON object with the following properties:
| Name | Type | Description |
| --- | --- | --- |
| loaders | `array<string>` | The loaders to filter for. |
| game_versions | `array<string>` | The game versions to filter for. |

## Response
### 200 OK
The response is a JSON object the same properties as the version objects returned by the `GET /project/{id|slug}/version` endpoint

### 404 Not Found
No version was found matching the given hash, loaders, and game versions.

### 400 Bad Request
The request body is invalid, such as missing required properties or invalid values.
