Install this to my AWS account in us-east-1. Use the same credentials I use
for my other AWS projects (the default profile). Run the container on ECS
Fargate behind an Application Load Balancer with an ACM certificate. Push
images to ECR. Keep GitHub Actions as CI, authenticating to AWS with OIDC, no
long-lived keys. Any secrets are to be requested through AWS Secrets Manager
with access granted to the task role only. Prefer Terraform for all
infrastructure.
