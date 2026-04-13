# Get version from hash
## Endpoint
`GET /version_file/{hash}`

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

### multiple
| Type | Description | Example |
| --- | --- | --- |
| `boolean` | Whether to return multiple files that match the hash. | `true` |

This parameter is optional

## Response
### 200 OK
Response is a JSON array of objects, the same as the version objects returned by the `GET /project/{id|slug}/version` endpoint.