# Latest versions of multiple project from hashes, loader(s), and game version(s)
## Endpoint
`POST /version_files/update`

## Request Body
Request body is a JSON object with the following properties:
| Name | Type | Description |
| --- | --- | --- |
| *hashes | `array<string>` | The hashes of the files. |
| *algorithm | `string` | The hashing algorithm used to generate the hashes. Allowed values: sha1 sha512. Default: sha1 |
| *loaders | `array<string>` | The loaders to filter for. |
| *game_versions | `array<string>` | The game versions to filter for. |


## Response
### 200 OK
A map from hashes to version objects. The version objects have the following properties:

| Name | Type | Description |
| --- | --- | --- |
| *id | `string` | The ID of the version. encoded as a base62 string |
| *project_id | `string` | The ID of the project this version belongs to |
| *author_id | `string` | The ID of the author who published this version |
| *date_published | `string` | The date the version was published format: ISO-8601 |
| *downloads | `integer` | The number of times the version has been downloaded |
| changelog_url | `string` | A link to the changelog for this version. Always null, only kept for legacy compatibility. |
| *files | `array<object>` | An array of file objects associated with this version |
| *name | `string` | The name of the version. |
| *version_number | `string` | The version number of the version. |
| changelog | `string` | The changelog of the version. |
| dependencies | `array<object>` | An array of dependency objects associated with this version |
| *game_versions | `array<string>` | The game versions the version supports |
| *version_type | `string` | The version type of the version. Allowed values: alpha beta release |
| *loaders | `array<string>` | The loaders the version supports |
| *featured | `boolean` | Whether the version is featured or not. |
| status | `string` | The status of the version. Allowed values: listed archived draft unlisted scheduled unknown |
| requested_status | `string` | The requested status of the version. Allowed values: listed archived draft unlisted |

The file objects in the response have the following properties:
| Name | Type | Description |
| --- | --- | --- |
| *id | `string` | The ID of the file. encoded as a base62 string |
| *hashes | `object` | An object containing the hashes of the file. |
| *url | `string` | The URL to download the file. |
| *filename | `string` | The name of the file. |
| *primary | `boolean` | Whether the file is the primary file of the version. |
| *size | `integer` | The size of the file in bytes. |
| file_type | `string` | The type of the additional file, used mainly for adding resource packs to datapacks |

The dependency objects in the response have the following properties:
| Name | Type | Description |
| --- | --- | --- |
| version_id | `string` | The ID of the version that this version depends on |
| project_id | `string` | The ID of the project that this version depends on |
| file_name | `string` | The file name of the dependency. |
| *dependency_type | `string` | The type of the dependency. Allowed values: required optional incompatible embedded |

The hashes object in the file objects has the following properties:
| Name | Type | Description |
| --- | --- | --- |
| sha512 | `string` | The SHA-512 hash of the file. |
| sha1 | `string` | The SHA-1 hash of the file. |