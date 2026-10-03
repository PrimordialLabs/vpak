# API

Two endpoints, both over HTTP, both unauthenticated.

`GET /healthz` returns `200 OK` with body `ok` when the process can serve
requests. It must not touch any dependency.

`GET /hello?name=<name>` returns `200 OK` with JSON:

```json
{"greeting": "Hello, <name>!", "service": "hello-service", "version": "<version>"}
```

When `name` is missing it greets `world`. `version` is the build version
baked into the image.

The service listens on the port named by the `PORT` environment variable,
defaulting to 8080. It logs one line per request to stdout.
