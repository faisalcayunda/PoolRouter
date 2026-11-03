# expect: from-tanpa-digest
FROM --platform=linux/amd64 docker.io/library/debian:bookworm
COPY app /app
USER 10001:10001
ENTRYPOINT ["/app"]
