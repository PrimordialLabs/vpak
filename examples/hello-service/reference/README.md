# hello-service (reference realization)

Go HTTP service, distroless non-root container, GitHub Actions CI that builds
on every push and deploys `main` to GCP Cloud Run through Workload Identity
Federation, and Terraform for the registry, runtime service account and the
Cloud Run service with public invoker access.

Local run:

```
go run . &
curl localhost:8080/healthz
curl 'localhost:8080/hello?name=vpak'
```
