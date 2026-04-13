# Search projects
## Endpoint
`GET /search`

## Query Parameters
### query
| Type | Description | Example |
| --- | --- | --- |
| `string` | The search query. | `gravestones` |

### facets
| Type | Description | Example |
| --- | --- | --- |
| `string` | essential concept for understanding how to filter out results | [["categories:forge"],["versions:1.17.1"],["project_type:mod"],["license:mit"]] |

These are the most commonly used facet types:

- project_type
- categories (loaders are lumped in with categories in search)
- versions
- client_side
- server_side
- open_source

Several others are also available for use, though these should not be used outside very specific use cases.

- title
- author
- follows
- project_id
- license
- downloads
- color
- created_timestamp (uses Unix timestamp)
- modified_timestamp (uses Unix timestamp)
- date_created (uses ISO-8601 timestamp)
- date_modified (uses ISO-8601 timestamp)

In order to then use these facets, you need a value to filter by, as well as an operation to perform on this value. The most common operation is : (same as =), though you can also use !=, >=, >, <=, and <. Join together the type, operation, and value, and you’ve got your string.
```
{type} {operation} {value}
```
Examples:
```
categories = adventure
versions != 1.20.1
downloads <= 100
```
You then join these strings together in arrays to signal AND and OR operators.

OR
All elements in a single array are considered to be joined by OR statements. For example, the search [["versions:1.16.5", "versions:1.17.1"]] translates to Projects that support 1.16.5 OR 1.17.1.

AND
Separate arrays are considered to be joined by AND statements. For example, the search [["versions:1.16.5"], ["project_type:modpack"]] translates to Projects that support 1.16.5 AND are modpacks.

## Response
### 200 OK
Response is a JSON object with the following properties:
| Name | Type | Description |
| --- | --- | --- |
| *hits | `array<object>` | list of results |
| *offset | `integer` | the number of results skipped |
| *limit | `integer` | number of results returned |
| *total_hits | `integer` | total number of results matching the query |

the hits array contains objects with the following properties(some are not listed), the "*" indicates that the property is required:
| Name | Type | Description |
| --- | --- | --- |
| *project_type | `string` | The type of the project. Allowed values: mod modpack resourcepack shader |
| *downloads | `integer` | The number of times the project has been downloaded |
| *project_id | `string` | The ID of the project. encoded as a base62 string |
| *author | `string` | The ID of the team that has ownership of this project |
| *versions | `array<string>` | The versions of the project |
| *follows | `integer` | The number of followers the project has |
| *date_created | `string` | The date the project was published format: ISO-8601 |
| *date_modified | `string` | The date the project was last updated format: ISO-8601 |
| *license | `string` | The license of the project. |
| slug | `string` | The slug of a project, used for vanity URLs. Regex: \^[\w!@$()`.+,"\\-']{3,64}$ |
| title | `string` | The title of the project. |
| description | `string` | The description of the project. |
| categories | `array<string>` | The categories of the project. |
| client_side | `string` | The client side of the project. Allowed values: required optional unsupported unknown |
| server_side | `string` | The server side of the project. Allowed values: required optional unsupported unknown |

### 400 Bad Request
Request is invalid, response is a JSON object with the following properties:
| Name | Type | Description |
| --- | --- | --- |
| error | `string` | name of the error |
| description | `string` | contents of the error |