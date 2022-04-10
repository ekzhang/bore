FROM rust:1.60.0-buster as builder
WORKDIR /use/src/app
COPY . .
RUN cargo install --path .

FROM debian:buster-slim
COPY --from=builder /usr/local/cargo/bin/bore /usr/local/bin/bore
ENTRYPOINT ["bore"]
CMD ["--help"]

