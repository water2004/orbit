# Latest versions of multiple project from hashes, loader(s), and game version(s)
## Endpoint
`POST /version_files/update`

## Request Body
### hashes
| Type | Description | Example |
| --- | --- | --- |
| `array<string>` | The hashes of the files | `["d2c7e5b6c8f9a1b2c3d4e5f6a7b8c9d0e1f", "e1f2d3c4b5a6978876543210fedcba987654"]` |

### algorithm
| Type | Description | Example |
| --- | --- | --- |
| `string` | The hashing algorithm used to generate the hashes. Allowed values: sha1 sha512 | `sha1` |

Default: sha1

### loaders
| Type | Description | Example |
| --- | --- | --- |
| `array<string>` | The loaders to filter for. | `["fabric", "forge"]` |

### game_versions
| Type | Description | Example |
| --- | --- | --- |
| `array<string>` | The game versions to filter for. | `["1.18.1", "1.18.2"]` |


## Response
### 200 OK
A map from hashes to versions, the same as the version objects returned by the `GET /project/{id|slug}/version` endpoint