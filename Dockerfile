FROM clux/muslrust:stable AS builder

# Bundle grpc_health_probe in the final Docker image
RUN GRPC_HEALTH_PROBE_VERSION=v0.4.22 && \
    curl -Lso /tmp/grpc_health_probe https://github.com/grpc-ecosystem/grpc-health-probe/releases/download/${GRPC_HEALTH_PROBE_VERSION}/grpc_health_probe-linux-amd64 && \
    chmod +x /tmp/grpc_health_probe

RUN rustup component add rustfmt
RUN cargo init --bin --name elayday --vcs none

COPY --chown=rust:rust ./Cargo.* ./
RUN cargo build --release
RUN rm src/*.rs

ADD --chown=rust:rust . ./
RUN cargo build --release

FROM alpine:latest
RUN apk --no-cache add ca-certificates dumb-init
COPY --from=builder \
    /volume/target/x86_64-unknown-linux-musl/release/elayday \
    /usr/local/bin/
COPY --from=builder \
    /tmp/grpc_health_probe \
    /usr/local/bin/
ENTRYPOINT ["/usr/bin/dumb-init", "--", "/usr/local/bin/elayday"]
