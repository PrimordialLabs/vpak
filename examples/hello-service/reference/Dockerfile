# syntax=docker/dockerfile:1
FROM golang:1.22-alpine AS build
ARG VERSION=dev
WORKDIR /src
COPY go.mod ./
COPY main.go ./
RUN CGO_ENABLED=0 go build -ldflags "-s -w -X main.version=${VERSION}" -o /out/hello-service .

FROM gcr.io/distroless/static-debian12:nonroot
COPY --from=build /out/hello-service /hello-service
USER nonroot:nonroot
ENV PORT=8080
EXPOSE 8080
ENTRYPOINT ["/hello-service"]
