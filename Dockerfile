FROM ekidd/rust-musl-builder:nightly-2021-02-13 AS builder

# Bundle grpc_health_probe in the final Docker image
RUN GRPC_HEALTH_PROBE_VERSION=v0.3.6 && \
    curl -Lso/home/rust/grpc_health_probe https://github.com/grpc-ecosystem/grpc-health-probe/releases/download/${GRPC_HEALTH_PROBE_VERSION}/grpc_health_probe-linux-amd64 && \
    chmod +x /home/rust/grpc_health_probe

RUN cargo init --bin --name elayday --vcs none

COPY --chown=rust:rust ./Cargo.* ./
RUN cargo build --release
RUN rm src/*.rs

ADD --chown=rust:rust . ./
RUN cargo build --release

FROM alpine:latest
RUN apk --no-cache add ca-certificates dumb-init
COPY --from=builder \
    /home/rust/src/target/x86_64-unknown-linux-musl/release/elayday \
    /usr/local/bin/
COPY --from=builder \
    /home/rust/grpc_health_probe \
    /usr/local/bin/
ENTRYPOINT ["/usr/bin/dumb-init", "--", "/usr/local/bin/elayday"]