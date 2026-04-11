# Overview

## Endpoints
Production server 
https://api.modrinth.com/v2
Staging server 
https://staging-api.modrinth.com/v2

## Identifiers
The majority of items you can interact with in the API have a unique eight-digit base62 ID. Projects, versions, users, threads, teams, and reports all use this same way of identifying themselves. Version files use the sha1 or sha512 file hashes as identifiers.

Each project and user has a friendlier way of identifying them; slugs and usernames, respectively. While unique IDs are constant, slugs and usernames can change at any moment. If you want to store something in the long term, it is recommended to use the unique ID.

## Ratelimits
The API has a ratelimit defined per IP. Limits and remaining amounts are given in the response headers.

- X-Ratelimit-Limit: the maximum number of requests that can be made in a minute
- X-Ratelimit-Remaining: the number of requests remaining in the current ratelimit window
- X-Ratelimit-Reset: the time in seconds until the ratelimit window resets

Ratelimits are the same no matter whether you use a token or not. The ratelimit is currently 300 requests per minute. If you have a use case requiring a higher limit, please contact us.

## User Agents
To access the Modrinth API, you must use provide a uniquely-identifying User-Agent header. Providing a user agent that only identifies your HTTP client library (such as “okhttp/4.9.3”) increases the likelihood that we will block your traffic. It is recommended, but not required, to include contact information in your user agent. This allows us to contact you if we would like a change in your application’s behavior without having to block your traffic.

- Bad: User-Agent: okhttp/4.9.3
- Good: User-Agent: project_name
- Better: User-Agent: github_username/project_name/1.56.0
- Best: User-Agent: github_username/project_name/1.56.0 (launcher.com) or User-Agent: github_username/project_name/1.56.0 (contact@launcher.com)