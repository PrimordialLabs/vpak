terraform {
  required_version = ">= 1.5"
  required_providers {
    google = { source = "hashicorp/google", version = "~> 5.0" }
  }
}

provider "google" {
  project = var.project
  region  = var.region
}

resource "google_artifact_registry_repository" "hello" {
  location      = var.region
  repository_id = "hello"
  format        = "DOCKER"
}

resource "google_service_account" "runtime" {
  account_id   = "hello-service-runtime"
  display_name = "hello-service runtime"
}

resource "google_cloud_run_v2_service" "hello" {
  name     = "hello-service"
  location = var.region
  ingress  = "INGRESS_TRAFFIC_ALL"

  template {
    service_account = google_service_account.runtime.email
    containers {
      image = var.image
      ports { container_port = 8080 }
      env {
        name  = "PORT"
        value = "8080"
      }
      startup_probe {
        http_get { path = "/healthz" }
      }
      liveness_probe {
        http_get { path = "/healthz" }
      }
    }
  }
}

resource "google_cloud_run_v2_service_iam_member" "public" {
  name     = google_cloud_run_v2_service.hello.name
  location = var.region
  role     = "roles/run.invoker"
  member   = "allUsers"
}
