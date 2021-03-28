FROM ekidd/rust-musl-builder:nightly-2021-02-13 AS builder

RUN cargo init --bin --name elayday --vcs none

COPY --chown=rust:rust ./Cargo.* ./
RUN cargo build --release
RUN rm src/*.rs

ADD --chown=rust:rust . ./
RUN cargo build --release

FROM alpine:latest
RUN apk --no-cache add ca-certificates
COPY --from=builder \
    /home/rust/src/target/x86_64-unknown-linux-musl/release/elayday \
    /usr/local/bin/
ENTRYPOINT ["/usr/local/bin/elayday"]