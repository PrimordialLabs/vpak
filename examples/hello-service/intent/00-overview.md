# hello-service

This vpak builds a small, stateless HTTP service that greets callers. It
exists to prove a deployment path end to end: build a container, run it
somewhere that gives it a public HTTPS URL, and keep it healthy.

What matters:

- Anyone with the URL can call it. There is no authentication.
- It must answer a health check so the platform can restart it when it hangs.
- It must be rebuilt and redeployed from source by CI on every push to the
  main branch. No hand deploys.
- It must run as a single container image. The image is the unit of release.

What does not matter:

- Which cloud or runtime hosts the container. The reference uses GCP Cloud
  Run, but ECS, Kubernetes, Fly, or a Docker host all satisfy the intent.
- Which CI system builds it, as long as it is triggered by source changes.
