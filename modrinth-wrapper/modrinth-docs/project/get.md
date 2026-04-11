# Get a project
## Endpoint
`GET /project/{id|slug}`

## Path Parameters
### id|slug
| Type | Description | Example |
| --- | --- | --- |
| `string` | The ID or slug of the project | `gravestones` |

## Response
### 200 OK
Response is a JSON object containing the following properties(some are not listed), the "*" indicates that the property is required:
| Name | Type | Description |
| --- | --- | --- |
| *id | `string` | The ID of the project. encoded as a base62 string |
| *team | `string` | The ID of the team that has ownership of this project |
| *published | `string` | The date the project was published format: ISO-8601 |
| *updated | `string` | The date the project was last updated format: ISO-8601 |
| *followers | `integer` | The number of followers the project has |
| *versions | `array<string>` | The versions of the project |
| *downloads | `integer` | The number of times the project has been downloaded |
| *project_type | `string` | The type of the project. Allowed values:
mod modpack resourcepack shader |
| slug | `string` | The slug of a project, used for vanity URLs. Regex: \^[\w!@$()`.+,"\\-']{3,64}$ |
| title | `string` | The title of the project. |
| description | `string` | The description of the project. |
| game_versions | `array<string>` | The game versions the project supports |
| loaders | `array<string>` | The loaders the project supports |
| categories | `array<string>` | The categories of the project. |
| client_side | `string` | The client side of the project. Allowed values: required optional unsupported unknown |
| server_side | `string` | The server side of the project. Allowed values: required optional unsupported unknown |
| body | `string` | A long form description of the project. |

### 404 Not Found
The project with the given ID or slug does not exist.