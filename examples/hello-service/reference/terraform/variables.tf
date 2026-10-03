variable "project" {
  type        = string
  description = "GCP project id"
}

variable "region" {
  type        = string
  description = "Cloud Run region"
  default     = "us-central1"
}

variable "image" {
  type        = string
  description = "Fully qualified container image to deploy"
}
