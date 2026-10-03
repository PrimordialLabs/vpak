output "url" {
  description = "Public HTTPS URL of the service"
  value       = google_cloud_run_v2_service.hello.uri
}

output "artifact_registry" {
  value = "${var.region}-docker.pkg.dev/${var.project}/${google_artifact_registry_repository.hello.repository_id}"
}
