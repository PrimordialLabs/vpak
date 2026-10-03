# Packer constraints

- HTTPS with a platform-managed certificate; no self-signed certificates.
- The container runs as a non-root user.
- Deploys are triggered by CI from the main branch.
- The image must be built from the Dockerfile in the reference, or an equivalent that keeps the non-root user and the PORT contract.
