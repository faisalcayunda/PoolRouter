FROM docker.io/library/rust:1.99.1-bookworm AS build
RUN cargo build --release

FROM debian:bookworm-slim
# syntax=docker/dockerfile:1.7
COPY --link --from=build /src/app /app
HEALTHCHECK CMD /app healthcheck
USER root
