// hello-service: a small HTTP service used as the vpak example reference.
package main

import (
	"encoding/json"
	"log"
	"net/http"
	"os"
)

// version is set at build time: go build -ldflags "-X main.version=1.2.3".
var version = "dev"

type greeting struct {
	Greeting string `json:"greeting"`
	Service  string `json:"service"`
	Version  string `json:"version"`
}

func main() {
	port := os.Getenv("PORT")
	if port == "" {
		port = "8080"
	}
	mux := http.NewServeMux()
	mux.HandleFunc("/healthz", func(w http.ResponseWriter, r *http.Request) {
		w.WriteHeader(http.StatusOK)
		_, _ = w.Write([]byte("ok"))
	})
	mux.HandleFunc("/hello", func(w http.ResponseWriter, r *http.Request) {
		name := r.URL.Query().Get("name")
		if name == "" {
			name = "world"
		}
		w.Header().Set("Content-Type", "application/json")
		_ = json.NewEncoder(w).Encode(greeting{Greeting: "Hello, " + name + "!", Service: "hello-service", Version: version})
	})
	handler := func(w http.ResponseWriter, r *http.Request) {
		log.Printf("%s %s", r.Method, r.URL.Path)
		mux.ServeHTTP(w, r)
	}
	log.Printf("hello-service %s listening on :%s", version, port)
	log.Fatal(http.ListenAndServe(":"+port, http.HandlerFunc(handler)))
}
