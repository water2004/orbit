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
| *slug | `string` | The slug of a project, used for vanity URLs. Regex: \^[\w!@$()`.+,"\\-']{3,64}$ |
| *project_type | `string` | The type of the project. Allowed values: mod modpack resourcepack shader |
| *team | `string` | The ID of the team that has ownership of this project |
| *title | `string` | The title of the project. |
| *description | `string` | The description of the project. |
| *body | `string` | A long form description of the project. |
| body_url | `string` | The link to the long description of the project. Always null, only kept for legacy compatibility. |
| *published | `string` | The date the project was published format: ISO-8601 |
| *updated | `string` | The date the project was last updated format: ISO-8601 |
| approved | `string` | The date the project's status was set to an approved status format: ISO-8601 |
| queued | `string` | The date the project's status was submitted to moderators for review format: ISO-8601 |
| *status | `string` | The status of the project. Allowed values: approved archived rejected draft unlisted processing withheld scheduled private unknown |
| requested_status | `string` | The requested status when submitting for review or scheduling the project for release. Allowed values: approved archived unlisted private draft |
| moderator_message | `object` | A message that a moderator sent regarding the project |
| license | `object` | The license of the project. |
| *client_side | `string` | The client side of the project. Allowed values: required optional unsupported unknown |
| *server_side | `string` | The server side of the project. Allowed values: required optional unsupported unknown |
| *downloads | `integer` | The number of times the project has been downloaded |
| *followers | `integer` | The number of followers the project has |
| *categories | `array<string>` | The categories of the project. |
| additional_categories | `array<string>` | A list of categories which are searchable but non-primary |
| loaders | `array<string>` | A list of all of the loaders supported by the project |
| versions | `array<string>` | A list of the version IDs of the project |
| game_versions | `array<string>` | A list of all of the game versions supported by the project |
| donation_urls | `array<object>` | A list of donation links for the project |
| gallery | `array<object>` | A list of images that have been uploaded to the project's gallery |
| issues_url | `string` | An optional link to where to submit bugs or issues with the project |
| source_url | `string` | An optional link to the source code of the project |
| wiki_url | `string` | An optional link to the project's wiki page or other relevant information |
| discord_url | `string` | An optional invite link to the project's discord |
| icon_url | `string` | The URL of the project's icon |
| color | `integer` | The RGB color of the project, automatically generated from the project icon |
| thread_id | `string` | The ID of the moderation thread associated with this project |
| monetization_status | `string` | The monetization status of the project. Allowed values: monetized demonetized force-demonetized |

### 404 Not Found
The project with the given ID or slug does not exist.